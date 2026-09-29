import Foundation

indirect enum PluginValue: Codable, Equatable {
    case string(String), integer(Int64), number(Double), boolean(Bool), array([PluginValue]), object([String: PluginValue]), null

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let v = try? value.decode(Bool.self) { self = .boolean(v) }
        else if let v = try? value.decode(Int64.self) { self = .integer(v) }
        else if let v = try? value.decode(Double.self) { self = .number(v) }
        else if let v = try? value.decode(String.self) { self = .string(v) }
        else if let v = try? value.decode([PluginValue].self) { self = .array(v) }
        else { self = .object(try value.decode([String: PluginValue].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .string(let v): try value.encode(v)
        case .integer(let v): try value.encode(v)
        case .number(let v): try value.encode(v)
        case .boolean(let v): try value.encode(v)
        case .array(let v): try value.encode(v)
        case .object(let v): try value.encode(v)
        case .null: try value.encodeNil()
        }
    }
    var legacyString: String {
        if case .string(let text) = self { return text }
        return (try? JSONEncoder().encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? "null"
    }
    var foundationValue: Any {
        switch self {
        case .string(let v): return v
        case .integer(let v): return v
        case .number(let v): return v
        case .boolean(let v): return v
        case .array(let v): return v.map(\.foundationValue)
        case .object(let v): return v.mapValues(\.foundationValue)
        case .null: return NSNull()
        }
    }
}

struct HostServiceCall: Codable {
    let service: String
    var parameters: [String: PluginValue] = [:]
    enum CodingKeys: String, CodingKey { case service, parameters }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        service = try values.decode(String.self, forKey: .service)
        parameters = try values.decodeIfPresent([String: PluginValue].self, forKey: .parameters) ?? [:]
    }
}

struct PluginConfigurationField: Codable {
    let id: String
    let label: String
    let kind: String
    let `default`: PluginValue
    var description: String?
    var options: [String]?
    var minimum: Int64?
    var maximum: Int64?

    func parse(_ text: String) -> PluginValue? {
        switch kind {
        case "text": return .string(text)
        case "boolean":
            if text == "true" { return .boolean(true) }
            if text == "false" { return .boolean(false) }
            return nil
        case "choice": return (options ?? []).contains(text) ? .string(text) : nil
        case "integer":
            guard let number = Int64(text), minimum.map({ number >= $0 }) ?? true,
                  maximum.map({ number <= $0 }) ?? true else { return nil }
            return .integer(number)
        default: return nil
        }
    }
}

enum PluginConfigurationPreference {
    static func settings(for id: String) -> [String: String] {
        UserDefaults.standard.dictionary(forKey: "plugin.configuration.\(id)") as? [String: String] ?? [:]
    }
    static func save(_ settings: [String: String], for id: String) {
        UserDefaults.standard.set(settings, forKey: "plugin.configuration.\(id)")
    }
}

extension SearchResult {
    func hostParameter(_ name: String, actionId: String) -> String? {
        if let call = actions.first(where: { $0.id == actionId })?.hostAction {
            return call.parameters[name]?.legacyString
        }
        return metadata[name]
    }
}
