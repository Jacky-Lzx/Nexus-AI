import AppKit
import XCTest
@testable import Nexus

@MainActor
private func textFields(in view: NSView) -> [NSTextField] {
    (view as? NSTextField).map { [$0] } ?? view.subviews.flatMap { textFields(in: $0) }
}

@MainActor
private func waitForInlineUpdate(_ predicate: () -> Bool) async -> Bool {
    for _ in 0..<150 {
        if predicate() { return true }
        try? await Task.sleep(for: .milliseconds(20))
    }
    return predicate()
}

@MainActor
final class InlineArgumentsTests: XCTestCase {
    func testInlineArgumentsSurviveTypingAndRefreshOnlyWhenCommandChanges() async throws {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nexus-inline-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let pluginID = "com.example.inline-\(UUID())"
        let executable = root.appendingPathComponent("plugin")
        try """
        #!/usr/bin/env python3
        import json, sys, time
        for line in sys.stdin:
            request = json.loads(line)
            if request['type'] == 'initialize':
                response = dict(type='initialized', plugin_id='\(pluginID)', protocol_version=1)
            elif request['type'] == 'search':
                time.sleep(0.08)
                query = request['query']
                field = dict(id='hours', label='hour', kind='integer', minimum=0)
                if query == 'schema':
                    field['minimum'] = 1
                if query == 'other':
                    field = dict(id='count', label='count', kind='integer')
                item = dict(plugin_id='\(pluginID)', id='other' if query == 'other' else 'awake',
                            title='Awake', kind='command', score=5000,
                            actions=[dict(id='run', title='Run', default=True, input_fields=[field])])
                response = dict(type='search_results', query_id=request['query_id'],
                                items=[] if query == 'no-match' else [item])
            elif request['type'] == 'shutdown':
                break
            else:
                continue
            print(json.dumps(response), flush=True)
        """.write(to: executable, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: executable.path)
        let descriptor = PluginDescriptor(
            id: pluginID, name: "Inline Test", version: "1", protocolVersion: 1,
            executable: "plugin", executableURL: executable,
            defaultEnabled: true, defaultIncludedInGlobalSearch: true, capabilities: ["search"]
        )
        let coordinator = SearchCoordinator(descriptors: [descriptor])
        defer { coordinator.updatePlugins([]) }
        let controller = LauncherPanelController(coordinator: coordinator)
        let content = try XCTUnwrap(controller.window?.contentView)
        let queryField = try XCTUnwrap(textFields(in: content).first {
            $0.placeholderString == SearchMode.global.placeholder
        })
        func field(_ label: String) -> NSTextField? {
            textFields(in: content).first { $0.accessibilityLabel() == label }
        }
        func typeQuery(_ query: String) {
            queryField.stringValue = query
            controller.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: queryField))
        }
        func settled() async -> Bool {
            var finished = false
            coordinator.activateWhenSearchSettles { finished = true }
            return await waitForInlineUpdate { finished }
        }

        func waitForSettled() async {
            let finished = await settled()
            XCTAssertTrue(finished)
        }

        typeQuery("c")
        await waitForSettled()
        let original = try XCTUnwrap(field("hour"))
        original.stringValue = "2"
        let initialWidth = try XCTUnwrap(queryField.constraints.first { $0.firstAttribute == .width }).constant
        for query in ["ca", "caf", "caff", "caffeinate", "caff"] {
            typeQuery(query)
            // Assert during the delayed search as well as after the result publication.
            XCTAssertTrue(field("hour") === original)
            await waitForSettled()
            XCTAssertTrue(field("hour") === original)
            XCTAssertTrue(original.stringValue == "2")
        }
        XCTAssertTrue(try XCTUnwrap(queryField.constraints.first { $0.firstAttribute == .width }).constant > initialWidth)

        typeQuery("schema")
        await waitForSettled()
        let updated = try XCTUnwrap(field("hour"))
        XCTAssertTrue(updated !== original)
        typeQuery("other")
        await waitForSettled()
        XCTAssertTrue(field("hour") == nil)
        XCTAssertTrue(field("count") != nil)

        typeQuery("no-match")
        await waitForSettled()
        XCTAssertTrue(field("count") == nil)
        typeQuery("c")
        await waitForSettled()
        XCTAssertTrue(field("hour") != nil)
        typeQuery("")
        XCTAssertTrue(field("hour") == nil)
    }
}
