import Foundation

final class PluginProcess {
    enum Event {
        case initialized(pluginId: String, protocolVersion: Int)
        case searchResults(queryId: UInt64, items: [SearchResult])
        case catalogChanged
        case actionFinished(requestId: UInt64, success: Bool, message: String?)
        case error(String)
        case terminated
    }

    let descriptor: PluginDescriptor
    var onEvent: ((Event) -> Void)?

    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private let logHandle: FileHandle?
    private let decodeQueue: DispatchQueue
    private var buffer = Data()

    private struct EventEnvelope: Decodable {
        let type: String
        let queryId: UInt64?
        let items: [SearchResult]?
        let requestId: UInt64?
        let success: Bool?
        let message: String?
        let pluginId: String?
        let protocolVersion: Int?

        enum CodingKeys: String, CodingKey {
            case type, items, success, message
            case queryId = "query_id"
            case requestId = "request_id"
            case pluginId = "plugin_id"
            case protocolVersion = "protocol_version"
        }
    }

    init(descriptor: PluginDescriptor, executableURL: URL) throws {
        self.descriptor = descriptor
        logHandle = Self.makeLogHandle(pluginId: descriptor.id)
        decodeQueue = DispatchQueue(label: "io.clipkeep.plugin.decode.\(descriptor.id)")
        process.executableURL = executableURL
        process.standardInput = input
        process.standardOutput = output
        process.standardError = logHandle ?? FileHandle.standardError
        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async { [weak self] in self?.onEvent?(.terminated) }
        }

        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            self?.decodeQueue.async { [weak self] in self?.consume(data) }
        }
        try process.run()
    }

    func initialize() {
        send(["type": "initialize", "protocol_version": 1])
    }

    deinit {
        output.fileHandleForReading.readabilityHandler = nil
        if process.isRunning {
            send(["type": "shutdown"])
            process.terminate()
        }
    }

    private static func makeLogHandle(pluginId: String) -> FileHandle? {
        guard let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
            return nil
        }
        let directory = support.appendingPathComponent("io.clipkeep.ClipKeep/Logs", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(pluginId).log")
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
        guard let handle = try? FileHandle(forWritingTo: url) else { return nil }
        _ = try? handle.seekToEnd()
        return handle
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
        case "initialized":
            guard let pluginId = envelope.pluginId, let protocolVersion = envelope.protocolVersion else { return }
            event = .initialized(pluginId: pluginId, protocolVersion: protocolVersion)
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
    let descriptors: [PluginDescriptor]

    private var processes: [String: PluginProcess] = [:]
    private var resultsByPlugin: [String: [SearchResult]] = [:]
    private var currentQuery = ""
    private var currentPluginId = "io.clipkeep.applications"
    private var queryId: UInt64 = 0
    private var requestId: UInt64 = 0
    private var actionCompletions: [UInt64: (pluginId: String, completion: (Bool) -> Void)] = [:]
    private var restartAttempts: [String: Int] = [:]
    private var pendingSearches: [String: UInt64] = [:]

    init(descriptors: [PluginDescriptor]) {
        self.descriptors = descriptors
        ClipboardPreference.registerDefaults()
        for plugin in descriptors where isEnabled(plugin.id) {
            start(plugin)
        }
        search("", pluginId: currentPluginId)
    }

    func isEnabled(_ id: String) -> Bool {
        let key = "plugin.enabled.\(id)"
        let defaultEnabled = descriptors.first(where: { $0.id == id })?.defaultEnabled ?? false
        return UserDefaults.standard.object(forKey: key) == nil
            ? defaultEnabled
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
        guard let process = processes[pluginId] else { return }
        pendingSearches[pluginId] = queryId
        process.search(queryId: queryId, query: query)
        let sentQueryId = queryId
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self, weak process] in
            guard let self, let process,
                  self.processes[pluginId] === process,
                  self.pendingSearches[pluginId] == sentQueryId else { return }
            let name = self.descriptors.first(where: { $0.id == pluginId })?.name ?? pluginId
            self.onStatusChanged?("\(name) is taking longer than expected…")
        }
    }

    func execute(_ result: SearchResult, actionId: String, completion: ((Bool) -> Void)? = nil) {
        execute(pluginId: result.pluginId, resultId: result.id, actionId: actionId, completion: completion)
    }

    func execute(pluginId: String, actionId: String, completion: ((Bool) -> Void)? = nil) {
        execute(pluginId: pluginId, resultId: "plugin:settings", actionId: actionId, completion: completion)
    }

    private func execute(
        pluginId: String,
        resultId: String,
        actionId: String,
        completion: ((Bool) -> Void)?
    ) {
        guard let process = processes[pluginId] else { completion?(false); return }
        requestId += 1
        let sentRequestId = requestId
        if let completion { actionCompletions[sentRequestId] = (pluginId, completion) }
        process.execute(
            requestId: sentRequestId,
            resultId: resultId,
            actionId: actionId
        )
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self, weak process] in
            guard let self, let process,
                  self.processes[pluginId] === process,
                  let pending = self.actionCompletions.removeValue(forKey: sentRequestId) else { return }
            self.onStatusChanged?("Plugin action timed out")
            pending.completion(false)
        }
    }

    func reloadSettings(for pluginId: String) {
        processes[pluginId]?.configure(settings: settings(for: pluginId))
    }

    private func start(_ descriptor: PluginDescriptor) {
        guard processes[descriptor.id] == nil else { return }
        let executable = descriptor.executableURL
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            onStatusChanged?("Missing plugin: \(descriptor.name)")
            return
        }
        do {
            let plugin = try PluginProcess(descriptor: descriptor, executableURL: executable)
            plugin.onEvent = { [weak self, weak plugin] event in
                guard let plugin else { return }
                self?.handle(event, from: descriptor.id, process: plugin)
            }
            processes[descriptor.id] = plugin
            plugin.initialize()
            plugin.configure(settings: settings(for: descriptor.id))
        } catch {
            onStatusChanged?("Could not start \(descriptor.name): \(error.localizedDescription)")
        }
    }

    private func settings(for pluginId: String) -> [String: String] {
        pluginId == "io.clipkeep.clipboard" ? ClipboardPreference.pluginSettings : [:]
    }

    private func handle(_ event: PluginProcess.Event, from pluginId: String, process: PluginProcess) {
        switch event {
        case let .initialized(actualPluginId, protocolVersion):
            guard actualPluginId == pluginId, protocolVersion == 1 else {
                onStatusChanged?("Rejected incompatible plugin: \(pluginId)")
                processes[pluginId] = nil
                return
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self, weak process] in
                guard let self, let process, self.processes[pluginId] === process else { return }
                self.restartAttempts[pluginId] = 0
            }
        case let .searchResults(eventQueryId, items) where eventQueryId == queryId:
            pendingSearches[pluginId] = nil
            resultsByPlugin[pluginId] = items
            publishResults()
        case .catalogChanged:
            search(currentQuery, pluginId: currentPluginId)
        case let .actionFinished(requestId, success, message):
            onStatusChanged?(message ?? "Done")
            actionCompletions.removeValue(forKey: requestId)?.completion(success)
            search(currentQuery, pluginId: currentPluginId)
        case let .error(message):
            onStatusChanged?(message)
        case .terminated:
            guard processes[pluginId] === process else { return }
            let failedActions = actionCompletions.filter { $0.value.pluginId == pluginId }
            failedActions.keys.forEach { actionCompletions[$0] = nil }
            failedActions.values.forEach { $0.completion(false) }
            processes[pluginId] = nil
            resultsByPlugin[pluginId] = nil
            publishResults()
            scheduleRestart(pluginId: pluginId)
        default:
            break
        }
    }

    private func scheduleRestart(pluginId: String) {
        guard isEnabled(pluginId), let descriptor = descriptors.first(where: { $0.id == pluginId }) else { return }
        let attempt = (restartAttempts[pluginId] ?? 0) + 1
        restartAttempts[pluginId] = attempt
        guard attempt <= 3 else {
            onStatusChanged?("\(descriptor.name) stopped after repeated crashes")
            return
        }
        onStatusChanged?("Restarting \(descriptor.name)…")
        DispatchQueue.main.asyncAfter(deadline: .now() + Double(attempt)) { [weak self] in
            guard let self, self.isEnabled(pluginId), self.processes[pluginId] == nil else { return }
            self.start(descriptor)
            if pluginId == self.currentPluginId {
                self.search(self.currentQuery, pluginId: pluginId)
            }
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
