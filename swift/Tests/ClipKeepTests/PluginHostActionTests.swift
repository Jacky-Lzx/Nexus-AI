import Foundation
import Testing
@testable import Nexus

@Test func thirdPartyPluginCanDeclareHostServicesWithoutBuiltinID() throws {
    let result = try JSONDecoder().decode(SearchResult.self, from: Data("""
    {"plugin_id":"com.example.audio","id":"quiet","title":"Quiet","kind":"command",
     "metadata":{"host_action.toggle":"system.media","command":"mute"}}
    """.utf8))
    let descriptor = PluginDescriptor(
        id: "com.example.audio", name: "Audio", version: "1", protocolVersion: 1,
        executable: "audio", executableURL: URL(fileURLWithPath: "/tmp/audio"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true, capabilities: ["system.control"]
    )
    #expect(try PluginHostAction.resolve(result: result, actionId: "toggle", descriptor: descriptor) == .media)
    #expect(try PluginHostAction.resolve(result: result, actionId: "other", descriptor: descriptor) == nil)
    #expect(SystemMediaController.command(for: result.metadata["command"]!)?.keyCode == 7)
}

@Test func undeclaredAndUnknownHostServicesAreRejected() throws {
    var result = try JSONDecoder().decode(SearchResult.self, from: Data("""
    {"plugin_id":"com.example.test","id":"one","title":"One","kind":"command",
     "metadata":{"host_action.run":"system.media"}}
    """.utf8))
    let descriptor = PluginDescriptor(
        id: "com.example.test", name: "Test", version: "1", protocolVersion: 1,
        executable: "test", executableURL: URL(fileURLWithPath: "/tmp/test"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true, capabilities: []
    )
    #expect(throws: (any Error).self) {
        try PluginHostAction.resolve(result: result, actionId: "run", descriptor: descriptor)
    }
    result.metadata["host_action.run"] = "unknown.service"
    #expect(throws: (any Error).self) {
        try PluginHostAction.resolve(result: result, actionId: "run", descriptor: descriptor)
    }
}
