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

enum SearchMode {
    case applications
    case clipboard

    var pluginId: String {
        switch self {
        case .applications: "io.clipkeep.applications"
        case .clipboard: "io.clipkeep.clipboard"
        }
    }

    var placeholder: String {
        switch self {
        case .applications: "Search applications…"
        case .clipboard: "Search clipboard history…"
        }
    }

    var title: String {
        switch self {
        case .applications: "Applications"
        case .clipboard: "Clipboard History"
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

struct PluginDescriptor {
    let id: String
    let name: String
    let executable: String
}

let builtInPlugins = [
    PluginDescriptor(
        id: "io.clipkeep.applications",
        name: "Applications",
        executable: "clipkeep-plugin-applications"
    ),
    PluginDescriptor(
        id: "io.clipkeep.clipboard",
        name: "Clipboard History",
        executable: "clipkeep-plugin-clipboard"
    ),
]
