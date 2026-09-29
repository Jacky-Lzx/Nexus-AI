use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use percent_encoding::percent_decode_str;
use std::{collections::BTreeMap, io};
use url::Url;

const PLUGIN_ID: &str = "io.clipkeep.url-inspector";
const RESULT_ID: &str = "url-inspector:result";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_QUERY_PARAMETERS: usize = 100;
const MAX_PREVIEW_CHARACTERS: usize = 240;
const MAX_URL_PREVIEW_CHARACTERS: usize = 1_024;

fn main() -> io::Result<()> {
    run_plugin(UrlInspectorPlugin::default())
}

#[derive(Default)]
struct UrlInspectorPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for UrlInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let input = match parse_request(query) {
            Ok(Some(input)) => input,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let inspection = match inspect_url(&input) {
            Ok(inspection) => inspection,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        self.remember_actions(&inspection);
        Ok(vec![inspection_result(inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("URL inspection is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive URL Inspector action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_url" | "copy_url" => "Normalized URL copied",
                "copy_host" => "URL host copied",
                "copy_path" => "URL path copied",
                "copy_query" => "URL query copied",
                "copy_fragment" => "URL fragment copied",
                _ => "URL component copied",
            }
            .into(),
        ))
    }
}

impl UrlInspectorPlugin {
    fn remember_actions(&mut self, inspection: &UrlInspection) {
        self.action_values
            .insert("paste_url".into(), inspection.normalized.clone());
        self.action_values
            .insert("copy_url".into(), inspection.normalized.clone());
        if let Some(host) = &inspection.host {
            self.action_values.insert("copy_host".into(), host.clone());
        }
        if !inspection.encoded_path.is_empty() {
            self.action_values
                .insert("copy_path".into(), inspection.encoded_path.clone());
        }
        if let Some(query) = &inspection.raw_query {
            self.action_values
                .insert("copy_query".into(), query.clone());
        }
        if let Some(fragment) = &inspection.raw_fragment {
            self.action_values
                .insert("copy_fragment".into(), fragment.clone());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UrlInspection {
    normalized: String,
    display_url: String,
    scheme: String,
    username: Option<String>,
    password_present: bool,
    host: Option<String>,
    explicit_port: Option<u16>,
    effective_port: Option<u16>,
    encoded_path: String,
    decoded_path: String,
    raw_query: Option<String>,
    query_parameters: Vec<(String, String)>,
    query_parameters_truncated: bool,
    raw_fragment: Option<String>,
    decoded_fragment: Option<String>,
    report: String,
}

fn parse_request(query: &str) -> Result<Option<String>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "url" | "uri") && command != "网址" {
        return Ok(None);
    }
    let input = query[command_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace());
    if input.is_empty() {
        return Err("expected a URL after 'url', 'uri', or '网址'".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("URL input exceeds 64 KiB".into());
    }
    Ok(Some(input.to_owned()))
}

fn inspect_url(input: &str) -> Result<UrlInspection, String> {
    let candidate = if has_explicit_scheme(input) {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let parsed = Url::parse(&candidate).map_err(|error| format!("invalid URL: {error}"))?;
    let normalized = parsed.as_str().to_owned();
    let password_present = parsed.password().is_some();
    let mut display = parsed.clone();
    if password_present {
        display
            .set_password(Some("REDACTED"))
            .map_err(|()| "could not redact URL credentials".to_owned())?;
    }
    let username = (!parsed.username().is_empty()).then(|| decode(parsed.username()));
    let host = parsed.host_str().map(str::to_owned);
    let encoded_path = parsed.path().to_owned();
    let decoded_path = decode(&encoded_path);
    let raw_query = parsed.query().map(str::to_owned);
    let mut query_parameters = Vec::new();
    let mut query_parameters_truncated = false;
    for (index, (key, value)) in parsed.query_pairs().enumerate() {
        if index == MAX_QUERY_PARAMETERS {
            query_parameters_truncated = true;
            break;
        }
        query_parameters.push((key.into_owned(), value.into_owned()));
    }
    let raw_fragment = parsed.fragment().map(str::to_owned);
    let decoded_fragment = raw_fragment.as_deref().map(decode);
    let mut inspection = UrlInspection {
        normalized,
        display_url: display.as_str().to_owned(),
        scheme: parsed.scheme().to_owned(),
        username,
        password_present,
        host,
        explicit_port: parsed.port(),
        effective_port: parsed.port_or_known_default(),
        encoded_path,
        decoded_path,
        raw_query,
        query_parameters,
        query_parameters_truncated,
        raw_fragment,
        decoded_fragment,
        report: String::new(),
    };
    inspection.report = build_report(&inspection);
    Ok(inspection)
}

fn has_explicit_scheme(input: &str) -> bool {
    if input.starts_with('[') || looks_like_host_port(input) {
        return false;
    }
    let Some(colon) = input.find(':') else {
        return false;
    };
    let scheme = &input[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn looks_like_host_port(input: &str) -> bool {
    let Some((host, _)) = input.split_once(':') else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host.contains('.')
        || host.parse::<std::net::Ipv4Addr>().is_ok()
}

fn build_report(inspection: &UrlInspection) -> String {
    let mut lines = vec![
        format!(
            "Normalized: {}",
            compact(&inspection.display_url, MAX_URL_PREVIEW_CHARACTERS)
        ),
        format!("Scheme: {}", inspection.scheme),
    ];
    if let Some(host) = &inspection.host {
        lines.push(format!("Host: {}", compact(host, MAX_PREVIEW_CHARACTERS)));
    }
    if let Some(port) = inspection.explicit_port {
        lines.push(format!("Port: {port}"));
    } else if let Some(port) = inspection.effective_port {
        lines.push(format!("Default port: {port}"));
    }
    if let Some(username) = &inspection.username {
        lines.push(format!(
            "Username: {}",
            compact(username, MAX_PREVIEW_CHARACTERS)
        ));
    }
    if inspection.password_present {
        lines.push("Password: present (hidden)".into());
    }
    if !inspection.encoded_path.is_empty() {
        lines.push(format!(
            "Path: {}",
            compact(&inspection.decoded_path, MAX_PREVIEW_CHARACTERS)
        ));
        if inspection.decoded_path != inspection.encoded_path {
            lines.push(format!(
                "Encoded path: {}",
                compact(&inspection.encoded_path, MAX_PREVIEW_CHARACTERS)
            ));
        }
    }
    if !inspection.query_parameters.is_empty() {
        lines.push(format!(
            "Query parameters ({}{}):",
            inspection.query_parameters.len(),
            if inspection.query_parameters_truncated {
                "+"
            } else {
                ""
            }
        ));
        for (key, value) in &inspection.query_parameters {
            lines.push(format!(
                "  {} = {}",
                compact(key, MAX_PREVIEW_CHARACTERS),
                compact(value, MAX_PREVIEW_CHARACTERS)
            ));
        }
        if inspection.query_parameters_truncated {
            lines.push(format!("  … limited to {MAX_QUERY_PARAMETERS} parameters"));
        }
    } else if inspection.raw_query.is_some() {
        lines.push("Query: present but empty".into());
    }
    if let Some(fragment) = &inspection.decoded_fragment {
        lines.push(format!(
            "Fragment: {}",
            compact(fragment, MAX_PREVIEW_CHARACTERS)
        ));
    }
    lines.join("\n")
}

fn inspection_result(inspection: UrlInspection) -> SearchResult {
    let host_label = inspection.host.as_deref().unwrap_or(&inspection.scheme);
    let parameter_label = match inspection.query_parameters.len() {
        0 => String::new(),
        1 => " · 1 query parameter".into(),
        count => format!(" · {count} query parameters"),
    };
    let credentials_label = if inspection.password_present {
        " · password hidden"
    } else {
        ""
    };
    let mut actions = vec![
        action("paste_url", "Paste Normalized URL", true, true),
        action("copy_url", "Copy Normalized URL", false, false),
    ];
    if inspection.host.is_some() {
        actions.push(action("copy_host", "Copy Host", false, false));
    }
    if !inspection.encoded_path.is_empty() {
        actions.push(action("copy_path", "Copy Encoded Path", false, false));
    }
    if inspection.raw_query.is_some() {
        actions.push(action("copy_query", "Copy Raw Query", false, false));
    }
    if inspection.raw_fragment.is_some() {
        actions.push(action("copy_fragment", "Copy Raw Fragment", false, false));
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!("URL · {}", compact(host_label, 160)),
        subtitle: format!(
            "{}{}{} · local inspection",
            inspection.scheme.to_ascii_uppercase(),
            parameter_label,
            credentials_label
        ),
        preview: inspection.report,
        kind: ResultKind::Command,
        score: 3_400.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "link".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "url-inspector:invalid".into(),
        title: "URL Inspector".into(),
        subtitle: format!("Invalid URL · {}", compact(message, 160)),
        preview: format!("Invalid URL\n{message}"),
        kind: ResultKind::Command,
        score: 3_400.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn action(id: &str, title: &str, default: bool, paste_to_previous: bool) -> ResultAction {
    ResultAction {
        host_action: None,
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window: paste_to_previous,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

fn decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn compact(value: &str, maximum_characters: usize) -> String {
    let mut characters = value.chars();
    let prefix: String = characters.by_ref().take(maximum_characters).collect();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_commands_and_rejects_unprefixed_text() {
        assert_eq!(
            parse_request("url: https://example.com").unwrap(),
            Some("https://example.com".into())
        );
        assert_eq!(
            parse_request("网址 example.cn").unwrap(),
            Some("example.cn".into())
        );
        assert_eq!(parse_request("visit example.com").unwrap(), None);
        assert!(parse_request("uri").is_err());
    }

    #[test]
    fn adds_https_for_hosts_and_host_ports() {
        let plain = inspect_url("example.com/a b").unwrap();
        assert_eq!(plain.normalized, "https://example.com/a%20b");
        let local = inspect_url("localhost:8080/status").unwrap();
        assert_eq!(local.scheme, "https");
        assert_eq!(local.host.as_deref(), Some("localhost"));
        assert_eq!(local.explicit_port, Some(8080));
        assert!(inspect_url("example.com:not-a-port/path").is_err());
    }

    #[test]
    fn reports_components_and_duplicate_query_parameters() {
        let inspected =
            inspect_url("https://alice:secret@example.com:8443/a%20b?q=one&q=two&empty=#part%201")
                .unwrap();
        assert_eq!(inspected.username.as_deref(), Some("alice"));
        assert!(inspected.password_present);
        assert_eq!(inspected.host.as_deref(), Some("example.com"));
        assert_eq!(inspected.explicit_port, Some(8443));
        assert_eq!(inspected.decoded_path, "/a b");
        assert_eq!(
            inspected.query_parameters,
            vec![
                ("q".into(), "one".into()),
                ("q".into(), "two".into()),
                ("empty".into(), "".into())
            ]
        );
        assert_eq!(inspected.decoded_fragment.as_deref(), Some("part 1"));
    }

    #[test]
    fn hides_passwords_from_visible_output_but_preserves_copy_value() {
        let inspected = inspect_url("https://user:hunter2@example.com/private").unwrap();
        assert!(!inspected.display_url.contains("hunter2"));
        assert!(!inspected.report.contains("hunter2"));
        assert!(inspected.display_url.contains("REDACTED"));
        assert!(inspected.normalized.contains("hunter2"));
    }

    #[test]
    fn supports_non_hierarchical_urls() {
        let inspected = inspect_url("mailto:user@example.com?subject=Hello%20World").unwrap();
        assert_eq!(inspected.scheme, "mailto");
        assert_eq!(inspected.host, None);
        assert_eq!(inspected.decoded_path, "user@example.com");
        assert_eq!(
            inspected.query_parameters,
            vec![("subject".into(), "Hello World".into())]
        );
    }

    #[test]
    fn search_result_has_component_actions_and_invalidates_old_values() {
        let mut plugin = UrlInspectorPlugin::default();
        let result = plugin
            .search("url https://example.com/path?a=1#top", 20)
            .unwrap()
            .remove(0);
        assert_eq!(result.title, "URL · example.com");
        assert!(result.preview.contains("Query parameters (1):"));
        assert_eq!(result.actions[0].id, "paste_url");
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert!(result.actions.iter().any(|action| action.id == "copy_host"));
        assert_eq!(plugin.action_values["copy_query"], "a=1");

        plugin.search("ordinary text", 20).unwrap();
        assert!(plugin.action_values.is_empty());
    }

    #[test]
    fn invalid_urls_are_visible_without_actions() {
        let mut plugin = UrlInspectorPlugin::default();
        let result = plugin.search("url https://[invalid", 20).unwrap().remove(0);
        assert!(result.subtitle.starts_with("Invalid URL · "));
        assert!(result.actions.is_empty());
        assert_eq!(result.metadata["symbol_color"], "#FF9F0A");
    }

    #[test]
    fn query_reports_are_bounded_and_compaction_is_unicode_safe() {
        let query = (0..105)
            .map(|index| format!("k{index}=v{index}"))
            .collect::<Vec<_>>()
            .join("&");
        let inspected = inspect_url(&format!("https://example.com/?{query}")).unwrap();
        assert_eq!(inspected.query_parameters.len(), 100);
        assert!(inspected.query_parameters_truncated);
        assert!(inspected.report.contains("limited to 100 parameters"));
        assert_eq!(compact("甲乙丙丁", 3), "甲乙丙…");

        let long = inspect_url(&format!("https://example.com/{}", "a".repeat(2_000))).unwrap();
        assert!(long.report.lines().next().unwrap().ends_with('…'));
        assert_eq!(long.normalized.len(), "https://example.com/".len() + 2_000);
    }
}
