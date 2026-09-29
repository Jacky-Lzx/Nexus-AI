import Foundation

final class PluginProcess {
    enum Event {
        case initialized(pluginId: String, protocolVersion: Int, features: [String], schema: [PluginConfigurationField], requiredFeatures: [String], requiredServices: [String: Int])
        case searchResults(
            queryId: UInt64,
            items: [SearchResult],
            presentationItems: [PluginPresentationItem]
        )
        case configurationItems(requestId: UInt64, items: [PluginConfigurationItem])
        case notification(PluginNotification)
        case catalogChanged
        case actionFinished(requestId: UInt64, success: Bool, message: String?)
        case error(PluginFailure)
        case terminated
    }

    let descriptor: PluginDescriptor
    let instanceId = UUID().uuidString
    private(set) var features: Set<String> = []
    private(set) var configurationSchema: [PluginConfigurationField] = []
    var onEvent: ((Event) -> Void)?

    private let process = Process()
    private let input = Pipe()
    private let output = Pipe()
    private let logHandle: FileHandle?
    private let decodeQueue: DispatchQueue
    private var framer = PluginLineFramer()
    private lazy var writer = PluginWriteQueue(handle: input.fileHandleForWriting) { [weak self] message in
        self?.transportFailure(message)
    }
    private(set) var isReady = false
    private var stopped = false
    private var waitingRequests: [[String: Any]] = []
    private var waitingBytes = 0
    private let deliveryLock = NSLock()
    private var pendingDeliveries = 0
    private var pendingDeliveryBytes = 0
    private var transportFailed = false

    private struct EventIdentity: Decodable {
        let type: String
        let queryId: UInt64?
        let requestId: UInt64?
        enum CodingKeys: String, CodingKey { case type; case queryId = "query_id"; case requestId = "request_id" }
        var summary: String { "\(type) (query=\(queryId.map(String.init) ?? "none"), request=\(requestId.map(String.init) ?? "none"))" }
    }

    struct EventEnvelope: Decodable {
        let type: String
        let queryId: UInt64?
        let items: [SearchResult]?
        let presentationItems: [PluginPresentationItem]?
        let configurationItems: [PluginConfigurationItem]?
        let notification: PluginNotification?
        let requestId: UInt64?
        let success: Bool?
        let message: String?
        let pluginId: String?
        let protocolVersion: Int?
        let code: String?
        let retryable: Bool?
        let features: [String]?
        let configurationSchema: [PluginConfigurationField]?
        let requiredFeatures: [String]?
        let requiredServices: [String: Int]?

        enum CodingKeys: String, CodingKey {
            case type, items, notification, success, message, code, retryable, features
            case configurationSchema = "configuration_schema"
            case requiredFeatures = "required_features"
            case requiredServices = "required_services"
            case queryId = "query_id"
            case requestId = "request_id"
            case pluginId = "plugin_id"
            case protocolVersion = "protocol_version"
            case configurationItems = "configuration_items"
            case presentationItems = "presentation_items"
        }
    }

    init(descriptor: PluginDescriptor, executableURL: URL) throws {
        self.descriptor = descriptor
        logHandle = Self.makeLogHandle(pluginId: descriptor.id)
        decodeQueue = DispatchQueue(label: "io.clipkeep.plugin.decode.\(descriptor.id)")
        process.executableURL = executableURL
        guard fcntl(input.fileHandleForWriting.fileDescriptor, F_SETNOSIGPIPE, 1) != -1 else {
            throw PluginTransportError(message: "Could not configure plugin pipe: \(errno)")
        }
        process.standardInput = input
        process.standardOutput = output
        process.standardError = logHandle ?? FileHandle.standardError
        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async { [weak self] in self?.onEvent?(.terminated) }
        }

        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard let self else { return }
            self.decodeQueue.sync {
                if data.isEmpty {
                    handle.readabilityHandler = nil
                    do { try self.framer.finish() }
                    catch { self.transportFailure(error.localizedDescription) }
                } else { self.consume(data) }
            }
        }
        try process.run()
    }

    func acceptFeatures(_ offered: [String], schema: [PluginConfigurationField]) {
        features = Set(offered).intersection(PluginContract.features)
        configurationSchema = schema
    }

    func initialize(timeout: TimeInterval = 5) {
        send(PluginContract.initializeRequest)
        DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { [weak self] in
            guard let self, !self.isReady, !self.stopped else { return }
            self.transportFailure("Plugin initialization timed out")
        }
    }

    func markReady(settings: [String: String]) {
        guard !isReady, !stopped else { transportFailure("Duplicate initialization"); return }
        isReady = true
        configure(settings: settings)
        let requests = waitingRequests
        waitingRequests.removeAll()
        waitingBytes = 0
        for request in requests { send(request) }
    }

    func stop() {
        stopped = true
        waitingRequests.removeAll()
        waitingBytes = 0
        writer.close()
        onEvent = nil
        output.fileHandleForReading.readabilityHandler = nil
        if process.isRunning {
            process.terminate()
            let retiringProcess = process
            DispatchQueue.global().asyncAfter(deadline: .now() + 1) {
                if retiringProcess.isRunning { kill(retiringProcess.processIdentifier, SIGKILL) }
            }
        }
    }

    deinit { stop() }

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

    func search(queryId: UInt64, query: String, timeout: TimeInterval) {
        send([
            "type": "search",
            "query_id": queryId,
            "query": query,
            "limit": 100,
            "timeout_ms": max(1, Int(timeout * 1000)),
        ])
    }

    func cancelSearch(queryId: UInt64) {
        waitingRequests.removeAll { ($0["query_id"] as? UInt64) == queryId && ($0["type"] as? String) == "search" }
        waitingBytes = waitingRequests.reduce(0) { $0 + ((try? JSONSerialization.data(withJSONObject: $1).count) ?? 0) }
        guard isReady, features.contains("search.cancel") else { return }
        send(["type": "cancel_search", "query_id": queryId])
    }

    func listConfigurationItems(requestId: UInt64) {
        send([
            "type": "list_configuration_items",
            "request_id": requestId,
        ])
    }

    func configure(settings: [String: String]) {
        var request: [String: Any] = ["type": "configure", "settings": settings]
        if features.contains("configuration.values") {
            var values: [String: Any] = settings
            for field in configurationSchema {
                values[field.id] = (settings[field.id].flatMap(field.parse) ?? field.default).foundationValue
            }
            request["values"] = values
        }
        send(request)
    }

    func execute(requestId: UInt64, resultId: String, actionId: String, input: String? = nil, arguments: [String: Any]? = nil) {
        var request: [String: Any] = [
            "type": "execute",
            "request_id": requestId,
            "result_id": resultId,
            "action_id": actionId,
        ]
        if let input { request["input"] = input }
        if features.contains("action.arguments"), let arguments { request["arguments"] = arguments }
        send(request)
    }

    private func send(_ object: [String: Any]) {
        guard !stopped, process.isRunning else { return }
        do {
            var data = try JSONSerialization.data(withJSONObject: object)
            guard data.count <= PluginLineFramer.maxBytes else {
                throw PluginTransportError(message: "Plugin request exceeds message limit")
            }
            if !isReady, object["type"] as? String != "initialize" {
                // Before readiness retain only the latest adjacent search.
                if object["type"] as? String == "search", waitingRequests.last?["type"] as? String == "search" {
                    let removed = waitingRequests.removeLast()
                    waitingBytes -= (try? JSONSerialization.data(withJSONObject: removed).count) ?? 0
                }
                guard waitingRequests.count < 64, waitingBytes + data.count <= 32 * 1024 * 1024 else {
                    throw PluginTransportError(message: "Plugin initialization queue limit exceeded")
                }
                waitingRequests.append(object)
                waitingBytes += data.count
                return
            }
            data.append(0x0A)
            guard writer.enqueue(data) else { throw PluginTransportError(message: "Plugin write queue limit exceeded") }
        } catch { transportFailure(error.localizedDescription) }
    }

    private func transportFailure(_ message: String) {
        deliveryLock.lock()
        guard !transportFailed else { deliveryLock.unlock(); return }
        transportFailed = true
        deliveryLock.unlock()
        // Never log payloads: clipboard and action inputs may contain private text.
        let diagnostic = "[\(descriptor.id)] \(message)"
        if let data = (diagnostic + "\n").data(using: .utf8) { try? logHandle?.write(contentsOf: data) }
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.stopped else { return }
            let handler = self.onEvent
            self.stop()
            handler?(.error(PluginFailure(message: diagnostic, code: "transport_error")))
            handler?(.terminated)
        }
    }

    private func deliver(_ event: Event, bytes: Int) {
        deliveryLock.lock()
        guard !transportFailed else { deliveryLock.unlock(); return }
        guard pendingDeliveries < 128, pendingDeliveryBytes + bytes <= 32 * 1024 * 1024 else {
            deliveryLock.unlock(); transportFailure("Plugin event queue limit exceeded"); return
        }
        pendingDeliveries += 1
        pendingDeliveryBytes += bytes
        deliveryLock.unlock()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.deliveryLock.lock()
            self.pendingDeliveries -= 1
            self.pendingDeliveryBytes -= bytes
            let failed = self.transportFailed
            self.deliveryLock.unlock()
            guard !self.stopped, !failed else { return }
            if case .initialized = event {
                guard !self.isReady else { self.transportFailure("Duplicate initialization"); return }
            } else if !self.isReady {
                if case .error = event { self.onEvent?(event); return }
                self.transportFailure("Plugin sent an event before initialization")
                return
            }
            self.onEvent?(event)
        }
    }

    private func consume(_ data: Data) {
        do { try framer.consume(data) { try decode($0) } }
        catch { transportFailure(error.localizedDescription) }
    }

    private func decode(_ data: Data) throws {
        let identity = try? JSONDecoder().decode(EventIdentity.self, from: data)
        let envelope: EventEnvelope
        do { envelope = try JSONDecoder().decode(EventEnvelope.self, from: data) }
        catch let DecodingError.dataCorrupted(context) {
            throw PluginTransportError(message: "Malformed plugin JSON at \(context.codingPath.map(\.stringValue).joined(separator: "."))")
        } catch {
            throw PluginTransportError(message: "Invalid plugin event schema: \(identity?.summary ?? "unknown event")")
        }
        func invalid(_ detail: String) -> PluginTransportError {
            PluginTransportError(message: "Invalid \(envelope.type) event (query=\(envelope.queryId.map(String.init) ?? "none"), request=\(envelope.requestId.map(String.init) ?? "none")): \(detail)")
        }
        let event: Event?
        switch envelope.type {
        case "initialized":
            guard let pluginId = envelope.pluginId, let protocolVersion = envelope.protocolVersion else { throw invalid("missing identity or version") }
            event = .initialized(pluginId: pluginId, protocolVersion: protocolVersion, features: envelope.features ?? [], schema: envelope.configurationSchema ?? [], requiredFeatures: envelope.requiredFeatures ?? [], requiredServices: envelope.requiredServices ?? [:])
        case "search_results":
            guard let queryId = envelope.queryId, let items = envelope.items else { throw invalid("missing results") }
            guard items.count <= 100, (envelope.presentationItems?.count ?? 0) <= 64,
                  items.allSatisfy({ $0.actions.count <= 64 && $0.actions.allSatisfy { $0.inputFields.count <= 32 } })
            else { throw invalid("result count limit exceeded") }
            event = .searchResults(
                queryId: queryId,
                items: items,
                presentationItems: envelope.presentationItems ?? []
            )
        case "configuration_items":
            guard let requestId = envelope.requestId,
                  let items = envelope.configurationItems, items.count <= 10_000 else { throw invalid("invalid configuration items") }
            event = .configurationItems(requestId: requestId, items: items)
        case "notification":
            guard let notification = envelope.notification else { throw invalid("missing notification") }
            event = .notification(notification)
        case "catalog_changed":
            event = .catalogChanged
        case "action_finished":
            guard let requestId = envelope.requestId, let success = envelope.success else { throw invalid("missing action identity or outcome") }
            event = .actionFinished(
                requestId: requestId,
                success: success,
                message: envelope.message
            )
        case "error":
            guard envelope.queryId == nil || envelope.requestId == nil else { throw invalid("ambiguous error identity") }
            event = .error(PluginFailure(message: envelope.message ?? "Plugin error",
                queryId: envelope.queryId, requestId: envelope.requestId,
                code: envelope.code, retryable: envelope.retryable ?? false))
        default:
            event = nil
        }
        if let event { deliver(event, bytes: data.count) }
    }
}

final class SearchCoordinator {
    var onResultsChanged: (([SearchResult]) -> Void)?
    var onStatusChanged: ((String) -> Void)?
    var onPluginStateChanged: (() -> Void)?
    var onNotification: ((PluginNotification) -> Void)?
    var onSystemNotification: ((PluginNotification) -> Void)?
    private(set) var descriptors: [PluginDescriptor]

    private var processes: [String: PluginProcess] = [:]
    private var resultsByPlugin: [String: [SearchResult]] = [:]
    private var stagedResultsByPlugin: [String: [SearchResult]] = [:]
    private var presentationItemsByPlugin: [String: [PluginPresentationItem]] = [:]
    private var currentQuery = ""
    private var currentPluginIds: Set<String> = []
    private var currentSearchIsGlobal = true
    private var currentShowsAllPluginItems = false
    private var queryId: UInt64 = 0
    private let searchActivation = SearchActivation()
    private var activationGeneration: UInt64 = 0
    private var requestId: UInt64 = 0
    private var actionCompletions: [UInt64: (pluginId: String, completion: (Bool) -> Void)] = [:]
    private var configurationCompletions: [
        UInt64: (pluginId: String, completion: ([PluginConfigurationItem]) -> Void)
    ] = [:]
    private var restartAttempts: [String: Int] = [:]
    private var pendingSearches: [String: UInt64] = [:]

    private let searchTimeout: TimeInterval

    init(descriptors: [PluginDescriptor], searchTimeout: TimeInterval = 2) {
        self.searchTimeout = searchTimeout
        self.descriptors = descriptors
        ClipboardPreference.registerDefaults()
        CalendarPreference.registerDefaults()
        ClashPreference.registerDefaults()
        for plugin in descriptors where isEnabled(plugin.id) {
            start(plugin)
        }
        searchGlobal("")
    }

    func updatePlugins(_ updated: [PluginDescriptor]) {
        guard descriptors != updated else { return }
        cancelPendingActivation()
        let changed = descriptors.filter { old in !updated.contains(old) }.map(\.id)
        descriptors = updated
        for id in changed {
            stopPlugin(id)
            restartAttempts[id] = nil
        }
        for plugin in updated where isEnabled(plugin.id) { start(plugin) }
        currentPluginIds = currentPluginIds.intersection(Set(updated.map(\.id)))
        repeatCurrentSearch()
        onPluginStateChanged?()
    }

    private func stopPlugin(_ id: String) {
        let process = processes.removeValue(forKey: id)
        process?.stop()
        pendingSearches[id] = nil
        resultsByPlugin[id] = nil
        stagedResultsByPlugin[id] = nil
        presentationItemsByPlugin[id] = nil
        let actions = actionCompletions.filter { $0.value.pluginId == id }
        actions.keys.forEach { actionCompletions[$0] = nil }
        let configurations = configurationCompletions.filter { $0.value.pluginId == id }
        configurations.keys.forEach { configurationCompletions[$0] = nil }
        actions.values.forEach { $0.completion(false) }
        configurations.values.forEach { $0.completion([]) }
    }

    func isEnabled(_ id: String) -> Bool {
        guard descriptors.contains(where: { $0.id == id }) else { return false }
        let key = "plugin.enabled.\(id)"
        let defaultEnabled = descriptors.first(where: { $0.id == id })?.defaultEnabled ?? false
        return UserDefaults.standard.object(forKey: key) == nil
            ? defaultEnabled
            : UserDefaults.standard.bool(forKey: key)
    }

    func presentationItems(for pluginId: String) -> [PluginPresentationItem] {
        presentationItemsByPlugin[pluginId] ?? []
    }

    func setEnabled(_ enabled: Bool, descriptor: PluginDescriptor) {
        UserDefaults.standard.set(enabled, forKey: "plugin.enabled.\(descriptor.id)")
        if enabled {
            start(descriptor)
        } else {
            stopPlugin(descriptor.id)
        }
        repeatCurrentSearch()
        onPluginStateChanged?()
    }

    func isIncludedInGlobalSearch(_ id: String) -> Bool {
        guard id != "io.clipkeep.clipboard" else { return false }
        let key = "plugin.globalSearch.\(id)"
        let defaultValue = descriptors.first(where: { $0.id == id })?.defaultIncludedInGlobalSearch ?? false
        return UserDefaults.standard.object(forKey: key) == nil
            ? defaultValue
            : UserDefaults.standard.bool(forKey: key)
    }

    func setIncludedInGlobalSearch(_ included: Bool, descriptor: PluginDescriptor) {
        guard descriptor.id != "io.clipkeep.clipboard" else { return }
        UserDefaults.standard.set(included, forKey: "plugin.globalSearch.\(descriptor.id)")
        if currentSearchIsGlobal { searchGlobal(currentQuery) }
        onPluginStateChanged?()
    }

    func mainItemName(for descriptor: PluginDescriptor) -> String {
        let key = "plugin.mainItemName.\(descriptor.id)"
        let configured = UserDefaults.standard.string(forKey: key)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return configured.isEmpty ? descriptor.name : configured
    }

    func setMainItemName(_ name: String, descriptor: PluginDescriptor) {
        let key = "plugin.mainItemName.\(descriptor.id)"
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.isEmpty {
            UserDefaults.standard.removeObject(forKey: key)
        } else {
            UserDefaults.standard.set(name, forKey: key)
        }
        if currentSearchIsGlobal { publishResults() }
    }

    func searchGlobal(_ query: String, preservingPublishedResults: Bool = false) {
        let pluginIds = descriptors
            .filter { isEnabled($0.id) && isIncludedInGlobalSearch($0.id) }
            .map(\.id)
        search(
            query,
            pluginIds: pluginIds,
            isGlobal: true,
            preservingPublishedResults: preservingPublishedResults
        )
    }

    func search(_ query: String, pluginId: String, preservingPublishedResults: Bool = false) {
        search(
            query,
            pluginIds: [pluginId],
            isGlobal: false,
            preservingPublishedResults: preservingPublishedResults
        )
    }

    func searchPluginEntries(_ query: String, preservingPublishedResults: Bool = false) {
        search(
            query,
            pluginIds: [],
            isGlobal: false,
            preservingPublishedResults: preservingPublishedResults,
            showsAllPluginItems: true
        )
    }

    private func search(
        _ query: String,
        pluginIds: [String],
        isGlobal: Bool,
        preservingPublishedResults: Bool,
        showsAllPluginItems: Bool = false
    ) {
        let requestedPluginIds = Set(pluginIds)
        let availablePluginIds = requestedPluginIds.filter { processes[$0] != nil }
        let targetChanged = currentPluginIds != requestedPluginIds
            || currentSearchIsGlobal != isGlobal
            || currentShowsAllPluginItems != showsAllPluginItems
        currentQuery = query
        currentPluginIds = requestedPluginIds
        currentSearchIsGlobal = isGlobal
        currentShowsAllPluginItems = showsAllPluginItems
        for (pluginId, oldQuery) in pendingSearches { processes[pluginId]?.cancelSearch(queryId: oldQuery) }
        queryId += 1
        searchActivation.begin(queryId: queryId)
        pendingSearches.removeAll()
        stagedResultsByPlugin.removeAll()
        if targetChanged {
            resultsByPlugin.removeAll()
            if !preservingPublishedResults {
                publishResults()
            }
        }
        guard !availablePluginIds.isEmpty else {
            resultsByPlugin.removeAll()
            publishResults()
            searchActivation.finish(queryId: queryId)
            return
        }
        let sentQueryId = queryId
        for pluginId in availablePluginIds {
            guard let process = processes[pluginId] else { continue }
            pendingSearches[pluginId] = sentQueryId
            process.search(queryId: sentQueryId, query: query, timeout: searchTimeout)
            DispatchQueue.main.asyncAfter(deadline: .now() + searchTimeout) { [weak self, weak process] in
                guard let self, let process,
                      self.processes[pluginId] === process,
                      self.pendingSearches[pluginId] == sentQueryId else { return }
                let name = self.descriptors.first(where: { $0.id == pluginId })?.name ?? pluginId
                self.onStatusChanged?("\(name) search timed out")
                self.failSearch(pluginId: pluginId, queryId: sentQueryId)
            }
        }
        // Publish fast plugins together instead of flashing an intermediate empty state.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) { [weak self] in
            self?.commitStagedResults(queryId: sentQueryId)
        }
    }

    func activateWhenSearchSettles(_ action: @escaping () -> Void) {
        activationGeneration += 1
        let expectedGeneration = activationGeneration
        let expectedQueryId = queryId
        searchActivation.request(action)
        // A hung plugin must not cause an unexpected action much later.
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            guard let self, self.queryId == expectedQueryId,
                  self.activationGeneration == expectedGeneration else { return }
            self.cancelPendingActivation()
        }
    }

    func cancelPendingActivation() {
        activationGeneration += 1
        searchActivation.cancel()
    }

    func canExecute(_ result: SearchResult) -> Bool {
        guard isEnabled(result.pluginId), let process = processes[result.pluginId] else { return false }
        return result.metadata["_nexus_process"] == process.instanceId
    }

    func execute(
        _ result: SearchResult,
        actionId: String,
        input: String? = nil,
        completion: ((Bool) -> Void)? = nil
    ) {
        guard let descriptor = descriptors.first(where: { $0.id == result.pluginId }),
              canExecute(result) else { completion?(false); return }
        do {
            if try PluginHostAction.resolve(result: result, actionId: actionId, descriptor: descriptor) == .media {
                guard let command = SystemMediaController.command(for: result.hostParameter("command", actionId: actionId) ?? "") else {
                    throw PluginInstallationError.invalid("Unknown media command")
                }
                let success = SystemMediaController.perform(command)
                onStatusChanged?(success ? command.confirmation : "Allow Accessibility access to control media playback")
                if success {
                    onNotification?(PluginNotification(
                        id: "system-media:\(UUID().uuidString)", title: descriptor.name, body: command.confirmation
                    ))
                }
                completion?(success)
                return
            }
        } catch {
            onStatusChanged?(error.localizedDescription)
            completion?(false)
            return
        }
        execute(
            pluginId: result.pluginId,
            resultId: result.id,
            actionId: actionId,
            input: input,
            completion: completion,
            arguments: result.actions.first(where: { $0.id == actionId })?.inputFields.isEmpty == false
                ? input?.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } : nil
        )
    }

    func execute(pluginId: String, actionId: String, completion: ((Bool) -> Void)? = nil) {
        execute(
            pluginId: pluginId,
            resultId: "plugin:settings",
            actionId: actionId,
            input: nil,
            completion: completion
        )
    }

    private func execute(
        pluginId: String,
        resultId: String,
        actionId: String,
        input: String?,
        completion: ((Bool) -> Void)?,
        arguments: [String: Any]? = nil
    ) {
        guard let process = processes[pluginId] else { completion?(false); return }
        requestId += 1
        let sentRequestId = requestId
        actionCompletions[sentRequestId] = (pluginId, completion ?? { _ in })
        process.execute(
            requestId: sentRequestId,
            resultId: resultId,
            actionId: actionId,
            input: input,
            arguments: arguments
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

    func configureRuntimeSettings(_ settings: [String: String], for pluginId: String) {
        processes[pluginId]?.configure(settings: settings)
    }

    func loadConfigurationItems(
        for pluginId: String,
        completion: @escaping ([PluginConfigurationItem]) -> Void
    ) {
        guard let process = processes[pluginId] else {
            completion([])
            return
        }
        requestId += 1
        let sentRequestId = requestId
        configurationCompletions[sentRequestId] = (pluginId, completion)
        process.listConfigurationItems(requestId: sentRequestId)
        DispatchQueue.main.asyncAfter(deadline: .now() + 5) { [weak self, weak process] in
            guard let self, let process,
                  self.processes[pluginId] === process,
                  let pending = self.configurationCompletions.removeValue(forKey: sentRequestId)
            else { return }
            pending.completion([])
        }
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

        } catch {
            onStatusChanged?("Could not start \(descriptor.name): \(error.localizedDescription)")
        }
    }

    func configurationSchema(for pluginId: String) -> [PluginConfigurationField] {
        processes[pluginId]?.configurationSchema ?? []
    }

    private func settings(for pluginId: String) -> [String: String] {
        let base: [String: String] = switch pluginId {
        case "io.clipkeep.clipboard": ClipboardPreference.pluginSettings
        case "io.clipkeep.calendar": CalendarPreference.pluginSettings
        case "io.clipkeep.clash": ClashPreference.pluginSettings
        case "io.clipkeep.applications", "io.clipkeep.system-commands":
            PluginItemPreference.pluginSettings(for: pluginId)
        default: PluginItemPreference.pluginSettings(for: pluginId)
        }
        return base.merging(PluginConfigurationPreference.settings(for: pluginId)) { _, custom in custom }
    }

    private func handle(_ event: PluginProcess.Event, from pluginId: String, process: PluginProcess) {
        guard processes[pluginId] === process else { return }
        switch event {
        case let .initialized(actualPluginId, protocolVersion, features, schema, requiredFeatures, requiredServices):
            guard actualPluginId == pluginId, protocolVersion == 1 else {
                onStatusChanged?("Rejected incompatible plugin: \(pluginId)")
                failSearch(pluginId: pluginId, queryId: queryId)
                stopPlugin(pluginId)
                return
            }
            do {
                try PluginContract.validate(requiredFeatures: requiredFeatures, requiredServices: requiredServices, schema: schema)
            } catch {
                onStatusChanged?(error.localizedDescription)
                failSearch(pluginId: pluginId, queryId: queryId)
                stopPlugin(pluginId)
                return
            }
            process.acceptFeatures(features, schema: schema)
            process.markReady(settings: settings(for: pluginId))
            onPluginStateChanged?()
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self, weak process] in
                guard let self, let process, self.processes[pluginId] === process else { return }
                self.restartAttempts[pluginId] = 0
            }
        case let .searchResults(eventQueryId, items, presentationItems)
            where eventQueryId == queryId && pendingSearches[pluginId] == eventQueryId:
            pendingSearches[pluginId] = nil
            stagedResultsByPlugin[pluginId] = items.filter { $0.pluginId == pluginId }.map { item in
                var item = item
                item.metadata["_nexus_process"] = process.instanceId
                return item
            }
            presentationItemsByPlugin[pluginId] = presentationItems
            if pendingSearches.isEmpty {
                commitStagedResults(queryId: eventQueryId)
                searchActivation.finish(queryId: eventQueryId)
            }
        case let .configurationItems(requestId, items):
            guard let pending = configurationCompletions[requestId],
                  pending.pluginId == pluginId else { break }
            configurationCompletions[requestId] = nil
            pending.completion(items)
        case let .notification(notification):
            if pluginId == "io.clipkeep.reminders" {
                onSystemNotification?(notification)
            } else {
                onNotification?(notification)
            }
            if currentPluginIds.contains(pluginId) {
                repeatCurrentSearch()
            }
        case .catalogChanged where currentPluginIds.contains(pluginId):
            repeatCurrentSearch()
        case let .actionFinished(requestId, success, message):
            guard actionCompletions[requestId]?.pluginId == pluginId else { break }
            onStatusChanged?(message ?? "Done")
            if success, let message, !message.isEmpty {
                let pluginName = descriptors.first(where: { $0.id == pluginId })?.name ?? "Nexus"
                onNotification?(PluginNotification(
                    id: "action:\(pluginId):\(requestId)",
                    title: pluginName,
                    body: message
                ))
            }
            actionCompletions.removeValue(forKey: requestId)?.completion(success)
            repeatCurrentSearch()
        case let .error(failure):
            if let failedQuery = failure.queryId {
                guard pendingSearches[pluginId] == failedQuery else { break }
                onStatusChanged?(failure.message)
                failSearch(pluginId: pluginId, queryId: failedQuery)
            } else if let request = failure.requestId {
                if actionCompletions[request]?.pluginId == pluginId {
                    onStatusChanged?(failure.message)
                    actionCompletions.removeValue(forKey: request)?.completion(false)
                } else if configurationCompletions[request]?.pluginId == pluginId {
                    onStatusChanged?(failure.message)
                    configurationCompletions.removeValue(forKey: request)?.completion([])
                }
            } else {
                // Legacy unscoped errors may come from background work. Do not
                // attribute them to the latest search; its deadline still applies.
                onStatusChanged?(failure.message)
            }
        case .terminated:
            guard processes[pluginId] === process else { return }
            failSearch(pluginId: pluginId, queryId: queryId)
            cancelPendingActivation()
            let failedActions = actionCompletions.filter { $0.value.pluginId == pluginId }
            failedActions.keys.forEach { actionCompletions[$0] = nil }
            failedActions.values.forEach { $0.completion(false) }
            let failedConfigurations = configurationCompletions.filter { $0.value.pluginId == pluginId }
            failedConfigurations.keys.forEach { configurationCompletions[$0] = nil }
            failedConfigurations.values.forEach { $0.completion([]) }
            processes[pluginId] = nil
            resultsByPlugin[pluginId] = nil
            stagedResultsByPlugin[pluginId] = nil
            publishResults()
            scheduleRestart(pluginId: pluginId)
        default:
            break
        }
    }

    private func failSearch(pluginId: String, queryId failedQuery: UInt64) {
        guard failedQuery == queryId, pendingSearches[pluginId] == failedQuery else { return }
        cancelPendingActivation()
        processes[pluginId]?.cancelSearch(queryId: failedQuery)
        pendingSearches[pluginId] = nil
        stagedResultsByPlugin[pluginId] = []
        presentationItemsByPlugin[pluginId] = []
        commitStagedResults(queryId: failedQuery)
        if pendingSearches.isEmpty { searchActivation.finish(queryId: failedQuery) }
    }

    private func scheduleRestart(pluginId: String) {
        guard isEnabled(pluginId), let descriptor = descriptors.first(where: { $0.id == pluginId }) else { return }
        let expectedDescriptor = descriptor
        let attempt = (restartAttempts[pluginId] ?? 0) + 1
        restartAttempts[pluginId] = attempt
        guard attempt <= 3 else {
            onStatusChanged?("\(descriptor.name) stopped after repeated crashes")
            return
        }
        onStatusChanged?("Restarting \(descriptor.name)…")
        DispatchQueue.main.asyncAfter(deadline: .now() + Double(attempt)) { [weak self] in
            guard let self, self.isEnabled(pluginId), self.processes[pluginId] == nil,
                  self.descriptors.contains(expectedDescriptor) else { return }
            self.start(descriptor)
            if self.currentSearchIsGlobal && self.isIncludedInGlobalSearch(pluginId) {
                self.searchGlobal(self.currentQuery)
            } else if self.currentPluginIds.contains(pluginId) {
                self.search(self.currentQuery, pluginId: pluginId)
            }
        }
    }

    private func repeatCurrentSearch() {
        if currentShowsAllPluginItems {
            searchPluginEntries(currentQuery)
        } else if currentSearchIsGlobal {
            searchGlobal(currentQuery)
        } else if let pluginId = currentPluginIds.first {
            search(currentQuery, pluginId: pluginId)
        } else {
            resultsByPlugin.removeAll()
            publishResults()
        }
    }

    private func commitStagedResults(queryId expectedQueryId: UInt64) {
        guard expectedQueryId == queryId, !stagedResultsByPlugin.isEmpty else { return }
        let hasAnyResult = stagedResultsByPlugin.values.contains { !$0.isEmpty }
        if !pendingSearches.isEmpty, !hasAnyResult, !resultsByPlugin.isEmpty { return }
        resultsByPlugin = stagedResultsByPlugin
        publishResults()
    }

    private func publishResults() {
        var results = resultsByPlugin.values
            .flatMap { $0 }
            .filter { result in
                guard let pluginId = result.metadata["route_plugin_id"] else { return true }
                return isEnabled(pluginId)
            }

        if currentSearchIsGlobal {
            results.append(contentsOf: mainItemResults())
            if let pluginsResult = pluginsRouteResult() {
                results.append(pluginsResult)
            }
            if let settingsResult = settingsRouteResult() {
                results.append(settingsResult)
            }
        } else if currentShowsAllPluginItems {
            results.append(contentsOf: pluginEntryResults(includeGlobalSearchPlugins: true))
        }

        if currentSearchIsGlobal {
            results = SearchRanking.sorted(results, query: currentQuery)
        } else {
            results.sort {
                if $0.score == $1.score { return $0.title < $1.title }
                return $0.score > $1.score
            }
        }
        onResultsChanged?(results)
    }

    private func mainItemResults() -> [SearchResult] {
        pluginEntryResults(includeGlobalSearchPlugins: false)
    }

    private func pluginEntryResults(includeGlobalSearchPlugins: Bool) -> [SearchResult] {
        descriptors.compactMap { descriptor in
            guard isEnabled(descriptor.id),
                  includeGlobalSearchPlugins || !isIncludedInGlobalSearch(descriptor.id)
            else { return nil }
            let itemName = mainItemName(for: descriptor)
            guard let score = mainItemScore(
                query: currentQuery,
                candidates: [itemName, descriptor.name, descriptor.id]
            ) else { return nil }
            return SearchResult(
                pluginId: descriptor.id,
                id: "plugin-route:\(descriptor.id)",
                title: itemName,
                subtitle: "Open \(descriptor.name)",
                preview: "Search with \(descriptor.name)",
                kind: .command,
                score: score,
                metadata: [
                    "route_plugin_id": descriptor.id,
                    "symbol_name": descriptor.symbolName,
                ],
                actions: []
            )
        }
    }

    private func pluginsRouteResult() -> SearchResult? {
        guard let score = mainItemScore(query: currentQuery, candidates: ["Plugins", "Plugin"]) else {
            return nil
        }
        return SearchResult(
            pluginId: "io.clipkeep.plugins",
            id: "host-route:plugins",
            title: "Plugins",
            subtitle: "Browse all plugins",
            preview: "Open the plugin browser",
            kind: .command,
            score: score + 200,
            metadata: [
                "route_mode": "plugins",
                "symbol_name": "puzzlepiece.extension.fill",
            ],
            actions: []
        )
    }

    private func settingsRouteResult() -> SearchResult? {
        guard let score = mainItemScore(
            query: currentQuery,
            candidates: ["Nexus Settings", "Settings", "Preferences"]
        ) else { return nil }
        return SearchResult(
            pluginId: "io.clipkeep.settings",
            id: "host-route:settings",
            title: "Nexus Settings",
            subtitle: "Open Nexus settings",
            preview: "Open the Nexus settings window",
            kind: .command,
            score: score + 200,
            metadata: [
                "route_mode": "settings",
                "symbol_name": "gearshape.fill",
            ],
            actions: []
        )
    }

    private func mainItemScore(query: String, candidates: [String]) -> Double? {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return 2_000 }

        return candidates.compactMap { candidate -> Double? in
            let candidate = candidate.lowercased()
            if candidate == query { return 3_500 }
            if candidate.hasPrefix(query) { return 3_300 - Double(candidate.count) }
            if let range = candidate.range(of: query) {
                let position = candidate.distance(from: candidate.startIndex, to: range.lowerBound)
                return 3_000 - Double(position * 10)
            }

            var queryIndex = query.startIndex
            var gaps = 0
            var matched = false
            for character in candidate {
                guard queryIndex < query.endIndex else { break }
                if character == query[queryIndex] {
                    matched = true
                    query.formIndex(after: &queryIndex)
                } else if matched {
                    gaps += 1
                }
            }
            return queryIndex == query.endIndex ? 2_600 - Double(gaps * 5) : nil
        }.max()
    }
}
