use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    process::{Command, Stdio},
};

const PLUGIN_ID: &str = "io.clipkeep.web-search";
const MAX_QUERY_CHARACTERS: usize = 512;

fn main() -> io::Result<()> {
    run_plugin(WebSearchPlugin::default())
}

#[derive(Default)]
struct WebSearchPlugin {
    recent_urls: HashMap<String, String>,
}

impl Plugin for WebSearchPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_urls.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some((engine, terms)) = parse_search_command(query) else {
            return Ok(Vec::new());
        };
        if terms.chars().count() > MAX_QUERY_CHARACTERS {
            return Ok(Vec::new());
        }

        let encoded_terms = percent_encode(terms);
        let url = format!("{}{encoded_terms}", engine.url_prefix);
        let result_id = format!("web-search:{}:{encoded_terms}", engine.id);
        self.recent_urls.insert(result_id.clone(), url.clone());
        Ok(vec![search_result(engine, terms, result_id, url)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id != "open" {
            return Err(format!("unknown web search action: {action_id}"));
        }
        let url = self
            .recent_urls
            .get(result_id)
            .ok_or_else(|| "web search result is no longer active".to_owned())?;
        let status = Command::new("/usr/bin/open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not open the default browser: {error}"))?;
        if !status.success() {
            return Err(format!("default browser returned {status}"));
        }
        Ok(Some(format!(
            "Searching {}",
            engine_name_from_result_id(result_id)
        )))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SearchEngine {
    id: &'static str,
    name: &'static str,
    aliases: &'static [&'static str],
    url_prefix: &'static str,
    symbol: &'static str,
}

const ENGINES: &[SearchEngine] = &[
    SearchEngine {
        id: "google",
        name: "Google",
        aliases: &["g", "google", "web"],
        url_prefix: "https://www.google.com/search?q=",
        symbol: "magnifyingglass",
    },
    SearchEngine {
        id: "bing",
        name: "Bing",
        aliases: &["b", "bing"],
        url_prefix: "https://www.bing.com/search?q=",
        symbol: "magnifyingglass",
    },
    SearchEngine {
        id: "duckduckgo",
        name: "DuckDuckGo",
        aliases: &["d", "ddg", "duck", "duckduckgo"],
        url_prefix: "https://duckduckgo.com/?q=",
        symbol: "hand.raised.fill",
    },
    SearchEngine {
        id: "github",
        name: "GitHub",
        aliases: &["gh", "github"],
        url_prefix: "https://github.com/search?q=",
        symbol: "chevron.left.forwardslash.chevron.right",
    },
    SearchEngine {
        id: "youtube",
        name: "YouTube",
        aliases: &["yt", "youtube"],
        url_prefix: "https://www.youtube.com/results?search_query=",
        symbol: "play.rectangle.fill",
    },
    SearchEngine {
        id: "maps",
        name: "Google Maps",
        aliases: &["map", "maps"],
        url_prefix: "https://www.google.com/maps/search/?api=1&query=",
        symbol: "map.fill",
    },
];

fn parse_search_command(query: &str) -> Option<(&'static SearchEngine, &str)> {
    let query = query.trim();
    let whitespace = query.find(char::is_whitespace);
    let colon = query.find(':');
    let (command, terms) = match (colon, whitespace) {
        (Some(colon), Some(whitespace)) if colon < whitespace => {
            (&query[..colon], query[colon + 1..].trim())
        }
        (_, Some(whitespace)) => {
            let (command, terms) = query.split_at(whitespace);
            (command.trim_end_matches(':'), terms.trim())
        }
        (Some(colon), None) => (&query[..colon], query[colon + 1..].trim()),
        (None, None) => return None,
    };
    if terms.is_empty() {
        return None;
    }
    let command = command.to_ascii_lowercase();
    ENGINES
        .iter()
        .find(|engine| engine.aliases.contains(&command.as_str()))
        .map(|engine| (engine, terms))
}

fn search_result(
    engine: &SearchEngine,
    terms: &str,
    result_id: String,
    url: String,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: format!("Search {} for “{terms}”", engine.name),
        subtitle: url.clone(),
        preview: url.clone(),
        kind: ResultKind::Command,
        score: 2_800.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), engine.symbol.into()),
            ("url".into(), url),
        ]),
        actions: vec![ResultAction {
            host_action: None,
            id: "open".into(),
            title: "Search in Browser".into(),
            destructive: false,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn engine_name_from_result_id(result_id: &str) -> &'static str {
    let engine_id = result_id
        .strip_prefix("web-search:")
        .and_then(|value| value.split(':').next());
    ENGINES
        .iter()
        .find(|engine| Some(engine.id) == engine_id)
        .map(|engine| engine.name)
        .unwrap_or("the web")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_space_and_colon_commands_case_insensitively() {
        let (engine, terms) = parse_search_command("GH rust language").unwrap();
        assert_eq!(engine.id, "github");
        assert_eq!(terms, "rust language");

        let (engine, terms) = parse_search_command("maps:Shanghai Museum").unwrap();
        assert_eq!(engine.id, "maps");
        assert_eq!(terms, "Shanghai Museum");
    }

    #[test]
    fn ignores_unprefixed_and_empty_queries() {
        assert!(parse_search_command("ordinary words").is_none());
        assert!(parse_search_command("google ").is_none());
        assert!(parse_search_command("").is_none());
    }

    #[test]
    fn percent_encoding_handles_spaces_symbols_and_unicode() {
        assert_eq!(percent_encode("rust & swift"), "rust%20%26%20swift");
        assert_eq!(percent_encode("上海"), "%E4%B8%8A%E6%B5%B7");
    }

    #[test]
    fn search_produces_one_safe_browser_action() {
        let mut plugin = WebSearchPlugin::default();
        let results = plugin.search("g Rust plugins", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, PLUGIN_ID);
        assert_eq!(results[0].actions.len(), 1);
        assert!(results[0].actions[0].default);
        assert_eq!(
            results[0].metadata["url"],
            "https://www.google.com/search?q=Rust%20plugins"
        );
        assert_eq!(plugin.recent_urls.len(), 1);
    }

    #[test]
    fn subsequent_search_invalidates_old_result_ids() {
        let mut plugin = WebSearchPlugin::default();
        let old_id = plugin.search("g rust", 20).unwrap()[0].id.clone();
        plugin.search("yt swift", 20).unwrap();
        assert!(!plugin.recent_urls.contains_key(&old_id));
    }
}
