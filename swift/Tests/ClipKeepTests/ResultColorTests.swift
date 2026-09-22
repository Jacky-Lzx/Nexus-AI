import AppKit
import Testing
@testable import Nexus

@Test func parsesPluginSymbolColors() {
    let opaque = resultSymbolColor(from: "#336699")?.usingColorSpace(.sRGB)
    #expect(abs((opaque?.redComponent ?? 0) - 0.2) < 0.001)
    #expect(abs((opaque?.greenComponent ?? 0) - 0.4) < 0.001)
    #expect(abs((opaque?.blueComponent ?? 0) - 0.6) < 0.001)
    #expect(abs((opaque?.alphaComponent ?? 0) - 1) < 0.001)

    let translucent = resultSymbolColor(from: "#FF000080")?.usingColorSpace(.sRGB)
    #expect(abs((translucent?.redComponent ?? 0) - 1) < 0.001)
    #expect(abs((translucent?.alphaComponent ?? 0) - (128.0 / 255.0)) < 0.001)
    #expect(resultSymbolColor(from: "#123") == nil)
    #expect(resultSymbolColor(from: "not-a-color") == nil)
}
