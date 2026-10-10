import AppKit

final class ExternalTextEditor {
    struct Configuration {
        let terminalPath: String
        let editorPath: String
    }
    private enum EditorError: LocalizedError {
        case executableNotFound(String)
        case editorFailed(Int32)

        var errorDescription: String? {
            switch self {
            case let .executableNotFound(name):
                "Could not find \(name). Set its executable path in the plugin settings."
            case let .editorFailed(status):
                "The external editor exited with status \(status)."
            }
        }
    }

    private var processes: [UUID: Process] = [:]
    private var dockedWindows: [UUID: DockedEditorWindow] = [:]

    static func compositionText(_ text: String) -> String {
        if text.hasSuffix("\r\n") { return String(decoding: text.utf8.dropLast(2), as: UTF8.self) }
        if text.hasSuffix("\n") { return String(decoding: text.utf8.dropLast(), as: UTF8.self) }
        return text
    }

    // A successful write alone is insufficient: :q! after further edits, :cq,
    // and closing Kitty must all cancel the paste-back.
    static let compositionScript = """
    local buffer = vim.api.nvim_get_current_buf()
    local path = vim.api.nvim_buf_get_name(buffer)
    local saved = false
    vim.opt_local.swapfile = false
    vim.opt_local.undofile = false
    vim.opt_local.backup = false
    vim.opt_local.writebackup = false
    vim.api.nvim_create_autocmd('BufWritePost', {
      buffer = buffer,
      callback = function() saved = true end,
    })
    vim.api.nvim_create_autocmd('VimLeavePre', {
      callback = function()
        if saved and vim.v.exiting == 0 and vim.api.nvim_buf_is_valid(buffer)
          and vim.api.nvim_buf_get_name(buffer) == path
          and not vim.bo[buffer].modified then
          vim.fn.writefile({'saved'}, vim.env.NEXUS_EDITOR_SAVED_PATH)
        end
      end,
    })
    vim.cmd.startinsert()
    """

    func edit(_ text: String, completion: @escaping (Result<String, Error>) -> Void) {
        launch(text, bottomFrame: nil, configuration: Configuration(
            terminalPath: ClipboardPreference.terminalExecutablePath,
            editorPath: ClipboardPreference.editorExecutablePath
        )) { result in
            completion(result.map { $0 ?? text })
        }
    }

    func compose(bottomFrame: NSRect, configuration: Configuration, completion: @escaping (Result<String?, Error>) -> Void) {
        launch("", bottomFrame: bottomFrame, configuration: configuration, completion: completion)
    }

    func focusComposition() {
        for (id, dockedWindow) in dockedWindows {
            guard let process = processes[id] else { continue }
            dockedWindow.focus()
            NSRunningApplication(processIdentifier: process.processIdentifier)?.activate()
        }
    }

    private func launch(
        _ text: String,
        bottomFrame: NSRect?,
        configuration: Configuration,
        completion: @escaping (Result<String?, Error>) -> Void
    ) {
        let sessionId = UUID()
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nexus-editor-\(sessionId.uuidString)", isDirectory: true)
        let file = directory.appendingPathComponent("clipboard.txt")
        let savedFile = directory.appendingPathComponent("saved")

        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: false,
                attributes: [.posixPermissions: 0o700]
            )
            try text.write(to: file, atomically: false, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)

            let terminal = try Self.resolveExecutable(
                configuredPath: configuration.terminalPath,
                name: "Kitty",
                command: "kitty",
                fallbacks: [
                    "/opt/homebrew/bin/kitty",
                    "/usr/local/bin/kitty",
                    "/Applications/kitty.app/Contents/MacOS/kitty",
                ]
            )
            let editor = try Self.resolveExecutable(
                configuredPath: configuration.editorPath,
                name: "Neovim",
                command: "nvim",
                fallbacks: ["/opt/homebrew/bin/nvim", "/usr/local/bin/nvim"]
            )

            let process = Process()
            process.executableURL = terminal
            var editorArguments = [
                "--start-as=normal",
                "--title=\(bottomFrame == nil ? "nvim_edit" : "nvim_input — Nexus")",
                "--session=none",
            ]
            if let bottomFrame {
                let position = DockedEditorWindow.accessibilityFrame(bottomFrame)
                editorArguments += [
                    "--position=\(Int(position.minX))x\(Int(position.minY))",
                    "-o", "remember_window_size=no",
                    "-o", "initial_window_width=\(Int(bottomFrame.width))",
                    "-o", "initial_window_height=\(Int(bottomFrame.height))",
                    "-o", "confirm_os_window_close=0",
                ]
                let script = directory.appendingPathComponent("session.lua")
                try Self.compositionScript.write(to: script, atomically: false, encoding: .utf8)
                var environment = ProcessInfo.processInfo.environment
                environment["NEXUS_EDITOR_SESSION_SCRIPT"] = script.path
                environment["NEXUS_EDITOR_SAVED_PATH"] = savedFile.path
                environment["NVIM_LOG_FILE"] = directory.appendingPathComponent("nvim.log").path
                process.environment = environment
            }
            editorArguments.append(editor.path)
            if bottomFrame != nil { editorArguments += ["-i", "NONE"] }
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
            if bottomFrame != nil {
                editorArguments += ["-c", "lua dofile(vim.env.NEXUS_EDITOR_SESSION_SCRIPT)"]
            }
            editorArguments.append(file.path)
            process.arguments = editorArguments
            process.terminationHandler = { [weak self] process in
                let result: Result<String?, Error>
                if process.terminationStatus == 0 {
                    result = Result {
                        guard bottomFrame == nil || FileManager.default.fileExists(atPath: savedFile.path)
                        else { return nil }
                        let savedText = try String(contentsOf: file, encoding: .utf8)
                        return bottomFrame == nil ? savedText : Self.compositionText(savedText)
                    }
                } else {
                    result = .failure(EditorError.editorFailed(process.terminationStatus))
                }
                try? FileManager.default.removeItem(at: directory)
                DispatchQueue.main.async {
                    self?.processes[sessionId] = nil
                    self?.dockedWindows[sessionId]?.stop()
                    self?.dockedWindows[sessionId] = nil
                    completion(result)
                }
            }
            try process.run()
            processes[sessionId] = process
            if let bottomFrame {
                let dockedWindow = DockedEditorWindow(processIdentifier: process.processIdentifier, frame: bottomFrame)
                dockedWindows[sessionId] = dockedWindow
                dockedWindow.start()
            }
            activateEditor(
                processIdentifier: process.processIdentifier,
                remainingAttempts: 15
            )
        } catch {
            try? FileManager.default.removeItem(at: directory)
            completion(.failure(error))
        }
    }

    private func activateEditor(
        processIdentifier: pid_t,
        remainingAttempts: Int
    ) {
        guard remainingAttempts > 0, processes.values.contains(where: { $0.processIdentifier == processIdentifier })
        else { return }
        let application = NSRunningApplication(processIdentifier: processIdentifier)
        if let application {
            application.activate(options: [.activateAllWindows])
            if application.isActive { return }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { [weak self] in
            self?.activateEditor(
                processIdentifier: processIdentifier,
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
