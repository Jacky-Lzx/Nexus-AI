import Foundation

struct PluginFailure {
    let message: String
    var queryId: UInt64? = nil
    var requestId: UInt64? = nil
    var code: String? = nil
    var retryable = false
}
