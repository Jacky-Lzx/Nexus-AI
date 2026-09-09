import AppKit
import Carbon
import Foundation

struct PluginShortcut: Codable, Equatable {
    let keyCode: UInt32
    let modifiers: UInt32
    let keyLabel: String

    var displayName: String {
        var value = ""
        if modifiers & UInt32(controlKey) != 0 { value += "⌃" }
        if modifiers & UInt32(optionKey) != 0 { value += "⌥" }
        if modifiers & UInt32(shiftKey) != 0 { value += "⇧" }
        if modifiers & UInt32(cmdKey) != 0 { value += "⌘" }
        return value + keyLabel
    }

    init(keyCode: UInt32, modifiers: UInt32, keyLabel: String) {
        self.keyCode = keyCode
        self.modifiers = modifiers
        self.keyLabel = keyLabel
    }

    init?(event: NSEvent, requiresModifier: Bool = true) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        var carbonModifiers: UInt32 = 0
        if flags.contains(.control) { carbonModifiers |= UInt32(controlKey) }
        if flags.contains(.option) { carbonModifiers |= UInt32(optionKey) }
        if flags.contains(.shift) { carbonModifiers |= UInt32(shiftKey) }
        if flags.contains(.command) { carbonModifiers |= UInt32(cmdKey) }
        guard !requiresModifier || carbonModifiers != 0 else { return nil }

        keyCode = UInt32(event.keyCode)
        modifiers = carbonModifiers
        keyLabel = Self.label(for: event)
    }

    private static func label(for event: NSEvent) -> String {
        switch Int(event.keyCode) {
        case kVK_Space: return "Space"
        case kVK_Return: return "Return"
        case kVK_Tab: return "Tab"
        case kVK_Delete: return "Delete"
        case kVK_ForwardDelete: return "Forward Delete"
        case kVK_LeftArrow: return "←"
        case kVK_RightArrow: return "→"
        case kVK_UpArrow: return "↑"
        case kVK_DownArrow: return "↓"
        case kVK_Home: return "Home"
        case kVK_End: return "End"
        case kVK_PageUp: return "Page Up"
        case kVK_PageDown: return "Page Down"
        default:
            let labels: [Int: String] = [
                kVK_F1: "F1", kVK_F2: "F2", kVK_F3: "F3", kVK_F4: "F4",
                kVK_F5: "F5", kVK_F6: "F6", kVK_F7: "F7", kVK_F8: "F8",
                kVK_F9: "F9", kVK_F10: "F10", kVK_F11: "F11", kVK_F12: "F12",
                kVK_F13: "F13", kVK_F14: "F14", kVK_F15: "F15", kVK_F16: "F16",
                kVK_F17: "F17", kVK_F18: "F18", kVK_F19: "F19", kVK_F20: "F20",
            ]
            if let label = labels[Int(event.keyCode)] { return label }
            let characters = event.charactersIgnoringModifiers?
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .uppercased() ?? ""
            return characters.isEmpty ? "Key \(event.keyCode)" : characters
        }
    }
}

enum LauncherKeyAction: String, CaseIterable {
    case moveDown
    case moveUp

    var title: String {
        switch self {
        case .moveDown: return "Move Down"
        case .moveUp: return "Move Up"
        }
    }

    var defaultShortcut: PluginShortcut {
        switch self {
        case .moveDown:
            return PluginShortcut(
                keyCode: UInt32(kVK_ANSI_J),
                modifiers: UInt32(optionKey),
                keyLabel: "J"
            )
        case .moveUp:
            return PluginShortcut(
                keyCode: UInt32(kVK_ANSI_K),
                modifiers: UInt32(optionKey),
                keyLabel: "K"
            )
        }
    }

    private var storageKey: String { "launcher.keyMapping.\(rawValue)" }

    var shortcut: PluginShortcut {
        guard let data = UserDefaults.standard.data(forKey: storageKey),
              let shortcut = try? JSONDecoder().decode(PluginShortcut.self, from: data)
        else { return defaultShortcut }
        return shortcut
    }

    func save(shortcut: PluginShortcut) {
        guard let data = try? JSONEncoder().encode(shortcut) else { return }
        UserDefaults.standard.set(data, forKey: storageKey)
    }

    func matches(_ event: NSEvent) -> Bool {
        guard let pressed = PluginShortcut(event: event) else { return false }
        let configured = shortcut
        return pressed.keyCode == configured.keyCode && pressed.modifiers == configured.modifiers
    }
}

final class GlobalHotKey {
    enum Action: UInt32, CaseIterable {
        case applications = 1
        case clipboard = 2

        var pluginId: String {
            switch self {
            case .applications: return "io.clipkeep.applications"
            case .clipboard: return "io.clipkeep.clipboard"
            }
        }

        var defaultShortcut: PluginShortcut {
            switch self {
            case .applications:
                return PluginShortcut(keyCode: UInt32(kVK_Space), modifiers: UInt32(cmdKey | shiftKey), keyLabel: "Space")
            case .clipboard:
                return PluginShortcut(keyCode: UInt32(kVK_ANSI_C), modifiers: UInt32(cmdKey | shiftKey), keyLabel: "C")
            }
        }

        private var storageKey: String { "plugin.shortcut.\(pluginId)" }

        var shortcut: PluginShortcut {
            guard let data = UserDefaults.standard.data(forKey: storageKey),
                  let shortcut = try? JSONDecoder().decode(PluginShortcut.self, from: data)
            else { return defaultShortcut }
            return shortcut
        }

        func save(shortcut: PluginShortcut) {
            guard let data = try? JSONEncoder().encode(shortcut) else { return }
            UserDefaults.standard.set(data, forKey: storageKey)
        }

        init?(pluginId: String) {
            guard let action = Self.allCases.first(where: { $0.pluginId == pluginId }) else { return nil }
            self = action
        }
    }

    static let shared = GlobalHotKey()
    var handler: ((Action) -> Void)?

    private var hotKeys: [Action: EventHotKeyRef] = [:]
    private var eventHandler: EventHandlerRef?

    func configure(enabledActions: Set<Action>) -> [Action] {
        guard installHandlerIfNeeded() else { return Array(enabledActions) }
        hotKeys.values.forEach { UnregisterEventHotKey($0) }
        hotKeys.removeAll()

        return enabledActions.sorted { $0.rawValue < $1.rawValue }.filter { action in
            !register(action.shortcut, action: action)
        }
    }

    private func installHandlerIfNeeded() -> Bool {
        if eventHandler != nil { return true }
        var eventType = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let status = InstallEventHandler(
            GetApplicationEventTarget(),
            { _, event, _ in
                guard let event else { return OSStatus(eventNotHandledErr) }
                var identifier = EventHotKeyID()
                let status = GetEventParameter(
                    event,
                    EventParamName(kEventParamDirectObject),
                    EventParamType(typeEventHotKeyID),
                    nil,
                    MemoryLayout<EventHotKeyID>.size,
                    nil,
                    &identifier
                )
                guard status == noErr, let action = Action(rawValue: identifier.id) else {
                    return OSStatus(eventNotHandledErr)
                }
                DispatchQueue.main.async { GlobalHotKey.shared.handler?(action) }
                return noErr
            },
            1,
            &eventType,
            nil,
            &eventHandler
        )
        return status == noErr
    }

    private func register(_ shortcut: PluginShortcut, action: Action) -> Bool {
        let identifier = EventHotKeyID(signature: OSType(0x434B4559), id: action.rawValue) // CKEY
        var reference: EventHotKeyRef?
        let status = RegisterEventHotKey(
            shortcut.keyCode,
            shortcut.modifiers,
            identifier,
            GetApplicationEventTarget(),
            0,
            &reference
        )
        if status == noErr, let reference { hotKeys[action] = reference }
        return status == noErr
    }

    deinit {
        hotKeys.values.forEach { UnregisterEventHotKey($0) }
        if let eventHandler { RemoveEventHandler(eventHandler) }
    }
}
