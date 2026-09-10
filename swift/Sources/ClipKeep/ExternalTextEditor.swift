import AppKit

final class ExternalTextEditor {
    private enum EditorError: LocalizedError {
        case executableNotFound(String)
        case editorFailed(Int32)

        var errorDescription: String? {
            switch self {
            case let .executableNotFound(name):
                "Could not find \(name). Set its executable path in Clipboard History settings."
            case let .editorFailed(status):
                "The external editor exited with status \(status)."
            }
        }
    }

    private var processes: [UUID: Process] = [:]

    func edit(_ text: String, completion: @escaping (Result<String, Error>) -> Void) {
        let sessionId = UUID()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nexus-editor-\(sessionId.uuidString)", isDirectory: true)
        let file = directory.appendingPathComponent("clipboard.txt")

        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: false,
                attributes: [.posixPermissions: 0o700]
            )
            try text.write(to: file, atomically: false, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)

            let terminal = try Self.resolveExecutable(
                configuredPath: ClipboardPreference.terminalExecutablePath,
                name: "Kitty",
                command: "kitty",
                fallbacks: [
                    "/opt/homebrew/bin/kitty",
                    "/usr/local/bin/kitty",
                    "/Applications/kitty.app/Contents/MacOS/kitty",
                ]
            )
            let editor = try Self.resolveExecutable(
                configuredPath: ClipboardPreference.editorExecutablePath,
                name: "Neovim",
                command: "nvim",
                fallbacks: ["/opt/homebrew/bin/nvim", "/usr/local/bin/nvim"]
            )

            let process = Process()
            process.executableURL = terminal
            var editorArguments = [
                "--start-as=normal",
                "--title=nvim_edit",
                editor.path,
            ]
            if text.hasSuffix("\n") {
                // Turn Vim's implicit EOF newline into an editable final empty line.
                editorArguments += ["-c", "call append(line('$'), '')"]
            }
            // Run after user BufWritePre hooks so Vim cannot synthesize an EOL.
            // With no implicit EOF newline, empty final lines map exactly to \n bytes.
            editorArguments += [
                "-c", "setlocal noendofline nofixeol nomodified",
                "-c", "autocmd BufWritePre <buffer> setlocal noendofline nofixeol",
            ]
            editorArguments.append(file.path)
            process.arguments = editorArguments
            process.terminationHandler = { [weak self] process in
                let result: Result<String, Error>
                if process.terminationStatus == 0 {
                    result = Result { try String(contentsOf: file, encoding: .utf8) }
                } else {
                    result = .failure(EditorError.editorFailed(process.terminationStatus))
                }
                try? FileManager.default.removeItem(at: directory)
                DispatchQueue.main.async {
                    self?.processes[sessionId] = nil
                    completion(result)
                }
            }
            try process.run()
            processes[sessionId] = process
            activateEditor(
                processIdentifier: process.processIdentifier,
                executableURL: terminal,
                remainingAttempts: 15
            )
        } catch {
            try? FileManager.default.removeItem(at: directory)
            completion(.failure(error))
        }
    }

    private func activateEditor(
        processIdentifier: pid_t,
        executableURL: URL,
        remainingAttempts: Int
    ) {
        guard remainingAttempts > 0, processes.values.contains(where: { $0.processIdentifier == processIdentifier })
        else { return }
        let application = NSRunningApplication(processIdentifier: processIdentifier)
            ?? NSWorkspace.shared.runningApplications.first(where: {
                $0.executableURL?.standardizedFileURL == executableURL.standardizedFileURL
            })
        if let application {
            application.activate(options: [.activateAllWindows])
            if application.isActive { return }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { [weak self] in
            self?.activateEditor(
                processIdentifier: processIdentifier,
                executableURL: executableURL,
                remainingAttempts: remainingAttempts - 1
            )
        }
    }

    private static func resolveExecutable(
        configuredPath: String,
        name: String,
        command: String,
        fallbacks: [String]
    ) throws -> URL {
        let configured = configuredPath.trimmingCharacters(in: .whitespacesAndNewlines)
        let candidates: [String]
        if configured.isEmpty {
            let pathCandidates = ProcessInfo.processInfo.environment["PATH"]?
                .split(separator: ":")
                .map { String($0) + "/" + command } ?? []
            candidates = pathCandidates + fallbacks
        } else {
            candidates = [(configured as NSString).expandingTildeInPath]
        }
        guard let path = candidates.first(where: FileManager.default.isExecutableFile(atPath:)) else {
            throw EditorError.executableNotFound(name)
        }
        return URL(fileURLWithPath: path)
    }
}
