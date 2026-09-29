import Foundation
import Testing
@testable import Nexus

@Test func framingHandlesChunksAndRejectsUnboundedOrIncompleteMessages() throws {
    var framer = PluginLineFramer(limit: 4)
    var lines: [Data] = []
    try framer.consume(Data("ab".utf8)) { lines.append($0) }
    try framer.consume(Data("cd\nx\n".utf8)) { lines.append($0) }
    #expect(lines == [Data("abcd".utf8), Data("x".utf8)])
    try framer.finish()
    #expect(throws: (any Error).self) { try framer.consume(Data("12345".utf8)) { _ in } }
    var partial = PluginLineFramer(limit: 4)
    try partial.consume(Data("a".utf8)) { _ in }
    #expect(throws: (any Error).self) { try partial.finish() }
}
