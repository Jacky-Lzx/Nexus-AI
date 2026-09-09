import Carbon
import Foundation

final class GlobalHotKey {
    enum Action: UInt32 {
        case applications = 1
        case clipboard = 2
    }

    static let shared = GlobalHotKey()
    var handler: ((Action) -> Void)?

    private var hotKeys: [EventHotKeyRef] = []
    private var eventHandler: EventHandlerRef?

    func register() -> Bool {
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )
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
        guard status == noErr else { return false }
        return register(keyCode: UInt32(kVK_Space), action: .applications)
            && register(keyCode: UInt32(kVK_ANSI_C), action: .clipboard)
    }

    private func register(keyCode: UInt32, action: Action) -> Bool {
        let signature = OSType(0x434B4559) // CKEY
        let identifier = EventHotKeyID(signature: signature, id: action.rawValue)
        var reference: EventHotKeyRef?
        let status = RegisterEventHotKey(
            keyCode, UInt32(cmdKey | shiftKey), identifier,
            GetApplicationEventTarget(), 0, &reference
        )
        if let reference { hotKeys.append(reference) }
        return status == noErr
    }

    deinit {
        hotKeys.forEach { UnregisterEventHotKey($0) }
        if let eventHandler { RemoveEventHandler(eventHandler) }
    }
}
