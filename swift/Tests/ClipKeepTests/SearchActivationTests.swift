import Testing
@testable import Nexus

@Test func opensFinalSelectionInsteadOfPreviousOrPartialResults() {
    let activation = SearchActivation()
    var selection = "previous query"
    var opened: String?
    activation.begin(queryId: 1)
    activation.request { opened = selection }
    selection = "fast plugin"
    #expect(opened == nil)
    selection = "final highest score"
    activation.finish(queryId: 1)
    #expect(opened == "final highest score")
}

@Test func waitsForFinalResultsAndOnlyRunsOnce() {
    let activation = SearchActivation()
    var opened = 0
    activation.begin(queryId: 1)
    activation.request { opened += 1 }
    activation.request { opened += 1 }
    #expect(opened == 0)
    activation.finish(queryId: 1)
    activation.finish(queryId: 1)
    #expect(opened == 1)
}

@Test func furtherTypingCancelsQueuedReturnAndIgnoresOldResponses() {
    let activation = SearchActivation()
    var opened = 0
    activation.begin(queryId: 1)
    activation.request { opened += 1 }
    activation.begin(queryId: 2)
    activation.finish(queryId: 1)
    #expect(opened == 0)
    activation.request { opened += 10 }
    #expect(opened == 0)
    activation.finish(queryId: 2)
    #expect(opened == 10)
}

@Test func closingOrTimingOutCancelsQueuedReturn() {
    let activation = SearchActivation()
    var opened = false
    activation.begin(queryId: 1)
    activation.request { opened = true }
    activation.cancel()
    activation.finish(queryId: 1)
    #expect(!opened)
}

@Test func settledSearchActivatesImmediately() {
    let activation = SearchActivation()
    var opened = false
    activation.begin(queryId: 1)
    activation.finish(queryId: 1)
    activation.request { opened = true }
    #expect(opened)
}

@Test func activationMayStartAnotherSearch() {
    let activation = SearchActivation()
    var opened = 0
    activation.begin(queryId: 1)
    activation.request {
        activation.begin(queryId: 2)
        activation.request { opened += 1 }
    }
    activation.finish(queryId: 1)
    #expect(opened == 0)
    activation.finish(queryId: 2)
    #expect(opened == 1)
}
