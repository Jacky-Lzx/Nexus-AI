import Testing
@testable import Nexus

private func rankingResult(_ title: String, score: Double) -> SearchResult {
    SearchResult(pluginId: "test", id: title, title: title, subtitle: "",
                 preview: "", kind: .command, score: score, metadata: [:], actions: [])
}

@Test func sleepCommandRanksAbovePreventSleepKeywordMatch() {
    let results = [rankingResult("Caffeinate", score: 4000),
                   rankingResult("Turn Display Off", score: 1999),
                   rankingResult("Sleep Mac", score: 1988)]
    let titles = SearchRanking.sorted(results, query: "sleep").map(\.title)
    #expect(titles == ["Sleep Mac", "Caffeinate", "Turn Display Off"])
}

@Test func exactTitleOutranksPrefixAndSubstringRegardlessOfPluginScore() {
    let results = [rankingResult("Caffeinate", score: 4000),
                   rankingResult("Display Sleep", score: 9000),
                   rankingResult("Sleep Mac", score: 2000),
                   rankingResult("Sleep", score: 1)]
    #expect(SearchRanking.sorted(results, query: "  SLEEP \n").map(\.title)
            == ["Sleep", "Sleep Mac", "Display Sleep", "Caffeinate"])
}

@Test func emptyQueryAndEqualMatchQualityPreservePluginScores() {
    let results = [rankingResult("Low", score: 1), rankingResult("High", score: 2)]
    #expect(SearchRanking.sorted(results, query: "  ").map(\.title) == ["High", "Low"])
    #expect(SearchRanking.sorted(results, query: "keyword").map(\.title) == ["High", "Low"])
}

@Test func caffeinateExactMatchStillWins() {
    let results = [rankingResult("Caffeinate", score: 4000), rankingResult("Sleep Mac", score: 9000)]
    #expect(SearchRanking.sorted(results, query: "caffeinate").first?.title == "Caffeinate")
}
