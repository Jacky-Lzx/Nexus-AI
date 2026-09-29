import Foundation

/// Optional protocol-v1 metadata: host_action.<action-id> selects a declared host service.
/// Unrecognized or undeclared services must fail instead of executing a different action.
enum PluginHostAction: String {
    case media = "system.media"
    case editText = "text.edit"
    case notificationPermission = "notifications.authorize"
    case menuPress = "menu.press"

    var capability: String {
        switch self {
        case .media: "system.control"
        case .editText: "clipboard.write"
        case .notificationPermission: "notifications"
        case .menuPress: "accessibility.press"
        }
    }

    static func resolve(result: SearchResult, actionId: String, descriptor: PluginDescriptor) throws -> Self? {
        let declared = result.actions.first(where: { $0.id == actionId })?.hostAction?.service
            ?? result.metadata["host_action.\(actionId)"]
        // Compatibility with already distributed v1 bundles. New plugins use metadata only.
        let legacy: Self?
        switch (descriptor.id, actionId) {
        case ("io.clipkeep.clipboard", "edit"): legacy = .editText
        case ("io.clipkeep.menu-bar", "press"): legacy = .menuPress
        case ("io.clipkeep.system-commands", "run")
            where SystemMediaController.command(for: result.metadata["command"] ?? "") != nil:
            legacy = .media
        default: legacy = nil
        }
        guard declared != nil || legacy != nil else { return nil }
        guard let service = declared.flatMap(Self.init(rawValue:)) ?? (declared == nil ? legacy : nil),
              descriptor.capabilities.contains(service.capability) else {
            throw PluginInstallationError.invalid("Plugin requested an unsupported or undeclared host action")
        }
        return service
    }
}
