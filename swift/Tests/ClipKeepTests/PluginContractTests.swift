import Foundation
import Testing
@testable import Nexus

private struct WireFixture: Decodable {
    let name: String
    let direction: String
    let message: PluginValue
}

@Test func sharedWireFixturesDecodeAndNegotiateInSwift() throws {
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    let cases = try JSONDecoder().decode([WireFixture].self, from: Data(contentsOf: root.appendingPathComponent("protocol/fixtures/v1.json")))
    for fixture in cases {
        let data = try JSONEncoder().encode(fixture.message)
        if fixture.direction == "event" {
            let event = try JSONDecoder().decode(PluginProcess.EventEnvelope.self, from: data)
            if event.type == "initialized" {
                try PluginContract.validate(requiredFeatures: event.requiredFeatures ?? [], requiredServices: event.requiredServices ?? [:], schema: event.configurationSchema ?? [])
            }
            if fixture.name == "typed_results" {
                #expect(event.items?.first?.hostParameter("command", actionId: "run") == "mute")
            }
            if fixture.name == "status_snapshot" {
                #expect(event.presentationItems?.first?.value == "Running")
                #expect(event.presentationItems?.first?.compactValue == "On")
            }
            if fixture.name == "status_cleared" {
                #expect(event.presentationItems?.isEmpty == true)
            }
        } else if fixture.name == "modern_initialize" {
            let actual = try JSONDecoder().decode(PluginValue.self, from: JSONSerialization.data(withJSONObject: PluginContract.initializeRequest))
            #expect(actual == fixture.message)
        }
    }
}

@Test func rejectsUnsupportedRequirementsAndInvalidSchema() throws {
    #expect(throws: (any Error).self) { try PluginContract.validate(requiredFeatures: ["future.feature"], requiredServices: [:], schema: []) }
    #expect(throws: (any Error).self) { try PluginContract.validate(requiredFeatures: [], requiredServices: ["system.media": 2], schema: []) }
    let invalid = try JSONDecoder().decode(PluginConfigurationField.self, from: Data("""
    {"id":"count","label":"Count","kind":"integer","default":true}
    """.utf8))
    #expect(throws: (any Error).self) { try PluginContract.validate(requiredFeatures: [], requiredServices: [:], schema: [invalid]) }
}
