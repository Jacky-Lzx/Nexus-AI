import AppKit
import Vision
import ImageIO

func recognize(_ data: Data) throws -> String {
    guard data.count <= 64 * 1024 * 1024,
          let source = CGImageSourceCreateWithData(data as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0, Double(width) * Double(height) <= 80_000_000 else {
        throw NSError(domain: "OCR", code: 1, userInfo: [NSLocalizedDescriptionKey: "Invalid image or image too large (64 MB / 80 megapixels maximum)"])
    }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
    request.automaticallyDetectsLanguage = true
    request.usesLanguageCorrection = true
    try VNImageRequestHandler(data: data).perform([request])
    return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
}

// A file-only diagnostic path exercises the same recognizer without changing the clipboard.
if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--recognize-file" {
    do { print(try recognize(Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2])))) }
    catch { FileHandle.standardError.write(Data("\(error.localizedDescription)\n".utf8)); exit(1) }
    exit(0)
}

final class OCRPlugin {
    let queue = DispatchQueue(label: "io.clipkeep.ocr.state")
    var busy = false
    var text = ""
    var generation = 0
    var status = "Copy an image, then press Return to recognize its text"

    func send(_ event: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: event, options: [.sortedKeys]) else { return }
        FileHandle.standardOutput.write(data + Data([10]))
    }

    func action(_ id: String, _ title: String, closes: Bool = false) -> [String: Any] {
        ["id": id, "title": title, "default": true, "closes_window": closes]
    }

    func results(query: String, limit: Int) -> [[String: Any]] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard limit > 0, query.isEmpty || ["ocr", "recognize text", "识别文字", "文字识别"].contains(where: { $0.contains(query) }) else { return [] }
        var items: [[String: Any]] = []
        if !text.isEmpty {
            items.append([
                "plugin_id": "io.clipkeep.ocr", "id": "text:\(generation)",
                "title": "OCR Result", "subtitle": String(text.prefix(160)).replacingOccurrences(of: "\n", with: " · "),
                "preview": text, "kind": "command", "score": 4100,
                "metadata": ["symbol_name": "doc.text", "preview_style": "text"],
                "actions": [action("copy", "Copy Text", closes: true)],
            ])
        }
        items.append([
            "plugin_id": "io.clipkeep.ocr", "id": "recognize", "title": busy ? "Recognizing Text…" : "OCR Clipboard Image",
            "subtitle": status, "preview": status, "kind": "command", "score": 4000,
            "metadata": ["symbol_name": "text.viewfinder"],
            "actions": busy ? [] : [action("recognize", "Recognize Clipboard Image")],
        ])
        return Array(items.prefix(limit))
    }

    func execute(_ request: [String: Any]) {
        let requestID = request["request_id"] ?? 0
        func finished(_ success: Bool, _ message: String) {
            send(["type": "action_finished", "request_id": requestID, "success": success, "message": message])
        }
        if request["action_id"] as? String == "copy", request["result_id"] as? String == "text:\(generation)", !text.isEmpty {
            let clipboard = NSPasteboard.general
            clipboard.clearContents()
            finished(clipboard.setString(text, forType: .string), "OCR text copied")
            return
        }
        guard request["action_id"] as? String == "recognize", request["result_id"] as? String == "recognize" else {
            finished(false, "OCR result is no longer available"); return
        }
        guard !busy else { finished(false, "Recognition is already running"); return }
        let clipboard = NSPasteboard.general
        guard let data = clipboard.data(forType: .png) ?? clipboard.data(forType: .tiff) else {
            finished(false, "No image in the clipboard. Copy a screenshot or image first."); return
        }
        busy = true
        text = ""
        generation += 1
        status = "Recognizing locally…"
        finished(true, "OCR started")
        send(["type": "catalog_changed"])
        DispatchQueue.global(qos: .userInitiated).async {
            let outcome = Result { try recognize(data) }
            self.queue.async {
                self.busy = false
                switch outcome {
                case .success(let text):
                    self.text = text
                    self.status = text.isEmpty ? "No text found. Try a clearer image." : "Recognize another clipboard image"
                case .failure(let error):
                    self.status = "OCR failed: \(error.localizedDescription)"
                }
                self.send(["type": "catalog_changed"])
            }
        }
    }

    func handle(_ request: [String: Any]) {
        switch request["type"] as? String {
        case "initialize":
            if request["protocol_version"] as? Int == 1 {
                send(["type": "initialized", "plugin_id": "io.clipkeep.ocr", "protocol_version": 1])
            } else { send(["type": "error", "message": "Unsupported protocol version"]) }
        case "search":
            send(["type": "search_results", "query_id": request["query_id"] ?? 0,
                  "items": results(query: request["query"] as? String ?? "", limit: request["limit"] as? Int ?? 50)])
        case "execute": execute(request)
        case "list_configuration_items":
            send(["type": "configuration_items", "request_id": request["request_id"] ?? 0, "configuration_items": []])
        default: break
        }
    }
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
