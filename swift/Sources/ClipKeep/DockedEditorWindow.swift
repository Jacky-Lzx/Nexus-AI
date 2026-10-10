import AppKit
import ApplicationServices

/// Keeps only this session's Kitty window at the bottom, including when a
/// window manager applies its own initial placement or the user resizes it.
final class DockedEditorWindow {
    private let application: AXUIElement
    private let frame: NSRect
    private var timer: Timer?
    private var window: AXUIElement?

    init(processIdentifier: pid_t, frame: NSRect) {
        application = AXUIElementCreateApplication(processIdentifier)
        self.frame = frame
    }

    static func bottomFrame(in visibleFrame: NSRect) -> NSRect {
        let width = min(1200, visibleFrame.width * 0.9)
        let height = min(420, visibleFrame.height * 0.32)
        return NSRect(
            x: visibleFrame.midX - width / 2,
            y: visibleFrame.minY + 12,
            width: width,
            height: height
        )
    }

    static func accessibilityFrame(_ frame: NSRect, primaryTop: CGFloat? = nil) -> NSRect {
        let top = primaryTop ?? NSScreen.screens.first?.frame.maxY ?? 0
        return NSRect(x: frame.minX, y: top - frame.maxY, width: frame.width, height: frame.height)
    }

    func start() {
        maintainFrame()
        let timer = Timer(timeInterval: 0.25, repeats: true) { [weak self] _ in self?.maintainFrame() }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        window = nil
    }

    func focus() {
        maintainFrame()
        if let window { AXUIElementPerformAction(window, kAXRaiseAction as CFString) }
    }

    private func maintainFrame() {
        guard AXIsProcessTrusted() else { return }
        if window == nil {
            let windows = Self.attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []
            window = windows.first { (Self.attribute($0, kAXTitleAttribute) as? String) == "nvim_input — Nexus" }
        }
        guard let window else { return }
        let desired = Self.accessibilityFrame(frame)
        let current = Self.frame(of: window)
        var size = desired.size
        var position = desired.origin
        if current == nil || abs(current!.width - size.width) > 2 || abs(current!.height - size.height) > 2 {
            if let value = AXValueCreate(.cgSize, &size) {
                AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, value)
            }
        }
        if current == nil || abs(current!.minX - position.x) > 2 || abs(current!.minY - position.y) > 2 {
            if let value = AXValueCreate(.cgPoint, &position) {
                AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, value)
            }
        }
    }

    static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
        return value
    }

    static func elementAttribute(_ element: AXUIElement, _ name: String) -> AXUIElement? {
        guard let value = attribute(element, name), CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
        return (value as! AXUIElement)
    }

    static func frame(of element: AXUIElement) -> NSRect? {
        guard let positionValue = attribute(element, kAXPositionAttribute),
              let sizeValue = attribute(element, kAXSizeAttribute),
              CFGetTypeID(positionValue) == AXValueGetTypeID(), CFGetTypeID(sizeValue) == AXValueGetTypeID()
        else { return nil }
        var position = CGPoint.zero
        var size = CGSize.zero
        guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &position),
              AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else { return nil }
        return NSRect(origin: position, size: size)
    }

    deinit { timer?.invalidate() }
}
