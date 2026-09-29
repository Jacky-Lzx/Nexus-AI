import Foundation

struct PluginTransportError: Error, LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Incremental framing with a fixed cap, including chunks without a newline.
struct PluginLineFramer {
    static let maxBytes = 16 * 1024 * 1024
    private var buffer = Data()
    let limit: Int
    init(limit: Int = Self.maxBytes) { self.limit = limit }

    mutating func consume(_ data: Data, line: (Data) throws -> Void) throws {
        var start = data.startIndex
        while start < data.endIndex {
            let end = data[start...].firstIndex(of: 0x0A) ?? data.endIndex
            guard buffer.count + data.distance(from: start, to: end) <= limit else {
                throw PluginTransportError(message: "Plugin message exceeds \(limit) bytes")
            }
            buffer.append(contentsOf: data[start..<end])
            if end != data.endIndex {
                if !buffer.isEmpty { try line(buffer) }
                buffer.removeAll(keepingCapacity: false)
                start = data.index(after: end)
            } else { start = end }
        }
    }
    mutating func finish() throws {
        guard buffer.isEmpty else { throw PluginTransportError(message: "Plugin output ended with an incomplete message") }
    }
}

/// One blocking pipe writer outside the main queue, with bounded pending storage.
final class PluginWriteQueue {
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "io.clipkeep.plugin.write")
    private var frames: [Data] = []
    private var bytes = 0
    private var draining = false
    private var closed = false
    private let handle: FileHandle
    private let failure: (String) -> Void
    init(handle: FileHandle, failure: @escaping (String) -> Void) { self.handle = handle; self.failure = failure }

    func enqueue(_ data: Data) -> Bool {
        lock.lock()
        guard !closed, frames.count < 64, data.count <= PluginLineFramer.maxBytes + 1,
              bytes + data.count <= 32 * 1024 * 1024 else { lock.unlock(); return false }
        frames.append(data)
        bytes += data.count
        let start = !draining
        draining = true
        lock.unlock()
        if start { queue.async { [self] in drain() } }
        return true
    }
    func close() {
        lock.lock(); closed = true; frames.removeAll(); bytes = 0; lock.unlock()
    }
    private func drain() {
        while true {
            lock.lock()
            guard !closed, !frames.isEmpty else { draining = false; lock.unlock(); return }
            let frame = frames.removeFirst()
            lock.unlock()
            do { try handle.write(contentsOf: frame) }
            catch { close(); failure("Plugin input write failed: \(error.localizedDescription)"); return }
            lock.lock(); bytes = max(0, bytes - frame.count); lock.unlock()
        }
    }
}
