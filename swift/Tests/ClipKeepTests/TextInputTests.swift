import AppKit
import XCTest
@testable import Nexus

final class TextInputTests: XCTestCase {
    private func runEditor(_ commands: [String]) throws -> (saved: Bool, text: String) {
        let editor = ["/opt/homebrew/bin/nvim", "/usr/local/bin/nvim"].first {
            FileManager.default.isExecutableFile(atPath: $0)
        }
        guard let editor else { throw XCTSkip("Neovim is required for editor lifecycle integration tests") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nexus-input-test-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("clipboard.txt")
        let script = directory.appendingPathComponent("session.lua")
        let saved = directory.appendingPathComponent("saved")
        try "".write(to: file, atomically: false, encoding: .utf8)
        try ExternalTextEditor.compositionScript.write(to: script, atomically: false, encoding: .utf8)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: editor)
        process.arguments = ["--headless", "-u", "NONE", "-i", "NONE", "-n", file.path,
                             "-c", "setlocal noendofline nofixeol nomodified",
                             "-c", "autocmd BufWritePre <buffer> setlocal noendofline nofixeol",
                             "-c", "lua dofile(vim.env.NEXUS_EDITOR_SESSION_SCRIPT)"]
            + commands.flatMap { ["-c", $0] }
        var environment = ProcessInfo.processInfo.environment
        environment["NEXUS_EDITOR_SESSION_SCRIPT"] = script.path
        environment["NEXUS_EDITOR_SAVED_PATH"] = saved.path
        environment["NVIM_LOG_FILE"] = directory.appendingPathComponent("nvim.log").path
        process.environment = environment
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        return (FileManager.default.fileExists(atPath: saved.path), try String(contentsOf: file, encoding: .utf8))
    }

    func testSavedUnicodeAndMultilineTextKeepsExactBytes() throws {
        let result = try runEditor(["lua vim.api.nvim_buf_set_lines(0, 0, -1, false, {'中文输入', 'second line', ''})", "wq"])
        XCTAssertTrue(result.saved)
        XCTAssertEqual(result.text, "中文输入\nsecond line\n")
        XCTAssertEqual(ExternalTextEditor.compositionText(result.text), "中文输入\nsecond line")
        let singleLine = try runEditor(["lua vim.api.nvim_buf_set_lines(0, 0, -1, false, {'hello'})", "wq"])
        XCTAssertTrue(singleLine.saved)
        XCTAssertEqual(singleLine.text, "hello")
    }

    func testDiscardingUnsavedTextDoesNotSubmit() throws {
        let result = try runEditor(["lua vim.api.nvim_buf_set_lines(0, 0, -1, false, {'discard me'})", "q!"])
        XCTAssertFalse(result.saved)
        XCTAssertEqual(result.text, "")
    }

    func testDiscardingChangesAfterAWriteDoesNotSubmitOldText() throws {
        let result = try runEditor(["lua vim.api.nvim_buf_set_lines(0, 0, -1, false, {'saved'})", "write",
                                    "lua vim.api.nvim_buf_set_lines(0, 0, -1, false, {'discard'})", "q!"])
        XCTAssertFalse(result.saved)
        XCTAssertEqual(result.text, "saved")
    }

    func testErrorExitAndQuitWithoutWritingCancel() throws {
        XCTAssertFalse(try runEditor(["q!"]).saved)
        XCTAssertFalse(try runEditor(["write", "cq"]).saved)
    }

    func testSavingToAnotherFileDoesNotSubmitOriginalBuffer() throws {
        let result = try runEditor(["execute 'saveas ' . fnameescape(expand('%:p:h') . '/other.txt')", "q"])
        XCTAssertFalse(result.saved)
    }

    func testBottomPlacementAndCoordinatesOnOtherDisplays() {
        let visible = NSRect(x: -1920, y: 100, width: 1920, height: 1040)
        let frame = DockedEditorWindow.bottomFrame(in: visible)
        XCTAssertEqual(frame.midX, visible.midX)
        XCTAssertEqual(frame.minY, visible.minY + 12)
        XCTAssertTrue(visible.contains(frame))
        let axFrame = DockedEditorWindow.accessibilityFrame(frame, primaryTop: 1080)
        XCTAssertEqual(axFrame.minX, frame.minX)
        XCTAssertEqual(axFrame.minY, 1080 - frame.maxY)
        XCTAssertEqual(DockedEditorWindow.accessibilityFrame(axFrame, primaryTop: 1080), frame)
    }

    func testCompositionRemovesOnlyOneFinalLineBreak() {
        XCTAssertEqual(ExternalTextEditor.compositionText("hello\n"), "hello")
        XCTAssertEqual(ExternalTextEditor.compositionText("hello\r\n"), "hello")
        XCTAssertEqual(ExternalTextEditor.compositionText("first\nsecond\n\n"), "first\nsecond\n")
        XCTAssertEqual(ExternalTextEditor.compositionText("hello  \n"), "hello  ")
        XCTAssertEqual(ExternalTextEditor.compositionText("hello"), "hello")
        XCTAssertEqual(ExternalTextEditor.compositionText(""), "")
    }
}
