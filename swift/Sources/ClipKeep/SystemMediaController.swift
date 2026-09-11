import AppKit

enum SystemMediaController {
    struct Command {
        let keyCode: Int
        let confirmation: String
    }

    static func command(for commandId: String) -> Command? {
        switch commandId {
        case "mute":
            Command(keyCode: 7, confirmation: "Mute toggled")
        case "volume-up":
            Command(keyCode: 0, confirmation: "Volume increased")
        case "volume-down":
            Command(keyCode: 1, confirmation: "Volume decreased")
        case "play-pause":
            Command(keyCode: 16, confirmation: "Play/pause sent")
        case "next-track":
            Command(keyCode: 17, confirmation: "Next track sent")
        case "previous-track":
            Command(keyCode: 18, confirmation: "Previous track sent")
        default:
            nil
        }
    }

    static func perform(_ command: Command) -> Bool {
        guard CGPreflightPostEventAccess() || CGRequestPostEventAccess() else { return false }
        return post(command.keyCode, state: 0xA) && post(command.keyCode, state: 0xB)
    }

    private static func post(_ keyCode: Int, state: Int) -> Bool {
        let stateBits = state << 8
        guard let event = NSEvent.otherEvent(
            with: .systemDefined,
            location: .zero,
            modifierFlags: NSEvent.ModifierFlags(rawValue: UInt(stateBits)),
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: 0,
            context: nil,
            subtype: 8,
            data1: (keyCode << 16) | stateBits,
            data2: -1
        ), let cgEvent = event.cgEvent else { return false }
        cgEvent.post(tap: .cghidEventTap)
        return true
    }
}
