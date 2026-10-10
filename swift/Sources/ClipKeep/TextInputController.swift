import AppKit
import ApplicationServices

/// Capture the original window and field before Kitty steals focus. Keeping
/// the application alone can paste into a different window of the same app.
struct TextInputDestination {
    let application: NSRunningApplication
    private let accessibilityApplication: AXUIElement
    private let window: AXUIElement?
    private let field: AXUIElement?
    private let selection: CFTypeRef?
    let screen: NSScreen

    init?(application: NSRunningApplication) {
        guard !application.isTerminated else { return nil }
        self.application = application
        accessibilityApplication = AXUIElementCreateApplication(application.processIdentifier)
        window = DockedEditorWindow.elementAttribute(accessibilityApplication, kAXFocusedWindowAttribute)
        field = DockedEditorWindow.elementAttribute(accessibilityApplication, kAXFocusedUIElementAttribute)
        selection = field.flatMap { DockedEditorWindow.attribute($0, kAXSelectedTextRangeAttribute) }
        let originalFrame = window.flatMap { DockedEditorWindow.frame(of: $0) }
        let appKitFrame = originalFrame.map { DockedEditorWindow.accessibilityFrame($0) }
        guard let screen = NSScreen.screens.max(by: {
            return Self.overlap($0.frame, appKitFrame) < Self.overlap($1.frame, appKitFrame)
        }), appKitFrame != nil else {
            guard let fallback = NSScreen.main ?? NSScreen.screens.first else { return nil }
            self.screen = fallback
            return
        }
        self.screen = screen
    }

    private static func overlap(_ screen: NSRect, _ window: NSRect?) -> CGFloat {
        guard let window else { return 0 }
        let intersection = screen.intersection(window)
        return intersection.isNull ? 0 : intersection.width * intersection.height
    }

    func restore(paste: Bool, completion: @escaping (Bool) -> Void) {
        guard !application.isTerminated else { completion(false); return }
        application.activate()
        if let window { AXUIElementPerformAction(window, kAXRaiseAction as CFString) }
        restoreWhenActive(paste: paste, attemptsRemaining: 30, completion: completion)
    }

    private func restoreWhenActive(paste: Bool, attemptsRemaining: Int, completion: @escaping (Bool) -> Void) {
        guard !application.isTerminated, attemptsRemaining > 0 else { completion(false); return }
        let isActive = NSWorkspace.shared.frontmostApplication?.processIdentifier == application.processIdentifier
        let focusedWindow = DockedEditorWindow.elementAttribute(accessibilityApplication, kAXFocusedWindowAttribute)
        let correctWindow = window.map { expected in focusedWindow.map { CFEqual(expected, $0) } ?? false } ?? true
        if isActive, correctWindow {
            if let field {
                AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
                if let selection { AXUIElementSetAttributeValue(field, kAXSelectedTextRangeAttribute as CFString, selection) }
            }
            if paste {
                guard AXIsProcessTrusted(),
                      let source = CGEventSource(stateID: .combinedSessionState),
                      let down = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: true),
                      let up = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: false)
                else { completion(false); return }
                down.flags = .maskCommand
                up.flags = .maskCommand
                down.postToPid(application.processIdentifier)
                up.postToPid(application.processIdentifier)
            }
            completion(true)
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            restoreWhenActive(paste: paste, attemptsRemaining: attemptsRemaining - 1, completion: completion)
        }
    }
}

final class TextInputController {
    private let editor = ExternalTextEditor()
    private(set) var isEditing = false

    func focusEditor() { editor.focusComposition() }

    func start(destination: TextInputDestination, configuration: [String: String]) {
        guard !isEditing else { focusEditor(); return }
        guard AXIsProcessTrusted() else {
            let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
            AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
            notify("Enable Nexus in System Settings → Privacy & Security → Accessibility, then try again.")
            return
        }
        isEditing = true
        NSApp.deactivate()
        editor.compose(
            bottomFrame: DockedEditorWindow.bottomFrame(in: destination.screen.visibleFrame),
            configuration: ExternalTextEditor.Configuration(
                terminalPath: configuration["terminal_path"] ?? "",
                editorPath: configuration["editor_path"] ?? ""
            )
        ) { [weak self] result in
            guard let self else { return }
            switch result {
            case let .failure(error):
                isEditing = false
                destination.restore(paste: false) { _ in }
                notify(error.localizedDescription)
            case let .success(text):
                var shouldPaste = false
                if let text {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    let copied = pasteboard.setString(text, forType: .string)
                    shouldPaste = copied && !text.isEmpty
                    if !copied { notify("Could not copy the text to the clipboard.") }
                }
                destination.restore(paste: shouldPaste) { [weak self] restored in
                    self?.isEditing = false
                    if shouldPaste && !restored {
                        self?.notify("Text copied. The original window is unavailable; paste it manually.")
                    }
                }
            }
        }
    }

    private func notify(_ message: String) {
        NotificationPresenter.shared.deliver(PluginNotification(id: UUID().uuidString, title: "Write in Neovim", body: message))
    }
}
