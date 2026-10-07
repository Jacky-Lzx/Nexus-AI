import AppKit

// A file-only diagnostic path exercises the same recognizer without changing the clipboard.
if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--recognize-file" {
    do { print(try recognize(Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2])))) }
    catch { FileHandle.standardError.write(Data("\(error.localizedDescription)\n".utf8)); exit(1) }
    exit(0)
}


let plugin = OCRPlugin()
while let line = readLine() {
    autoreleasepool {
        guard let data = line.data(using: .utf8),
              let request = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        if request["type"] as? String == "shutdown" { exit(0) }
        plugin.queue.sync { plugin.handle(request) }
    }
}
