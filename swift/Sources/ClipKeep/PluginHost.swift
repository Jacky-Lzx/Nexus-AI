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
    private let decodeQueue: DispatchQueue
    private var buffer = Data()

    private struct EventEnvelope: Decodable {
        let type: String
        let queryId: UInt64?
        let items: [SearchResult]?
        let requestId: UInt64?
        let success: Bool?
        let message: String?

        enum CodingKeys: String, CodingKey {
            case type, items, success, message
            case queryId = "query_id"
            case requestId = "request_id"
        }
    }

    init(descriptor: PluginDescriptor, executableURL: URL) throws {
        self.descriptor = descriptor
        decodeQueue = DispatchQueue(label: "io.clipkeep.plugin.decode.\(descriptor.id)")
        process.executableURL = executableURL
        process.standardInput = input
        process.standardOutput = output
        process.standardError = FileHandle.standardError

        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            self?.decodeQueue.async { [weak self] in self?.consume(data) }
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

    func configure(settings: [String: String]) {
        send(["type": "configure", "settings": settings])
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
        guard let envelope = try? JSONDecoder().decode(EventEnvelope.self, from: data) else { return }
        let event: Event?
        switch envelope.type {
        case "search_results":
            guard let queryId = envelope.queryId, let items = envelope.items else { return }
            event = .searchResults(queryId: queryId, items: items)
        case "catalog_changed":
            event = .catalogChanged
        case "action_finished":
            event = .actionFinished(
                requestId: envelope.requestId ?? 0,
                success: envelope.success ?? false,
                message: envelope.message
            )
        case "error":
            event = .error(envelope.message ?? "Plugin error")
        default:
            event = nil
        }
        if let event {
            DispatchQueue.main.async { [weak self] in self?.onEvent?(event) }
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
        ClipboardPreference.registerDefaults()
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
        let pluginChanged = currentPluginId != pluginId
        currentQuery = query
        currentPluginId = pluginId
        queryId += 1
        if pluginChanged {
            resultsByPlugin.removeAll()
            publishResults()
        }
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

    func execute(pluginId: String, actionId: String, completion: ((Bool) -> Void)? = nil) {
        requestId += 1
        if let completion { actionCompletions[requestId] = completion }
        processes[pluginId]?.execute(
            requestId: requestId,
            resultId: "plugin:settings",
            actionId: actionId
        )
    }

    func reloadSettings(for pluginId: String) {
        processes[pluginId]?.configure(settings: settings(for: pluginId))
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
            plugin.configure(settings: settings(for: descriptor.id))
        } catch {
            onStatusChanged?("Could not start \(descriptor.name): \(error.localizedDescription)")
        }
    }

    private func settings(for pluginId: String) -> [String: String] {
        pluginId == "io.clipkeep.clipboard" ? ClipboardPreference.pluginSettings : [:]
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
