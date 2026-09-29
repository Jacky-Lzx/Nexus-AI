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

@Test func notificationPermissionRequiresDeclaredCapability() throws {
    let result = try JSONDecoder().decode(SearchResult.self, from: Data("""
    {"plugin_id":"com.example.remind","id":"new","title":"Reminder","kind":"command",
     "metadata":{"host_action.create":"notifications.authorize"}}
    """.utf8))
    let descriptor = PluginDescriptor(
        id: "com.example.remind", name: "Reminders", version: "1", protocolVersion: 1,
        executable: "remind", executableURL: URL(fileURLWithPath: "/tmp/remind"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true, capabilities: ["notifications"]
    )
    #expect(try PluginHostAction.resolve(result: result, actionId: "create", descriptor: descriptor) == .notificationPermission)
    let withoutPermission = PluginDescriptor(
        id: descriptor.id, name: descriptor.name, version: descriptor.version,
        protocolVersion: descriptor.protocolVersion, executable: descriptor.executable,
        executableURL: descriptor.executableURL, defaultEnabled: true,
        defaultIncludedInGlobalSearch: true, capabilities: []
    )
    #expect(throws: (any Error).self) {
        try PluginHostAction.resolve(result: result, actionId: "create", descriptor: withoutPermission)
    }
}

@Test func typedHostServiceUsesItsOwnParametersAndCapabilityCheck() throws {
    let result = try JSONDecoder().decode(SearchResult.self, from: Data("""
    {"plugin_id":"com.example.audio","id":"quiet","title":"Quiet","kind":"command",
     "metadata":{"command":"volume-up"},
     "actions":[{"id":"toggle","title":"Mute","host_action":{"service":"system.media","parameters":{"command":"mute"}}}]}
    """.utf8))
    let descriptor = PluginDescriptor(
        id: "com.example.audio", name: "Audio", version: "1", protocolVersion: 1,
        executable: "audio", executableURL: URL(fileURLWithPath: "/tmp/audio"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true, capabilities: ["system.control"])
    #expect(try PluginHostAction.resolve(result: result, actionId: "toggle", descriptor: descriptor) == .media)
    #expect(result.hostParameter("command", actionId: "toggle") == "mute")
    #expect(result.hostParameter("target_pid", actionId: "toggle") == nil)
}

@Test func configurationTypesKeepBooleansAndNumbersDistinct() throws {
    let field = try JSONDecoder().decode(PluginConfigurationField.self, from: Data("""
    {"id":"count","label":"Count","kind":"integer","default":2,"minimum":1,"maximum":10}
    """.utf8))
    #expect(field.parse("3") == .integer(3))
    #expect(field.parse("3.5") == nil)
    #expect(field.parse("true") == nil)
    #expect(field.parse("11") == nil)
    let value = try JSONDecoder().decode(PluginValue.self, from: Data("{\"flag\":true,\"count\":2,\"text\":\"2\"}".utf8))
    #expect(value == .object(["flag": .boolean(true), "count": .integer(2), "text": .string("2")]))
    #expect(try JSONDecoder().decode(PluginValue.self, from: JSONEncoder().encode(value)) == value)
}
