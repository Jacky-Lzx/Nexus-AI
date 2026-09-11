import Foundation
import Testing
@testable import Nexus

private func temporaryDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("nexus-test-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

private func fixture(_ root: URL, id: String = "com.example.test", version: String = "1.0", delay: Double = 0) throws -> URL {
    let bundle = root.appendingPathComponent("\(id).clipkeepplugin")
    try FileManager.default.createDirectory(at: bundle, withIntermediateDirectories: true)
    try """
    id = "\(id)"
    name = "Test Plugin"
    version = "\(version)"
    protocol_version = 1
    entrypoint = "plugin"
    default_enabled = true
    include_in_global_search = true
    icon = "star"
    description = "An independently installed plugin"
    capabilities = ["search"]
    """.write(to: bundle.appendingPathComponent("plugin.toml"), atomically: true, encoding: .utf8)
    let executable = bundle.appendingPathComponent("plugin")
    try """
    #!/usr/bin/env python3
    import json, sys, time
    for line in sys.stdin:
        request = json.loads(line)
        if request['type'] == 'initialize':
            response = dict(type='initialized', plugin_id='\(id)', protocol_version=1)
        elif request['type'] == 'search':
            time.sleep(\(delay))
            response = dict(type='search_results', query_id=request['query_id'], items=[dict(
                plugin_id='\(id)', id='item', title='\(version):'+request['query'], kind='command', score=5000)])
        elif request['type'] == 'shutdown':
            break
        else:
            continue
        print(json.dumps(response), flush=True)
    """.write(to: executable, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: executable.path)
    return bundle
}

@Test func discoveryAllowsEmptyDirectoryAndUserOverrides() throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    #expect(PluginRegistry.scan(roots: [root]).descriptors.isEmpty)
    let builtIn = root.appendingPathComponent("builtin")
    let user = root.appendingPathComponent("user")
    _ = try fixture(builtIn)
    let override = try fixture(user, version: "2.0")
    let discovery = PluginRegistry.scan(roots: [builtIn, user])
    #expect(discovery.errors.isEmpty)
    #expect(discovery.descriptors.count == 1)
    #expect(discovery.descriptors.first?.version == "2.0")
    #expect(discovery.descriptors.first?.symbolName == "star")
    try FileManager.default.removeItem(at: override)
    #expect(PluginRegistry.scan(roots: [builtIn, user]).descriptors.first?.version == "1.0")
}

@Test func installingAndReplacingPluginDoesNotRequireHostChanges() throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let installation = PluginInstallation(directory: root.appendingPathComponent("installed"))
    let source = try fixture(root.appendingPathComponent("source"))
    let first = try installation.install(from: source)
    #expect(installation.isUserInstalled(first))
    let updated = try fixture(root.appendingPathComponent("update"), version: "2.0")
    let second = try installation.install(from: updated)
    #expect(first.bundleURL == second.bundleURL)
    #expect(first.revision != second.revision)
    #expect(second.version == "2.0")
    let invalid = try fixture(root.appendingPathComponent("invalid"), version: "3.0")
    try FileManager.default.removeItem(at: invalid.appendingPathComponent("plugin"))
    #expect(throws: (any Error).self) { try installation.install(from: invalid) }
    #expect(PluginRegistry.scan(roots: [installation.directory]).descriptors.first?.version == "2.0")
    let remaining = try FileManager.default.contentsOfDirectory(atPath: installation.directory.path)
    #expect(remaining == ["com.example.test.clipkeepplugin"])
}

@Test func rejectsEntrypointOutsideBundle() throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let bundle = try fixture(root)
    let executable = bundle.appendingPathComponent("plugin")
    try FileManager.default.removeItem(at: executable)
    try FileManager.default.createSymbolicLink(at: executable, withDestinationURL: URL(fileURLWithPath: "/bin/sh"))
    #expect(throws: (any Error).self) { try PluginRegistry.load(bundle) }
    #expect(PluginRegistry.scan(roots: [root]).errors.count == 1)
}

@MainActor
private func eventually(_ predicate: () -> Bool) async -> Bool {
    for _ in 0..<150 {
        if predicate() { return true }
        try? await Task.sleep(for: .milliseconds(20))
    }
    return predicate()
}

@Test @MainActor func addUpdateAndRemoveLivePluginProcesses() async throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let coordinator = SearchCoordinator(descriptors: [])
    defer { coordinator.updatePlugins([]) }
    var titles: [String] = []
    var latestResult: SearchResult?
    coordinator.onResultsChanged = {
        titles = $0.map(\.title)
        latestResult = $0.first
    }
    let bundle = try fixture(root)
    coordinator.updatePlugins([try PluginRegistry.load(bundle)])
    coordinator.searchGlobal("first")
    #expect(await eventually { titles.contains("1.0:first") })
    let retiredResult = try #require(latestResult)
    #expect(coordinator.canExecute(retiredResult))
    _ = try fixture(root, version: "2.0")
    coordinator.updatePlugins([try PluginRegistry.load(bundle)])
    #expect(await eventually { titles.contains("2.0:first") })
    #expect(!coordinator.canExecute(retiredResult))
    #expect(coordinator.canExecute(try #require(latestResult)))
    coordinator.searchGlobal("pending")
    var activated = false
    coordinator.activateWhenSearchSettles { activated = true }
    coordinator.updatePlugins([])
    try? await Task.sleep(for: .milliseconds(200))
    #expect(!activated)
    #expect(titles.isEmpty)
    #expect(!coordinator.isEnabled("com.example.test"))
}

@Test @MainActor func rapidQueryAndReturnWaitForRealPluginResponse() async throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let bundle = try fixture(root, delay: 0.04)
    let coordinator = SearchCoordinator(descriptors: [try PluginRegistry.load(bundle)])
    defer { coordinator.updatePlugins([]) }
    var selection: String?
    var opened: String?
    coordinator.onResultsChanged = { selection = $0.first?.title }
    coordinator.searchGlobal("m")
    coordinator.searchGlobal("mu")
    coordinator.searchGlobal("mut")
    coordinator.searchGlobal("mute")
    coordinator.activateWhenSearchSettles { opened = selection }
    #expect(opened == nil)
    #expect(await eventually { opened != nil })
    #expect(opened == "1.0:mute")
}

@Test func manifestCommentsAndValidation() throws {
    let root = try temporaryDirectory()
    defer { try? FileManager.default.removeItem(at: root) }
    let bundle = try fixture(root)
    let manifest = bundle.appendingPathComponent("plugin.toml")
    let source = try String(contentsOf: manifest, encoding: .utf8)
    try source.replacingOccurrences(of: "Test Plugin", with: "C# Plugin")
        .appending("\n# Comment\n")
        .write(to: manifest, atomically: true, encoding: .utf8)
    #expect(try PluginRegistry.load(bundle).name == "C# Plugin")
    try source.replacingOccurrences(of: "default_enabled = true", with: "default_enabled = invalid")
        .write(to: manifest, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try PluginRegistry.load(bundle) }
    try source.replacingOccurrences(of: "protocol_version = 1", with: "protocol_version = 2")
        .write(to: manifest, atomically: true, encoding: .utf8)
    #expect(throws: (any Error).self) { try PluginRegistry.load(bundle) }
}
