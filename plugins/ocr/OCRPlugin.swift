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

// Capture to a private temporary file so cancellation never replaces the clipboard.
func captureScreenshot() throws -> Data? {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nexus-ocr-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                          attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: directory) }
    let file = directory.appendingPathComponent("screenshot.png")
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-i", "-x", "-t", "png", file.path]
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    // Nexus takes 0.13 seconds to hide its panel before selection begins.
    Thread.sleep(forTimeInterval: 0.3)
    try process.run()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
        throw NSError(domain: "OCR", code: 2, userInfo: [NSLocalizedDescriptionKey:
            "Screenshot failed. Check Screen Recording permission for Nexus in System Settings."])
    }
    guard FileManager.default.fileExists(atPath: file.path) else { return nil }
    return try Data(contentsOf: file)
}

func copyOCRText(_ text: String) -> Bool {
    let clipboard = NSPasteboard.general
    clipboard.clearContents()
    return clipboard.setString(text, forType: .string)
}

final class OCRPlugin {
    let queue = DispatchQueue(label: "io.clipkeep.ocr.state")
    let capture: () throws -> Data?
    let recognizer: (Data) throws -> String
    let copyText: (String) -> Bool
    let emit: (([String: Any]) -> Void)?
    var busy = false
    var text = ""
    var generation = 0
    var status = "Copy an image, then press Return to recognize its text"
    var screenshotStatus = "Select an area (Space for a window); text is copied automatically"

    init(capture: @escaping () throws -> Data? = captureScreenshot,
         recognizer: @escaping (Data) throws -> String = recognize,
         copyText: @escaping (String) -> Bool = copyOCRText,
         emit: (([String: Any]) -> Void)? = nil) {
        self.capture = capture
        self.recognizer = recognizer
        self.copyText = copyText
        self.emit = emit
    }

    func send(_ event: [String: Any]) {
        if let emit { emit(event); return }
        guard let data = try? JSONSerialization.data(withJSONObject: event, options: [.sortedKeys]) else { return }
        FileHandle.standardOutput.write(data + Data([10]))
    }

    func action(_ id: String, _ title: String, closes: Bool = false) -> [String: Any] {
        ["id": id, "title": title, "default": true, "closes_window": closes]
    }

    func results(query: String, limit: Int) -> [[String: Any]] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let commonMatch = query.isEmpty || ["ocr", "recognize text", "识别文字", "文字识别"].contains(where: { $0.contains(query) })
        let screenshotMatch = commonMatch || ["screenshot", "capture", "截图", "截屏", "一条龙"].contains(where: { $0.contains(query) })
        guard limit > 0, screenshotMatch else { return [] }
        var items: [[String: Any]] = []
        items.append([
            "plugin_id": "io.clipkeep.ocr", "id": "screenshot", "title": "OCR Screenshot → Copy Text",
            "subtitle": busy ? "OCR is busy…" : screenshotStatus, "preview": screenshotStatus,
            "kind": "command", "score": 4200,
            "metadata": ["symbol_name": "viewfinder"],
            "actions": busy ? [] : [action("screenshot", "Screenshot and Copy Text", closes: true)],
        ])
        guard commonMatch else { return Array(items.prefix(limit)) }
        if !text.isEmpty {
            items.append([
                "plugin_id": "io.clipkeep.ocr", "id": "text:\(generation)",
                "title": "OCR Result", "subtitle": String(text.prefix(160)).replacingOccurrences(of: "\n", with: " · "),
                "preview": text, "kind": "command", "score": 4100,
                "metadata": ["symbol_name": "doc.text", "preview_style": "text"],
                "actions": busy ? [] : [action("copy", "Copy Text", closes: true)],
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
        if request["action_id"] as? String == "copy", request["result_id"] as? String == "text:\(generation)", !text.isEmpty, !busy {
            let copied = copyText(text)
            finished(copied, copied ? "OCR text copied" : "Could not copy OCR text")
            return
        }
        if request["action_id"] as? String == "screenshot", request["result_id"] as? String == "screenshot" {
            guard !busy else { finished(false, "Recognition is already running"); return }
            busy = true
            generation += 1
            screenshotStatus = "Select an area; press Esc to cancel"
            // Acknowledge immediately; the host's action deadline is five seconds.
            // Completion is reported through a separate notification after selection/OCR.
            send(["type": "action_finished", "request_id": requestID, "success": true])
            send(["type": "catalog_changed"])
            DispatchQueue.global(qos: .userInitiated).async {
                let outcome = Result { () throws -> String? in
                    guard let data = try self.capture() else { return nil }
                    self.queue.async {
                        self.screenshotStatus = "Recognizing screenshot locally…"
                        self.send(["type": "catalog_changed"])
                    }
                    return try self.recognizer(data)
                }
                self.queue.async {
                    self.busy = false
                    switch outcome {
                    case .success(nil):
                        self.screenshotStatus = "Screenshot cancelled. Select an area to try again."
                    case .success(let recognized?):
                        if recognized.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            self.text = ""
                            self.screenshotStatus = "No text found. Try a clearer screenshot."
                        } else {
                            self.text = recognized
                            self.screenshotStatus = self.copyText(recognized)
                                ? "OCR text copied to clipboard"
                                : "Could not copy OCR text. Open OCR Result to copy it again."
                        }
                    case .failure(let error):
                        self.screenshotStatus = "OCR failed: \(error.localizedDescription)"
                    }
                    self.send(["type": "catalog_changed"])
                    self.send(["type": "notification", "notification": [
                        "id": "ocr-screenshot:\(self.generation)", "title": "OCR", "body": self.screenshotStatus,
                    ]])
                }
            }
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
            let outcome = Result { try self.recognizer(data) }
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
