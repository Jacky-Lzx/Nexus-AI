import Foundation

/// Plugin scores break ties only after the user's query has been matched to titles.
enum SearchRanking {
    static func sorted(_ results: [SearchResult], query: String) -> [SearchResult] {
        let query = normalized(query)
        return results.sorted {
            let lhs = titleMatch($0.title, query: query)
            let rhs = titleMatch($1.title, query: query)
            if lhs != rhs { return lhs > rhs }
            if $0.score != $1.score { return $0.score > $1.score }
            if $0.title != $1.title { return $0.title < $1.title }
            if $0.pluginId != $1.pluginId { return $0.pluginId < $1.pluginId }
            return $0.id < $1.id
        }
    }

    private static func normalized(_ value: String) -> String {
        value.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
            .folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "en_US_POSIX"))
    }

    private static func titleMatch(_ title: String, query: String) -> Int {
        guard !query.isEmpty else { return 0 }
        let title = normalized(title)
        if title == query { return 3 }
        if title.hasPrefix(query) { return 2 }
        if title.contains(query) { return 1 }
        return 0
    }
}
