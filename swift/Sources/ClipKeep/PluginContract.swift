import Foundation

enum PluginContract {
    static let features: Set<String> = ["search.cancel", "action.arguments", "configuration.values", "host_action.object", "status.snapshots"]
    static let services: [String: Int] = ["system.media": 1, "text.edit": 1, "text.compose": 1, "notifications.authorize": 1, "menu.press": 1]
    static var initializeRequest: [String: Any] {
        ["type": "initialize", "protocol_version": 1, "host_features": features.sorted(), "host_services": services]
    }

    static func validate(requiredFeatures: [String], requiredServices: [String: Int], schema: [PluginConfigurationField]) throws {
        let missing = Set(requiredFeatures).subtracting(features)
        guard missing.isEmpty else {
            throw PluginInstallationError.invalid("Unsupported required plugin features: \(missing.sorted().joined(separator: ", "))")
        }
        for (service, version) in requiredServices where services[service] != version {
            throw PluginInstallationError.invalid("Unsupported host service: \(service) version \(version)")
        }
        guard schema.count <= 64, Set(schema.map(\.id)).count == schema.count else {
            throw PluginInstallationError.invalid("Invalid or duplicate plugin settings")
        }
        for field in schema {
            guard !field.id.isEmpty, field.id.utf8.count <= 128,
                  field.label.utf8.count <= 1024, (field.options?.count ?? 0) <= 256,
                  field.parse(field.default.legacyString) == field.default else {
                throw PluginInstallationError.invalid("Unsupported plugin setting or invalid default: \(field.id)")
            }
        }
    }
}
