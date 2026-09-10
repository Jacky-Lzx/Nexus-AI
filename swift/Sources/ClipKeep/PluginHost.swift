import Foundation

final class PluginProcess {
    enum Event {
        case initialized(pluginId: String, protocolVersion: Int)
        case searchResults(
            queryId: UInt64,
            items: [SearchResult],
            presentationItems: [PluginPresentationItem]
        )
        case configurationItems(requestId: UInt64, items: [PluginConfigurationItem])
        case notification(PluginNotification)
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
        let presentationItems: [PluginPresentationItem]?
        let configurationItems: [PluginConfigurationItem]?
        let notification: PluginNotification?
        let requestId: UInt64?
        let success: Bool?
        let message: String?
        let pluginId: String?
        let protocolVersion: Int?

        enum CodingKeys: String, CodingKey {
            case type, items, notification, success, message
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

    func listConfigurationItems(requestId: UInt64) {
        send([
            "type": "list_configuration_items",
            "request_id": requestId,
        ])
    }

    func configure(settings: [String: String]) {
        send(["type": "configure", "settings": settings])
    }

    func execute(requestId: UInt64, resultId: String, actionId: String, input: String? = nil) {
        var request: [String: Any] = [
            "type": "execute",
            "request_id": requestId,
            "result_id": resultId,
            "action_id": actionId,
        ]
        if let input { request["input"] = input }
        send(request)
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
            event = .searchResults(
                queryId: queryId,
                items: items,
                presentationItems: envelope.presentationItems ?? []
            )
        case "configuration_items":
            guard let requestId = envelope.requestId,
                  let items = envelope.configurationItems else { return }
            event = .configurationItems(requestId: requestId, items: items)
        case "notification":
            guard let notification = envelope.notification else { return }
            event = .notification(notification)
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
    var onNotification: ((PluginNotification) -> Void)?
    let descriptors: [PluginDescriptor]

    private var processes: [String: PluginProcess] = [:]
    private var resultsByPlugin: [String: [SearchResult]] = [:]
    private var stagedResultsByPlugin: [String: [SearchResult]] = [:]
    private var presentationItemsByPlugin: [String: [PluginPresentationItem]] = [:]
    private var currentQuery = ""
    private var currentPluginIds: Set<String> = []
    private var currentSearchIsGlobal = true
    private var currentShowsAllPluginItems = false
    private var queryId: UInt64 = 0
    private var requestId: UInt64 = 0
    private var actionCompletions: [UInt64: (pluginId: String, completion: (Bool) -> Void)] = [:]
    private var configurationCompletions: [
        UInt64: (pluginId: String, completion: ([PluginConfigurationItem]) -> Void)
    ] = [:]
    private var restartAttempts: [String: Int] = [:]
    private var pendingSearches: [String: UInt64] = [:]

    init(descriptors: [PluginDescriptor]) {
        self.descriptors = descriptors
        ClipboardPreference.registerDefaults()
        CalendarPreference.registerDefaults()
        ClashPreference.registerDefaults()
        for plugin in descriptors where isEnabled(plugin.id) {
            start(plugin)
        }
        searchGlobal("")
    }

    func isEnabled(_ id: String) -> Bool {
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
            processes[descriptor.id] = nil
            resultsByPlugin[descriptor.id] = nil
            stagedResultsByPlugin[descriptor.id] = nil
            presentationItemsByPlugin[descriptor.id] = nil
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
        queryId += 1
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
            return
        }
        let sentQueryId = queryId
        for pluginId in availablePluginIds {
            guard let process = processes[pluginId] else { continue }
            pendingSearches[pluginId] = sentQueryId
            process.search(queryId: sentQueryId, query: query)
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self, weak process] in
                guard let self, let process,
                      self.processes[pluginId] === process,
                      self.pendingSearches[pluginId] == sentQueryId else { return }
                let name = self.descriptors.first(where: { $0.id == pluginId })?.name ?? pluginId
                self.onStatusChanged?("\(name) is taking longer than expected…")
                if targetChanged, preservingPublishedResults {
                    self.publishResults()
                }
            }
        }
        // Publish fast plugins together instead of flashing an intermediate empty state.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) { [weak self] in
            self?.commitStagedResults(queryId: sentQueryId)
        }
    }

    func execute(
        _ result: SearchResult,
        actionId: String,
        input: String? = nil,
        completion: ((Bool) -> Void)? = nil
    ) {
        execute(
            pluginId: result.pluginId,
            resultId: result.id,
            actionId: actionId,
            input: input,
            completion: completion
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
        completion: ((Bool) -> Void)?
    ) {
        guard let process = processes[pluginId] else { completion?(false); return }
        requestId += 1
        let sentRequestId = requestId
        if let completion { actionCompletions[sentRequestId] = (pluginId, completion) }
        process.execute(
            requestId: sentRequestId,
            resultId: resultId,
            actionId: actionId,
            input: input
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
            plugin.configure(settings: settings(for: descriptor.id))
        } catch {
            onStatusChanged?("Could not start \(descriptor.name): \(error.localizedDescription)")
        }
    }

    private func settings(for pluginId: String) -> [String: String] {
        switch pluginId {
        case "io.clipkeep.clipboard": ClipboardPreference.pluginSettings
        case "io.clipkeep.calendar": CalendarPreference.pluginSettings
        case "io.clipkeep.clash": ClashPreference.pluginSettings
        case "io.clipkeep.applications", "io.clipkeep.system-commands":
            PluginItemPreference.pluginSettings(for: pluginId)
        default: [:]
        }
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
        case let .searchResults(eventQueryId, items, presentationItems)
            where eventQueryId == queryId && currentPluginIds.contains(pluginId):
            pendingSearches[pluginId] = nil
            stagedResultsByPlugin[pluginId] = items
            presentationItemsByPlugin[pluginId] = presentationItems
            if pendingSearches.isEmpty {
                commitStagedResults(queryId: eventQueryId)
            }
        case let .configurationItems(requestId, items):
            guard let pending = configurationCompletions[requestId],
                  pending.pluginId == pluginId else { break }
            configurationCompletions[requestId] = nil
            pending.completion(items)
        case let .notification(notification):
            onNotification?(notification)
            if currentPluginIds.contains(pluginId) {
                repeatCurrentSearch()
            }
        case .catalogChanged where currentPluginIds.contains(pluginId):
            repeatCurrentSearch()
        case let .actionFinished(requestId, success, message):
            onStatusChanged?(message ?? "Done")
            actionCompletions.removeValue(forKey: requestId)?.completion(success)
            repeatCurrentSearch()
        case let .error(message):
            onStatusChanged?(message)
        case .terminated:
            guard processes[pluginId] === process else { return }
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
        } else if currentShowsAllPluginItems {
            results.append(contentsOf: pluginEntryResults(includeGlobalSearchPlugins: true))
        }

        results.sort {
            if $0.score == $1.score { return $0.title < $1.title }
            return $0.score > $1.score
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
