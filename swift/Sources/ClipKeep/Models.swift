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
    case plugins
    case clipboard
    case plugin(PluginDescriptor)

    var placeholder: String {
        switch self {
        case .global: "Search Nexus…"
        case .plugins: "Search plugins…"
        case .clipboard: "Search clipboard history…"
        case let .plugin(plugin): "Search \(plugin.name)…"
        }
    }

    var title: String {
        switch self {
        case .global: "Nexus"
        case .plugins: "Plugins"
        case .clipboard: "Clipboard History"
        case let .plugin(plugin): plugin.name
        }
    }

    var showsDetails: Bool { self == .clipboard }
    var expandsWhenEmpty: Bool {
        switch self {
        case .global: false
        case .plugins, .clipboard, .plugin: true
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
    let inputFields: [ActionInputField]

    enum CodingKeys: String, CodingKey {
        case id, title, destructive, `default`
        case closesWindow = "closes_window"
        case pasteToPrevious = "paste_to_previous"
        case inputFields = "input_fields"
    }


    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        destructive = try values.decodeIfPresent(Bool.self, forKey: .destructive) ?? false
        `default` = try values.decodeIfPresent(Bool.self, forKey: .default) ?? false
        closesWindow = try values.decodeIfPresent(Bool.self, forKey: .closesWindow) ?? false
        pasteToPrevious = try values.decodeIfPresent(Bool.self, forKey: .pasteToPrevious) ?? false
        inputFields = try values.decodeIfPresent([ActionInputField].self, forKey: .inputFields) ?? []
    }
}

enum ActionInputKind: String, Codable {
    case text
    case integer
}

struct ActionInputField: Codable {
    let id: String
    let label: String
    let placeholder: String
    let kind: ActionInputKind
    let required: Bool
    let minimum: Int?
    let maximum: Int?

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        label = try values.decode(String.self, forKey: .label)
        placeholder = try values.decodeIfPresent(String.self, forKey: .placeholder) ?? label
        kind = try values.decodeIfPresent(ActionInputKind.self, forKey: .kind) ?? .text
        required = try values.decodeIfPresent(Bool.self, forKey: .required) ?? false
        minimum = try values.decodeIfPresent(Int.self, forKey: .minimum)
        maximum = try values.decodeIfPresent(Int.self, forKey: .maximum)
    }
}

enum ClipboardPreference {
    static let retentionDays = "clipboard.retentionDays"
    static let ignoreConcealed = "clipboard.ignoreConcealed"
    static let ignoredBundleIds = "clipboard.ignoredBundleIds"
    static let terminalExecutable = "clipboard.terminalExecutable"
    static let editorExecutable = "clipboard.editorExecutable"

    static func registerDefaults() {
        UserDefaults.standard.register(defaults: [
            retentionDays: 30,
            ignoreConcealed: true,
            ignoredBundleIds: "com.1password.1password,com.agilebits.onepassword7",
            terminalExecutable: "",
            editorExecutable: "",
        ])
    }

    static var terminalExecutablePath: String {
        UserDefaults.standard.string(forKey: terminalExecutable) ?? ""
    }

    static var editorExecutablePath: String {
        UserDefaults.standard.string(forKey: editorExecutable) ?? ""
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

enum ClashPreference {
    static let controllerURL = "clash.controllerURL"
    static let secret = "clash.secret"
    static let unixSocket = "clash.unixSocket"

    static func registerDefaults() {
        UserDefaults.standard.register(defaults: [
            controllerURL: "http://127.0.0.1:9090",
            secret: "",
            unixSocket: "/tmp/verge/verge-mihomo.sock",
        ])
    }

    static var pluginSettings: [String: String] {
        [
            "controller_url": UserDefaults.standard.string(forKey: controllerURL)
                ?? "http://127.0.0.1:9090",
            "secret": UserDefaults.standard.string(forKey: secret) ?? "",
            "unix_socket": UserDefaults.standard.string(forKey: unixSocket)
                ?? "/tmp/verge/verge-mihomo.sock",
        ]
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
    var metadata: [String: String]
    let actions: [ResultAction]

    enum CodingKeys: String, CodingKey {
        case pluginId = "plugin_id"
        case id, title, subtitle, preview, kind, score, metadata, actions
    }
}

extension SearchResult {
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        pluginId = try values.decode(String.self, forKey: .pluginId)
        id = try values.decode(String.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        kind = try values.decode(ResultKind.self, forKey: .kind)
        subtitle = try values.decodeIfPresent(String.self, forKey: .subtitle) ?? ""
        preview = try values.decodeIfPresent(String.self, forKey: .preview) ?? ""
        score = try values.decodeIfPresent(Double.self, forKey: .score) ?? 0
        metadata = try values.decodeIfPresent([String: String].self, forKey: .metadata) ?? [:]
        actions = try values.decodeIfPresent([ResultAction].self, forKey: .actions) ?? []
    }
}

struct PluginConfigurationItem: Codable {
    let id: String
    let title: String
    let subtitle: String
    let metadata: [String: String]
}

struct PluginNotification: Codable {
    let id: String
    let title: String
    let body: String
}

enum PluginPresentationStyle: String, Codable {
    case info
    case success
    case warning
}

struct PluginPresentationItem: Codable {
    let id: String
    let title: String
    let value: String
    let style: PluginPresentationStyle
    let metadata: [String: String]

    enum CodingKeys: String, CodingKey {
        case id, title, value, style, metadata
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        title = try values.decode(String.self, forKey: .title)
        value = try values.decodeIfPresent(String.self, forKey: .value) ?? ""
        style = try values.decodeIfPresent(PluginPresentationStyle.self, forKey: .style) ?? .info
        metadata = try values.decodeIfPresent([String: String].self, forKey: .metadata) ?? [:]
    }
}

enum PluginItemPreference {
    static func hiddenItemIds(for pluginId: String) -> Set<String> {
        Set(UserDefaults.standard.stringArray(forKey: key(for: pluginId)) ?? [])
    }

    static func setVisible(_ visible: Bool, itemId: String, pluginId: String) {
        var hidden = hiddenItemIds(for: pluginId)
        if visible {
            hidden.remove(itemId)
        } else {
            hidden.insert(itemId)
        }
        UserDefaults.standard.set(hidden.sorted(), forKey: key(for: pluginId))
    }

    static func pluginSettings(for pluginId: String) -> [String: String] {
        let items = hiddenItemIds(for: pluginId).sorted()
        guard let data = try? JSONSerialization.data(withJSONObject: items),
              let json = String(data: data, encoding: .utf8) else { return [:] }
        return ["hidden_items": json]
    }

    private static func key(for pluginId: String) -> String {
        "plugin.hiddenItems.\(pluginId)"
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

    var bundleURL: URL? = nil
    var revision: String = ""
    var iconName: String? = nil
    var summary: String? = nil

    var symbolName: String {
        if let iconName { return iconName }
        return switch id {
        case "io.clipkeep.clipboard": "clipboard"
        case "io.clipkeep.applications": "square.grid.2x2"
        case "io.clipkeep.caffeinate": "cup.and.saucer.fill"
        case "io.clipkeep.calculator": "function"
        case "io.clipkeep.calendar": "calendar"
        case "io.clipkeep.clash": "network"
        case "io.clipkeep.files": "doc.text.magnifyingglass"
        case "io.clipkeep.kill-process": "xmark.circle"
        case "io.clipkeep.menu-bar": "menubar.rectangle"
        case "io.clipkeep.reminders": "bell.badge"
        case "io.clipkeep.system-commands": "power"
        case "io.clipkeep.system-status": "gauge.with.dots.needle.67percent"
        case "io.clipkeep.text-tools": "textformat"
        case "io.clipkeep.time-date": "clock"
        case "io.clipkeep.unit-converter": "ruler"
        case "io.clipkeep.web-search": "globe"
        default: "puzzlepiece.extension"
        }
    }
}
