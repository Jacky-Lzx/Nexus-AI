import Foundation

struct PluginDiscovery {
    let descriptors: [PluginDescriptor]
    let errors: [String]
}

enum PluginRegistry {
    static var userDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("io.clipkeep.ClipKeep/Plugins", isDirectory: true)
    }

    static var roots: [URL] {
        [Bundle.main.bundleURL.appendingPathComponent("Contents/PlugIns", isDirectory: true), userDirectory]
    }

    static func discover() -> [PluginDescriptor] { scan().descriptors }

    /// Later roots override earlier roots, so user-installed updates win over bundled versions.
    static func scan(roots: [URL] = roots) -> PluginDiscovery {
        var plugins: [String: PluginDescriptor] = [:]
        var errors: [String] = []
        for root in roots {
            let children: [URL]
            do {
                children = try FileManager.default.contentsOfDirectory(
                    at: root, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
                )
            } catch {
                if FileManager.default.fileExists(atPath: root.path) {
                    errors.append("Could not read \(root.lastPathComponent): \(error.localizedDescription)")
                }
                continue
            }
            var seen: Set<String> = []
            for bundle in children.sorted(by: { $0.lastPathComponent < $1.lastPathComponent })
                where bundle.pathExtension == "clipkeepplugin" {
                do {
                    let plugin = try load(bundle)
                    guard seen.insert(plugin.id).inserted else {
                        errors.append("Duplicate plugin ID: \(plugin.id)")
                        continue
                    }
                    plugins[plugin.id] = plugin
                } catch {
                    errors.append("\(bundle.lastPathComponent): \(error.localizedDescription)")
                }
            }
        }
        return PluginDiscovery(descriptors: plugins.values.sorted {
            $0.name == $1.name ? $0.id < $1.id : $0.name < $1.name
        }, errors: errors)
    }

    static func load(_ bundle: URL) throws -> PluginDescriptor {
        let bundle = bundle.standardizedFileURL
        let bundledManifest = bundle.appendingPathComponent("Contents/Resources/plugin.toml")
        let structured = FileManager.default.fileExists(atPath: bundledManifest.path)
        let manifest = structured ? bundledManifest : bundle.appendingPathComponent("plugin.toml")
        let directory = structured ? bundle.appendingPathComponent("Contents/MacOS") : bundle
        guard bundle.pathExtension == "clipkeepplugin",
              var plugin = parseManifest(at: manifest, directory: directory) else {
            throw PluginInstallationError.invalid("Invalid plugin.toml or unsupported protocol version")
        }
        let resolvedRoot = bundle.resolvingSymlinksInPath().path + "/"
        guard manifest.resolvingSymlinksInPath().path.hasPrefix(resolvedRoot),
              plugin.executableURL.resolvingSymlinksInPath().path.hasPrefix(resolvedRoot),
              FileManager.default.isExecutableFile(atPath: plugin.executableURL.path),
              (try plugin.executableURL.resourceValues(forKeys: [.isRegularFileKey])).isRegularFile == true
        else { throw PluginInstallationError.invalid("Entrypoint must be an executable file inside the plugin") }
        let attributes = try FileManager.default.attributesOfItem(atPath: plugin.executableURL.path)
        let source = try String(contentsOf: manifest, encoding: .utf8)
        plugin.bundleURL = bundle
        plugin.revision = "\(source)|\((attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0)|\(attributes[.size] ?? "")|\(attributes[.systemFileNumber] ?? "")"
        return plugin
    }

    private static func parseManifest(at url: URL, directory: URL) -> PluginDescriptor? {
        guard let source = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        var values: [String: String] = [:]
        for rawLine in source.split(separator: "\n", omittingEmptySubsequences: false) {
            var quoted = false
            var escaped = false
            var characters = ""
            for character in rawLine {
                if character == "#", !quoted { break }
                characters.append(character)
                if escaped { escaped = false; continue }
                if character == "\\", quoted { escaped = true }
                else if character == "\"" { quoted.toggle() }
            }
            let line = characters.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !line.isEmpty else { continue }
            guard !line.hasPrefix("["), let separator = line.firstIndex(of: "=") else { return nil }
            let key = line[..<separator].trimmingCharacters(in: .whitespaces)
            guard values[key] == nil else { return nil }
            values[key] = line[line.index(after: separator)...].trimmingCharacters(in: .whitespaces)
        }
        func string(_ key: String) -> String? {
            guard let raw = values[key]?.data(using: .utf8) else { return nil }
            return try? JSONDecoder().decode(String.self, from: raw)
        }
        func strings(_ key: String) -> [String]? {
            guard let value = values[key] else { return [] }
            return try? JSONDecoder().decode([String].self, from: Data(value.utf8))
        }
        guard let capabilities = strings("capabilities"),
              ["default_enabled", "include_in_global_search"].allSatisfy({ key in
                  values[key] == nil || values[key] == "true" || values[key] == "false"
              }) else { return nil }
        guard let id = string("id"), id.range(of: #"^[A-Za-z0-9][A-Za-z0-9._-]+$"#, options: .regularExpression) != nil,
              let name = string("name"), !name.isEmpty, let version = string("version"), !version.isEmpty,
              let entrypoint = string("entrypoint"), !entrypoint.isEmpty, !entrypoint.contains("/") && !entrypoint.contains(".."),
              let protocolVersion = values["protocol_version"].flatMap(Int.init), protocolVersion == 1
        else { return nil }
        let defaultShortcut = string("default_shortcut").flatMap(PluginShortcut.init(definition:))
        guard values["default_shortcut"] == nil || defaultShortcut != nil,
              values["shortcut_host_action"] == nil || string("shortcut_host_action") != nil else { return nil }
        let shortcutHostAction = string("shortcut_host_action")
        if let shortcutHostAction {
            // Direct shortcuts only support services designed to run without a
            // search result, and still require the manifest capability.
            guard shortcutHostAction == PluginHostAction.composeText.rawValue,
                  capabilities.contains(PluginHostAction.composeText.capability) else { return nil }
        }
        return PluginDescriptor(
            id: id,
            name: name,
            version: version,
            protocolVersion: protocolVersion,
            executable: entrypoint,
            executableURL: directory.appendingPathComponent(entrypoint),
            defaultEnabled: values["default_enabled"] != "false",
            defaultIncludedInGlobalSearch: values["include_in_global_search"] == "true",
            capabilities: capabilities,
            iconName: string("icon"),
            summary: string("description"),
            defaultShortcut: defaultShortcut,
            shortcutHostAction: shortcutHostAction
        )
    }
}
