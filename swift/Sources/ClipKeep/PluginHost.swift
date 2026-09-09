import Foundation

final class PluginProcess {
    enum Event {
        case searchResults(queryId: UInt64, items: [SearchResult])
        case catalogChanged
        case actionFinished(requestId: UInt64, success: Bool, message: String?)
        case error(String)
    }

    let descriptor: PluginDescriptor
    var onEvent: ((Event) -> Void)?

    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private var buffer = Data()

    init(descriptor: PluginDescriptor, executableURL: URL) throws {
        self.descriptor = descriptor
        process.executableURL = executableURL
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.standardError

        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            DispatchQueue.main.async { self?.consume(data) }
        }
        try process.run()
        send(["type": "initialize", "protocol_version": 1])
    }

    deinit {
        output.fileHandleForReading.readabilityHandler = nil
        if process.isRunning {
            send(["type": "shutdown"])
            process.terminate()
        }
    }

    func search(queryId: UInt64, query: String) {
        send([
            "type": "search",
            "query_id": queryId,
            "query": query,
            "limit": 100,
        ])
    }

    func execute(requestId: UInt64, resultId: String, actionId: String) {
        send([
            "type": "execute",
            "request_id": requestId,
            "result_id": resultId,
            "action_id": actionId,
        ])
    }

    private func send(_ object: [String: Any]) {
        guard process.isRunning,
              var data = try? JSONSerialization.data(withJSONObject: object) else { return }
        data.append(0x0A)
        do {
            try input.fileHandleForWriting.write(contentsOf: data)
        } catch {
            onEvent?(.error(error.localizedDescription))
        }
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer[..<newline]
            buffer.removeSubrange(...newline)
            guard !line.isEmpty else { continue }
            decode(Data(line))
        }
    }

    private func decode(_ data: Data) {
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = json["type"] as? String else { return }
        switch type {
        case "search_results":
            guard let queryId = (json["query_id"] as? NSNumber)?.uint64Value,
                  let rawItems = json["items"],
                  let itemData = try? JSONSerialization.data(withJSONObject: rawItems),
                  let items = try? JSONDecoder().decode([SearchResult].self, from: itemData)
            else { return }
            onEvent?(.searchResults(queryId: queryId, items: items))
        case "catalog_changed":
            onEvent?(.catalogChanged)
        case "action_finished":
            let requestId = (json["request_id"] as? NSNumber)?.uint64Value ?? 0
            let success = json["success"] as? Bool ?? false
            onEvent?(.actionFinished(
                requestId: requestId,
                success: success,
                message: json["message"] as? String
            ))
        case "error":
            onEvent?(.error(json["message"] as? String ?? "Plugin error"))
        default:
            break
        }
    }
}

final class SearchCoordinator {
    var onResultsChanged: (([SearchResult]) -> Void)?
    var onStatusChanged: ((String) -> Void)?
    var onPluginStateChanged: (() -> Void)?

    private var processes: [String: PluginProcess] = [:]
    private var resultsByPlugin: [String: [SearchResult]] = [:]
    private var currentQuery = ""
    private var currentPluginId = "io.clipkeep.applications"
    private var queryId: UInt64 = 0
    private var requestId: UInt64 = 0
    private var actionCompletions: [UInt64: (Bool) -> Void] = [:]

    init() {
        for plugin in builtInPlugins where isEnabled(plugin.id) {
            start(plugin)
        }
        search("", pluginId: currentPluginId)
    }

    func isEnabled(_ id: String) -> Bool {
        let key = "plugin.enabled.\(id)"
        return UserDefaults.standard.object(forKey: key) == nil
            ? true
            : UserDefaults.standard.bool(forKey: key)
    }

    func setEnabled(_ enabled: Bool, descriptor: PluginDescriptor) {
        UserDefaults.standard.set(enabled, forKey: "plugin.enabled.\(descriptor.id)")
        if enabled {
            start(descriptor)
        } else {
            processes[descriptor.id] = nil
            resultsByPlugin[descriptor.id] = nil
            publishResults()
        }
        search(currentQuery, pluginId: currentPluginId)
        onPluginStateChanged?()
    }

    func search(_ query: String, pluginId: String) {
        currentQuery = query
        currentPluginId = pluginId
        queryId += 1
        resultsByPlugin.removeAll()
        publishResults()
        processes[pluginId]?.search(queryId: queryId, query: query)
    }

    func execute(_ result: SearchResult, actionId: String, completion: ((Bool) -> Void)? = nil) {
        requestId += 1
        if let completion { actionCompletions[requestId] = completion }
        processes[result.pluginId]?.execute(
            requestId: requestId,
            resultId: result.id,
            actionId: actionId
        )
    }

    private func start(_ descriptor: PluginDescriptor) {
        guard processes[descriptor.id] == nil else { return }
        let executable = Bundle.main.bundleURL
            .appendingPathComponent("Contents/MacOS")
            .appendingPathComponent(descriptor.executable)
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            onStatusChanged?("Missing plugin: \(descriptor.name)")
            return
        }
        do {
            let plugin = try PluginProcess(descriptor: descriptor, executableURL: executable)
            plugin.onEvent = { [weak self] event in self?.handle(event, from: descriptor.id) }
            processes[descriptor.id] = plugin
        } catch {
            onStatusChanged?("Could not start \(descriptor.name): \(error.localizedDescription)")
        }
    }

    private func handle(_ event: PluginProcess.Event, from pluginId: String) {
        switch event {
        case let .searchResults(eventQueryId, items) where eventQueryId == queryId:
            resultsByPlugin[pluginId] = items
            publishResults()
        case .catalogChanged:
            search(currentQuery, pluginId: currentPluginId)
        case let .actionFinished(requestId, success, message):
            onStatusChanged?(message ?? "Done")
            actionCompletions.removeValue(forKey: requestId)?(success)
            search(currentQuery, pluginId: currentPluginId)
        case let .error(message):
            onStatusChanged?(message)
        default:
            break
        }
    }

    private func publishResults() {
        let results = resultsByPlugin.values
            .flatMap { $0 }
            .sorted {
                if $0.score == $1.score { return $0.title < $1.title }
                return $0.score > $1.score
            }
        onResultsChanged?(results)
    }
}
