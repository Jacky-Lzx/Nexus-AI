import AppKit

@main
struct ScreenshotWorkflowTests {
    static func main() throws {
        let image = Data("screenshot fixture".utf8)
        let error = NSError(domain: "OCRTest", code: 1,
                            userInfo: [NSLocalizedDescriptionKey: "capture unavailable"])
        func scenario(capture: @escaping () throws -> Data?,
                      recognition: @escaping (Data) throws -> String,
                      copySucceeds: Bool = true,
                      expectedMessage: String, expectedText: String, expectedCopies: [String]) {
            var events: [[String: Any]] = []
            var copies: [String] = []
            let done = DispatchSemaphore(value: 0)
            let plugin = OCRPlugin(capture: capture, recognizer: recognition, copyText: {
                copies.append($0)
                return copySucceeds
            }, emit: {
                events.append($0)
                if $0["type"] as? String == "notification" { done.signal() }
            })
            plugin.queue.sync {
                plugin.text = "previous result"
                plugin.execute(["request_id": 1, "result_id": "screenshot", "action_id": "screenshot"])
                assert(plugin.busy)
                let acknowledged = events.first { $0["type"] as? String == "action_finished" }!
                assert(acknowledged["success"] as? Bool == true)
                assert(acknowledged["message"] == nil) // no toast over the screenshot selector
            }
            assert(done.wait(timeout: .now() + 3) == .success)
            plugin.queue.sync {
                assert(!plugin.busy)
                assert(plugin.screenshotStatus.contains(expectedMessage), plugin.screenshotStatus)
                assert(plugin.text == expectedText)
                assert(copies == expectedCopies)
                let body = (events.last!["notification"] as! [String: Any])["body"] as! String
                assert(body == plugin.screenshotStatus)
            }
        }
        scenario(capture: { image }, recognition: { assert($0 == image); return "你好 Nexus 12345" },
                 expectedMessage: "copied to clipboard", expectedText: "你好 Nexus 12345",
                 expectedCopies: ["你好 Nexus 12345"])
        scenario(capture: { nil }, recognition: { _ in fatalError("cancelled capture must not recognize") },
                 expectedMessage: "cancelled", expectedText: "previous result", expectedCopies: [])
        scenario(capture: { throw error }, recognition: { _ in fatalError("failed capture must not recognize") },
                 expectedMessage: "capture unavailable", expectedText: "previous result", expectedCopies: [])
        scenario(capture: { image }, recognition: { _ in throw error },
                 expectedMessage: "capture unavailable", expectedText: "previous result", expectedCopies: [])
        scenario(capture: { image }, recognition: { _ in "" },
                 expectedMessage: "No text found", expectedText: "", expectedCopies: [])
        scenario(capture: { image }, recognition: { _ in " \n " },
                 expectedMessage: "No text found", expectedText: "", expectedCopies: [])
        scenario(capture: { image }, recognition: { _ in "recognized" }, copySucceeds: false,
                 expectedMessage: "Could not copy", expectedText: "recognized", expectedCopies: ["recognized"])

        // The selector may stay open beyond the host deadline. Search remains responsive,
        // and competing screenshot/clipboard/copy actions cannot start another operation.
        let release = DispatchSemaphore(value: 0), done = DispatchSemaphore(value: 0)
        var events: [[String: Any]] = []
        let plugin = OCRPlugin(capture: { release.wait(); return nil }, emit: {
            events.append($0)
            if $0["type"] as? String == "notification" { done.signal() }
        })
        plugin.queue.sync {
            let screenshot = plugin.results(query: "ocr", limit: 1).first!
            assert(screenshot["id"] as? String == "screenshot")
            assert((screenshot["actions"] as! [[String: Any]]).first!["closes_window"] as? Bool == true)
            assert(plugin.results(query: "截图", limit: 10).count == 1)
            assert(plugin.results(query: "unrelated", limit: 10).isEmpty)
            assert(plugin.results(query: "ocr", limit: 0).isEmpty)
            plugin.text = "old"
            plugin.execute(["request_id": 10, "result_id": "screenshot", "action_id": "screenshot"])
            plugin.handle(["type": "search", "query_id": 11, "query": "ocr", "limit": 10])
            let items = events.last!["items"] as! [[String: Any]]
            assert(items.allSatisfy { ($0["actions"] as! [[String: Any]]).isEmpty })
            for (result, action) in [("screenshot", "screenshot"), ("recognize", "recognize"), ("text:0", "copy")] {
                plugin.execute(["request_id": 12, "result_id": result, "action_id": action])
                assert(events.last!["success"] as? Bool == false)
            }
        }
        release.signal()
        assert(done.wait(timeout: .now() + 3) == .success)
        print("PASS: screenshot auto-copy, cancellation, capture/OCR/copy errors, empty image, search and busy/stale guards")
    }
}
