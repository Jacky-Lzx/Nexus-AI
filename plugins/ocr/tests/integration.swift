import AppKit

let binary = CommandLine.arguments[1]
let clipboard = NSPasteboard.general
let saved = (clipboard.pasteboardItems ?? []).map { item -> NSPasteboardItem in
    let copy = NSPasteboardItem()
    for type in item.types { if let data = item.data(forType: type) { copy.setData(data, forType: type) } }
    return copy
}
defer { clipboard.clearContents(); clipboard.writeObjects(saved) }

let image = NSImage(size: NSSize(width: 1000, height: 260))
image.lockFocus()
NSColor.white.setFill()
NSRect(x: 0, y: 0, width: 1000, height: 260).fill()
("Nexus OCR 12345\n你好，世界" as NSString).draw(at: NSPoint(x: 50, y: 70), withAttributes: [.font: NSFont.systemFont(ofSize: 48), .foregroundColor: NSColor.black])
image.unlockFocus()
let png = NSBitmapImageRep(data: image.tiffRepresentation!)!.representation(using: .png, properties: [:])!
// Blank and corrupt inputs exercise the recognizer's non-success paths.
let temporary = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: temporary) }
let blank = NSImage(size: NSSize(width: 300, height: 100))
blank.lockFocus()
NSColor.white.setFill()
NSRect(x: 0, y: 0, width: 300, height: 100).fill()
blank.unlockFocus()
let blankData = NSBitmapImageRep(data: blank.tiffRepresentation!)!.representation(using: .png, properties: [:])!
for (name, data, expectedStatus) in [("blank.png", blankData, Int32(0)), ("invalid.png", Data("invalid".utf8), Int32(1))] {
    let file = temporary.appendingPathComponent(name)
    try data.write(to: file)
    let diagnostic = Process(), diagnosticOutput = Pipe()
    diagnostic.executableURL = URL(fileURLWithPath: binary)
    diagnostic.arguments = ["--recognize-file", file.path]
    diagnostic.standardOutput = diagnosticOutput
    diagnostic.standardError = FileHandle.nullDevice
    try diagnostic.run()
    let output = diagnosticOutput.fileHandleForReading.readDataToEndOfFile()
    diagnostic.waitUntilExit()
    assert(diagnostic.terminationStatus == expectedStatus)
    if expectedStatus == 0 { assert(String(data: output, encoding: .utf8)!.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
}
let process = Process()
let input = Pipe(), output = Pipe()
process.executableURL = URL(fileURLWithPath: binary)
process.standardInput = input; process.standardOutput = output
try process.run()
defer { if process.isRunning { process.terminate() }; process.waitUntilExit() }
func send(_ value: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: value)
    input.fileHandleForWriting.write(data + Data([10]))
}
func receive(_ type: String) throws -> [String: Any] {
    while true {
        var data = Data()
        while let byte = try output.fileHandleForReading.read(upToCount: 1), !byte.isEmpty {
            if byte[0] == 10 { break }; data.append(byte)
        }
        let event = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        if event["type"] as? String == type { return event }
    }
}
try send(["type": "initialize", "protocol_version": 1])
_ = try receive("initialized")
clipboard.clearContents(); clipboard.setString("no image", forType: .string)
try send(["type": "execute", "request_id": 1, "result_id": "recognize", "action_id": "recognize"])
let missing = try receive("action_finished")
assert(missing["success"] as? Bool == false)
clipboard.clearContents(); clipboard.setData(png, forType: .png)
try send(["type": "execute", "request_id": 2, "result_id": "recognize", "action_id": "recognize"])
let started = try receive("action_finished")
assert(started["success"] as? Bool == true)
// Search must remain responsive while recognition is running.
try send(["type": "search", "query_id": 3, "query": "ocr", "limit": 10])
let initial = try receive("search_results")
assert(initial["query_id"] as? Int == 3)
var result: [String: Any]?
for _ in 0..<100 {
    try send(["type": "search", "query_id": 4, "query": "ocr", "limit": 10])
    let items = try receive("search_results")["items"] as! [[String: Any]]
    result = items.first { ($0["id"] as? String)?.hasPrefix("text:") == true }
    if result != nil { break }
    Thread.sleep(forTimeInterval: 0.1)
}
let recognized = result!["preview"] as! String
assert(recognized.contains("Nexus OCR 12345") && recognized.contains("你好"))
try send(["type": "execute", "request_id": 5, "result_id": result!["id"]!, "action_id": "copy"])
let copied = try receive("action_finished")
assert(copied["success"] as? Bool == true)
assert(clipboard.string(forType: .string) == recognized)
try send(["type": "search", "query_id": 6, "query": "unrelated", "limit": 10])
let unrelated = try receive("search_results")
assert((unrelated["items"] as! [[String: Any]]).isEmpty)
try send(["type": "shutdown"])
process.waitUntilExit()
print("PASS: blank/corrupt images, missing image, async search, bilingual OCR, copy, query filtering, shutdown")
