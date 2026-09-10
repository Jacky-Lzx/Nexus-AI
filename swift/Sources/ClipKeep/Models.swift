import Foundation

enum ImagePreviewQuality: String, CaseIterable {
    case highDefinition = "high_definition"
    case original

    var title: String {
        switch self {
        case .highDefinition: "High Definition"
        case .original: "Original"
        }
    }
}

enum SearchMode: Equatable {
    case global
    case clipboard
    case plugin(PluginDescriptor)

    var placeholder: String {
        switch self {
        case .global: "Search Nexus…"
        case .clipboard: "Search clipboard history…"
        case let .plugin(plugin): "Search \(plugin.name)…"
        }
    }

    var title: String {
        switch self {
        case .global: "Nexus"
        case .clipboard: "Clipboard History"
        case let .plugin(plugin): plugin.name
        }
    }

    var showsDetails: Bool { self == .clipboard }
    var expandsWhenEmpty: Bool {
        switch self {
        case .global: false
        case .clipboard, .plugin: true
        }
    }
}

enum ResultKind: String, Codable {
    case application
    case clipboardText = "clipboard_text"
    case clipboardRichText = "clipboard_rich_text"
    case clipboardImage = "clipboard_image"
    case clipboardFiles = "clipboard_files"
    case command
    case file
    case other
}

extension ResultKind {
    var isClipboard: Bool {
        switch self {
        case .clipboardText, .clipboardRichText, .clipboardImage, .clipboardFiles: true
        default: false
        }
    }
}

struct ResultAction: Codable {
    let id: String
    let title: String
    let destructive: Bool
    let `default`: Bool
    let closesWindow: Bool
    let pasteToPrevious: Bool

    enum CodingKeys: String, CodingKey {
        case id, title, destructive, `default`
        case closesWindow = "closes_window"
        case pasteToPrevious = "paste_to_previous"
    }


    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        destructive = try values.decodeIfPresent(Bool.self, forKey: .destructive) ?? false
        `default` = try values.decodeIfPresent(Bool.self, forKey: .default) ?? false
        closesWindow = try values.decodeIfPresent(Bool.self, forKey: .closesWindow) ?? false
        pasteToPrevious = try values.decodeIfPresent(Bool.self, forKey: .pasteToPrevious) ?? false
    }
}

enum ClipboardPreference {
    static let retentionDays = "clipboard.retentionDays"
    static let ignoreConcealed = "clipboard.ignoreConcealed"
    static let ignoredBundleIds = "clipboard.ignoredBundleIds"

    static func registerDefaults() {
        UserDefaults.standard.register(defaults: [
            retentionDays: 30,
            ignoreConcealed: true,
            ignoredBundleIds: "com.1password.1password,com.agilebits.onepassword7",
        ])
    }

    static var pluginSettings: [String: String] {
        [
            "retention_days": String(UserDefaults.standard.integer(forKey: retentionDays)),
            "ignore_concealed": String(UserDefaults.standard.bool(forKey: ignoreConcealed)),
            "ignored_bundle_ids": UserDefaults.standard.string(forKey: ignoredBundleIds) ?? "",
        ]
    }
}

enum CalendarPreference {
    static let futureDays = "calendar.futureDays"

    static func registerDefaults() {
        UserDefaults.standard.register(defaults: [futureDays: 14])
    }

    static var pluginSettings: [String: String] {
        ["future_days": String(UserDefaults.standard.integer(forKey: futureDays))]
    }
}

struct SearchResult: Codable {
    let pluginId: String
    let id: String
    let title: String
    let subtitle: String
    let preview: String
    let kind: ResultKind
    let score: Double
    let metadata: [String: String]
    let actions: [ResultAction]

    enum CodingKeys: String, CodingKey {
        case pluginId = "plugin_id"
        case id, title, subtitle, preview, kind, score, metadata, actions
    }
}

struct PluginDescriptor: Equatable {
    let id: String
    let name: String
    let version: String
    let protocolVersion: Int
    let executable: String
    let executableURL: URL
    let defaultEnabled: Bool
    let defaultIncludedInGlobalSearch: Bool
    let capabilities: [String]

    var symbolName: String {
        switch id {
        case "io.clipkeep.clipboard": "clipboard"
        case "io.clipkeep.applications": "square.grid.2x2"
        case "io.clipkeep.calculator": "function"
        case "io.clipkeep.calendar": "calendar"
        case "io.clipkeep.system-commands": "power"
        default: "puzzlepiece.extension"
        }
    }
}

private let fallbackBuiltInPlugins = [
    PluginDescriptor(
        id: "io.clipkeep.applications",
        name: "Applications",
        version: "0.1.0",
        protocolVersion: 1,
        executable: "clipkeep-plugin-applications",
        executableURL: Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/clipkeep-plugin-applications"),
        defaultEnabled: true,
        defaultIncludedInGlobalSearch: true,
        capabilities: ["applications.read", "applications.open", "background"]
    ),
    PluginDescriptor(
        id: "io.clipkeep.clipboard",
        name: "Clipboard History",
        version: "0.1.0",
        protocolVersion: 1,
        executable: "clipkeep-plugin-clipboard",
        executableURL: Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/clipkeep-plugin-clipboard"),
        defaultEnabled: true,
        defaultIncludedInGlobalSearch: false,
        capabilities: ["clipboard.read", "clipboard.write", "storage", "background"]
    ),
    PluginDescriptor(
        id: "io.clipkeep.calculator", name: "Calculator", version: "0.1.0", protocolVersion: 1,
        executable: "clipkeep-plugin-calculator",
        executableURL: Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/clipkeep-plugin-calculator"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true,
        capabilities: ["calculate", "clipboard.write"]
    ),
    PluginDescriptor(
        id: "io.clipkeep.calendar", name: "Upcoming Events", version: "0.1.0", protocolVersion: 1,
        executable: "clipkeep-plugin-calendar",
        executableURL: Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/clipkeep-plugin-calendar"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: false,
        capabilities: ["calendar.read", "applications.open"]
    ),
    PluginDescriptor(
        id: "io.clipkeep.system-commands", name: "System Commands", version: "0.1.0", protocolVersion: 1,
        executable: "clipkeep-plugin-system-commands",
        executableURL: Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/clipkeep-plugin-system-commands"),
        defaultEnabled: true, defaultIncludedInGlobalSearch: true,
        capabilities: ["system.control", "applications.open"]
    ),
]

enum PluginRegistry {
    static func discover() -> [PluginDescriptor] {
        var roots: [URL] = []
        roots.append(Bundle.main.bundleURL.appendingPathComponent("Contents/PlugIns", isDirectory: true))
        if let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first {
            roots.append(support.appendingPathComponent("io.clipkeep.ClipKeep/Plugins", isDirectory: true))
        }
        var descriptors: [PluginDescriptor] = []
        for root in roots {
            let children = (try? FileManager.default.contentsOfDirectory(
                at: root,
                includingPropertiesForKeys: [.isDirectoryKey],
                options: [.skipsHiddenFiles]
            )) ?? []
            for directory in children.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
                let flatManifest = directory.appendingPathComponent("plugin.toml")
                let bundledManifest = directory.appendingPathComponent("Contents/Resources/plugin.toml")
                let manifest = FileManager.default.fileExists(atPath: bundledManifest.path)
                    ? bundledManifest : flatManifest
                let executableDirectory = FileManager.default.fileExists(
                    atPath: directory.appendingPathComponent("Contents/MacOS").path
                ) ? directory.appendingPathComponent("Contents/MacOS") : directory
                if let descriptor = parseManifest(at: manifest, directory: executableDirectory) {
                    if !descriptors.contains(where: { $0.id == descriptor.id }) {
                        descriptors.append(descriptor)
                    }
                }
            }
        }
        return descriptors.isEmpty ? fallbackBuiltInPlugins : descriptors.sorted { $0.name < $1.name }
    }
    private static func parseManifest(at url: URL, directory: URL) -> PluginDescriptor? {
        guard let source = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        var values: [String: String] = [:]
        for rawLine in source.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = rawLine.split(separator: "#", maxSplits: 1).first?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !line.isEmpty, !line.hasPrefix("["), let separator = line.firstIndex(of: "=") else { continue }
            let key = line[..<separator].trimmingCharacters(in: .whitespaces)
            let value = line[line.index(after: separator)...].trimmingCharacters(in: .whitespaces)
            values[key] = value
        }
        func string(_ key: String) -> String? {
            guard let raw = values[key], raw.hasPrefix("\""), raw.hasSuffix("\"") else { return nil }
            return String(raw.dropFirst().dropLast())
        }
        func strings(_ key: String) -> [String] {
            guard let raw = values[key], raw.hasPrefix("["), raw.hasSuffix("]") else { return [] }
            return raw.dropFirst().dropLast().split(separator: ",").compactMap { element in
                let value = element.trimmingCharacters(in: .whitespaces)
                guard value.hasPrefix("\""), value.hasSuffix("\"") else { return nil }
                return String(value.dropFirst().dropLast())
            }
        }
        guard let id = string("id"), id.range(of: #"^[A-Za-z0-9][A-Za-z0-9._-]+$"#, options: .regularExpression) != nil,
              let name = string("name"), let version = string("version"),
              let entrypoint = string("entrypoint"), !entrypoint.contains("/") && !entrypoint.contains(".."),
              let protocolVersion = values["protocol_version"].flatMap(Int.init), protocolVersion == 1
        else { return nil }
        return PluginDescriptor(
            id: id,
            name: name,
            version: version,
            protocolVersion: protocolVersion,
            executable: entrypoint,
            executableURL: directory.appendingPathComponent(entrypoint),
            defaultEnabled: values["default_enabled"] != "false",
            defaultIncludedInGlobalSearch: values["include_in_global_search"] == "true",
            capabilities: strings("capabilities")
        )
    }
}
