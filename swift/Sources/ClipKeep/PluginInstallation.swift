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

/// Re-scan only when a plugin directory changes, including files inside existing bundles.
final class PluginDirectoryMonitor {
    private let roots: [URL]
    private let onChange: () -> Void
    private var sources: [DispatchSourceFileSystemObject] = []
    private var pendingReload: DispatchWorkItem?

    init(roots: [URL] = PluginRegistry.roots, onChange: @escaping () -> Void) {
        self.roots = roots
        self.onChange = onChange
        let userDirectory = PluginRegistry.userDirectory.standardizedFileURL
        if roots.contains(where: { $0.standardizedFileURL == userDirectory }) {
            try? FileManager.default.createDirectory(
                at: PluginRegistry.userDirectory,
                withIntermediateDirectories: true
            )
        }
        rebuildWatches()
    }

    deinit {
        pendingReload?.cancel()
        sources.forEach { $0.cancel() }
    }

    private func rebuildWatches() {
        sources.forEach { $0.cancel() }
        sources.removeAll()

        for directory in watchedDirectories() {
            let descriptor = open(directory.path, O_EVTONLY | O_CLOEXEC)
            guard descriptor >= 0 else { continue }
            let source = DispatchSource.makeFileSystemObjectSource(
                fileDescriptor: descriptor,
                eventMask: [.write, .extend, .attrib, .rename, .delete],
                queue: .main
            )
            source.setEventHandler { [weak self] in self?.scheduleReload() }
            source.setCancelHandler { close(descriptor) }
            sources.append(source)
            source.resume()
        }
    }

    private func watchedDirectories() -> [URL] {
        var directories: [URL] = []
        for root in roots {
            guard (try? root.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true else {
                continue
            }
            directories.append(root)
            guard let children = try? FileManager.default.contentsOfDirectory(
                at: root,
                includingPropertiesForKeys: [.isDirectoryKey],
                options: [.skipsHiddenFiles]
            ) else { continue }
            for bundle in children where bundle.pathExtension == "clipkeepplugin" {
                directories.append(bundle)
                guard let enumerator = FileManager.default.enumerator(
                    at: bundle,
                    includingPropertiesForKeys: [.isDirectoryKey],
                    options: [.skipsHiddenFiles]
                ) else { continue }
                for case let url as URL in enumerator
                    where (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
                    directories.append(url)
                }
            }
        }
        return directories
    }

    private func scheduleReload() {
        pendingReload?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.onChange()
            self.rebuildWatches()
        }
        pendingReload = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2, execute: work)
    }
}
