import AppKit
import XCTest
@testable import Nexus

@MainActor
private func descendant<T: NSView>(of view: NSView, matching predicate: (T) -> Bool) -> T? {
    if let match = view as? T, predicate(match) { return match }
    for child in view.subviews {
        if let match: T = descendant(of: child, matching: predicate) { return match }
    }
    return nil
}

@MainActor
final class LauncherAnimationTests: XCTestCase {
    func testResultsAreVisibleAndLaidOutBeforeExpansionFinishes() async throws {
        _ = NSApplication.shared
        let coordinator = SearchCoordinator(descriptors: [])
        let controller = LauncherPanelController(coordinator: coordinator)
        let panel = try XCTUnwrap(controller.window)
        panel.orderFrontRegardless()
        defer { panel.orderOut(nil) }
        let view = try XCTUnwrap(panel.contentView)
        let query: NSTextField = try XCTUnwrap(descendant(of: view) {
            $0.placeholderString == SearchMode.global.placeholder
        })
        let root = try XCTUnwrap(query.superview?.superview)
        let table: NSTableView = try XCTUnwrap(descendant(of: root) { _ in true })
        var content: NSView = table
        while let parent = content.superview, parent !== root { content = parent }
        XCTAssertTrue(content.superview === root)
        XCTAssertTrue(content.isHidden)

        func typeQuery(_ text: String) {
            query.stringValue = text
            controller.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: query))
        }
        func fixedHeight() -> NSLayoutConstraint? {
            content.constraints.first { $0.isActive && $0.firstAttribute == .height && $0.secondItem == nil }
        }

        typeQuery("a")
        // Check synchronously, before any animation completion callback can run.
        XCTAssertFalse(content.isHidden)
        XCTAssertTrue(root.layer?.masksToBounds == true)
        let height = try XCTUnwrap(fixedHeight())
        XCTAssertEqual(height.constant, 440, accuracy: 0.5)
        XCTAssertEqual(content.frame.height, height.constant, accuracy: 0.5)

        typeQuery("ab")
        XCTAssertTrue(fixedHeight() === height)
        XCTAssertFalse(content.isHidden)
        try await Task.sleep(for: .milliseconds(100))
        typeQuery("abc")
        try await Task.sleep(for: .milliseconds(140))
        // More typing must not restart an expansion that already targets this size.
        XCTAssertNil(fixedHeight())
        XCTAssertEqual(panel.frame.height, 520, accuracy: 0.5)

        // An older expansion completion must not reveal a collapsed result area.
        typeQuery("")
        XCTAssertTrue(content.isHidden)
        XCTAssertNil(fixedHeight())
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertTrue(content.isHidden)
        XCTAssertEqual(panel.frame.height, 72, accuracy: 0.5)

        // Also cover reversing direction twice before either animation completes.
        typeQuery("a")
        typeQuery("")
        typeQuery("ab")
        XCTAssertFalse(content.isHidden)
        XCTAssertNotNil(fixedHeight())
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertFalse(content.isHidden)
        XCTAssertEqual(content.alphaValue, 1, accuracy: 0.01)
        XCTAssertEqual(panel.frame.height, 520, accuracy: 0.5)
        XCTAssertEqual(content.frame.height, 440, accuracy: 0.5)
        XCTAssertNil(fixedHeight())
    }
}
