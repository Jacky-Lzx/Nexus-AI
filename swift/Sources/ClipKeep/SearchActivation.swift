/// Keeps an activation tied to one search until its final results are published.
final class SearchActivation {
    private var queryId: UInt64 = 0
    private var settled = false
    private var activation: (() -> Void)?

    func begin(queryId: UInt64) {
        self.queryId = queryId
        settled = false
        cancel()
    }

    func request(_ action: @escaping () -> Void) {
        if settled {
            action()
        } else {
            activation = action
        }
    }

    func finish(queryId: UInt64) {
        guard self.queryId == queryId else { return }
        settled = true
        let action = activation
        cancel()
        action?()
    }

    func cancel() {
        activation = nil
    }
}
