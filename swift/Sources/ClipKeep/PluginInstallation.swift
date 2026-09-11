import Foundation
import Darwin

enum PluginInstallationError: LocalizedError {
    case invalid(String)
    var errorDescription: String? {
        switch self { case let .invalid(message): message }
    }
}

final class PluginInstallation {
    let directory: URL

    init(directory: URL = PluginRegistry.userDirectory) {
        self.directory = directory.standardizedFileURL
    }

    func isUserInstalled(_ plugin: PluginDescriptor) -> Bool {
        plugin.bundleURL?.deletingLastPathComponent().standardizedFileURL.path == directory.path
    }

    @discardableResult
    func install(from source: URL) throws -> PluginDescriptor {
        let incoming = try PluginRegistry.load(source)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let existing = PluginRegistry.scan(roots: [directory]).descriptors.first { $0.id == incoming.id }
        let destination = existing?.bundleURL
            ?? directory.appendingPathComponent(incoming.id + ".clipkeepplugin", isDirectory: true)
        if source.standardizedFileURL.path == destination.standardizedFileURL.path { return incoming }
        let staging = directory.appendingPathComponent(".install-\(UUID().uuidString).clipkeepplugin")
        defer { try? FileManager.default.removeItem(at: staging) }
        try FileManager.default.copyItem(at: source, to: staging)
        let staged = try PluginRegistry.load(staging)
        guard staged.id == incoming.id else {
            throw PluginInstallationError.invalid("Plugin changed while being installed; try again")
        }
        if FileManager.default.fileExists(atPath: destination.path) {
            // Atomic directory exchange: the watcher never sees a half-installed update.
            guard renameatx_np(AT_FDCWD, staging.path, AT_FDCWD, destination.path, UInt32(RENAME_SWAP)) == 0 else {
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
        } else {
            try FileManager.default.moveItem(at: staging, to: destination)
        }
        return try PluginRegistry.load(destination)
    }

    func uninstall(_ plugin: PluginDescriptor) throws {
        guard isUserInstalled(plugin), let bundle = plugin.bundleURL else {
            throw PluginInstallationError.invalid("Bundled plugins can be disabled; only installed plugins can be uninstalled")
        }
        // Plugin data and preferences are deliberately retained; the package is recoverable.
        try FileManager.default.trashItem(at: bundle, resultingItemURL: nil)
    }
}

/// Re-scan on a low-frequency timer, including changes inside existing bundles.
/// File metadata avoids reloading unchanged plugins and catches atomic executable replacement.
final class PluginDirectoryMonitor {
    private var timer: Timer?

    init(onChange: @escaping () -> Void) {
        timer = Timer(timeInterval: 1, repeats: true) { _ in onChange() }
        if let timer { RunLoop.main.add(timer, forMode: .common) }
    }

    deinit { timer?.invalidate() }
}
