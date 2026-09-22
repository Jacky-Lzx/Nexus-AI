use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io;

const PLUGIN_ID: &str = "io.clipkeep.query-string";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_COMPONENT_BYTES: usize = 16 * 1024;
const MAX_PAIRS: usize = 1_024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(QueryStringPlugin::default())
}

#[derive(Default)]
struct QueryStringPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for QueryStringPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.result_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let built_from_json = matches!(&request, Request::Build(_));
        let pairs = match request {
            Request::Parse(input) => match parse_query_string(&input) {
                Ok(pairs) => pairs,
                Err(message) => return Ok(vec![invalid_result(query, &message)]),
            },
            Request::Build(input) => match pairs_from_json(&input) {
                Ok(pairs) => pairs,
                Err(message) => return Ok(vec![invalid_result(query, &message)]),
            },
        };
        let unique_keys = pairs
            .iter()
            .map(|pair| pair.key.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let canonical = match canonical_query(&pairs) {
            Ok(canonical) => canonical,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let json = if built_from_json || limit == 1 {
            None
        } else {
            match pairs_to_json(&pairs) {
                Ok(json) => Some(json),
                Err(message) => return Ok(vec![invalid_result(query, &message)]),
            }
        };
        let mut results = Vec::new();
        let canonical_id = "query-string:canonical".to_owned();
        self.result_values
            .insert(canonical_id.clone(), canonical.clone());
        results.push(canonical_result(
            canonical_id,
            canonical,
            &pairs,
            unique_keys,
            built_from_json,
            3_410.0,
        ));

        if let Some(json) = json {
            let json_id = "query-string:json".to_owned();
            self.result_values.insert(json_id.clone(), json.clone());
            results.push(json_result(
                json_id,
                json,
                pairs.len(),
                unique_keys,
                3_409.0,
            ));
        }
        results.truncate(limit);
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Query String Tools action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "query string result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Query string result copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Request {
    Parse(String),
    Build(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pair {
    key: String,
    value: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let short_alias = command_lower == "qs";
    let build = match command_lower.as_str() {
        "querystring" | "queryparams" | "query-params" | "qs" => Some(false),
        "queryjson" | "qsjson" | "query-from-json" => Some(true),
        _ if command == "查询参数" => Some(false),
        _ if command == "查询参数JSON" => Some(true),
        _ => None,
    };
    let Some(build) = build else {
        return Ok(None);
    };
    if short_alias && !query.contains("::") {
        return Ok(None);
    }
    if query.len() > MAX_INPUT_BYTES + 1024 {
        return Err("Query String Tools input exceeds its bounded size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let Some((options, input)) = remainder.split_once("::") else {
        return Err("expected '::' before the query string or JSON object".into());
    };
    if !options.trim().is_empty() {
        return Err("this command does not accept options before '::'".into());
    }
    let input = input.strip_prefix(' ').unwrap_or(input);
    if input.is_empty() {
        return Err("query string or JSON object cannot be empty".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("query string or JSON object exceeds 64 KiB".into());
    }
    Ok(Some(if build {
        Request::Build(input.into())
    } else {
        Request::Parse(input.into())
    }))
}

fn parse_query_string(input: &str) -> Result<Vec<Pair>, String> {
    let input = input.strip_prefix('?').unwrap_or(input);
    if input.is_empty() {
        return Err("query string has no parameters".into());
    }
    let mut pairs = Vec::new();
    for (index, segment) in input.split('&').enumerate() {
        if segment.is_empty() {
            return Err(format!("empty query parameter at position {}", index + 1));
        }
        if pairs.len() == MAX_PAIRS {
            return Err(format!("query string exceeds {MAX_PAIRS} parameters"));
        }
        let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
        let key = form_decode(key)?;
        let value = form_decode(value)?;
        validate_component("key", &key)?;
        validate_component("value", &value)?;
        pairs.push(Pair { key, value });
    }
    Ok(pairs)
}

fn form_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(format!("incomplete percent escape at byte {index}"));
                }
                let high = hex_value(bytes[index + 1])
                    .ok_or_else(|| format!("invalid percent escape at byte {index}"))?;
                let low = hex_value(bytes[index + 2])
                    .ok_or_else(|| format!("invalid percent escape at byte {index}"))?;
                output.push((high << 4) | low);
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
        if output.len() > MAX_COMPONENT_BYTES {
            return Err(format!(
                "decoded query component exceeds {MAX_COMPONENT_BYTES} bytes"
            ));
        }
    }
    String::from_utf8(output).map_err(|_| "query component is not valid UTF-8".into())
}

fn validate_component(kind: &str, value: &str) -> Result<(), String> {
    if value.len() > MAX_COMPONENT_BYTES {
        return Err(format!("query parameter {kind} exceeds 16 KiB"));
    }
    Ok(())
}

fn canonical_query(pairs: &[Pair]) -> Result<String, String> {
    let mut output = String::new();
    for (index, pair) in pairs.iter().enumerate() {
        if index > 0 {
            output.push('&');
        }
        output.push_str(&percent_encode(pair.key.as_bytes()));
        output.push('=');
        output.push_str(&percent_encode(pair.value.as_bytes()));
        if output.len() > MAX_OUTPUT_BYTES {
            return Err("canonical query string exceeds 256 KiB".into());
        }
    }
    Ok(output)
}

fn pairs_to_json(pairs: &[Pair]) -> Result<String, String> {
    let mut grouped = BTreeMap::<&str, Vec<&str>>::new();
    for pair in pairs {
        grouped
            .entry(pair.key.as_str())
            .or_default()
            .push(pair.value.as_str());
    }
    let mut object = Map::new();
    for (key, values) in grouped {
        let value = if values.len() == 1 {
            Value::String(values[0].into())
        } else {
            Value::Array(
                values
                    .into_iter()
                    .map(|value| Value::String(value.into()))
                    .collect(),
            )
        };
        object.insert(key.into(), value);
    }
    let output =
        serde_json::to_string_pretty(&Value::Object(object)).map_err(|error| error.to_string())?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err("query parameter JSON exceeds 256 KiB".into());
    }
    Ok(output)
}

fn pairs_from_json(input: &str) -> Result<Vec<Pair>, String> {
    let value: Value =
        serde_json::from_str(input).map_err(|error| format!("invalid JSON: {error}"))?;
    let Value::Object(object) = value else {
        return Err("query JSON must be an object".into());
    };
    if object.is_empty() {
        return Err("query JSON object cannot be empty".into());
    }
    let mut pairs = Vec::new();
    for (key, value) in object {
        validate_component("key", &key)?;
        match value {
            Value::Array(values) => {
                if values.is_empty() {
                    return Err(format!("array for query key '{key}' cannot be empty"));
                }
                for value in values {
                    let value = json_scalar(value, &key)?;
                    push_json_pair(&mut pairs, &key, value)?;
                }
            }
            value => {
                let value = json_scalar(value, &key)?;
                push_json_pair(&mut pairs, &key, value)?;
            }
        }
    }
    Ok(pairs)
}

fn json_scalar(value: Value, key: &str) -> Result<String, String> {
    match value {
        Value::Null => Ok(String::new()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => Ok(value),
        Value::Array(_) | Value::Object(_) => Err(format!(
            "query key '{key}' contains a nested array or object"
        )),
    }
}

fn push_json_pair(pairs: &mut Vec<Pair>, key: &str, value: String) -> Result<(), String> {
    if pairs.len() == MAX_PAIRS {
        return Err(format!(
            "query JSON exceeds {MAX_PAIRS} generated parameters"
        ));
    }
    validate_component("value", &value)?;
    pairs.push(Pair {
        key: key.into(),
        value,
    });
    Ok(())
}

fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(*byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0x0F) as usize]));
        }
    }
    output
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn canonical_result(
    result_id: String,
    value: String,
    pairs: &[Pair],
    unique_keys: usize,
    built_from_json: bool,
    score: f32,
) -> SearchResult {
    let pair_count = pairs.len();
    let source = if built_from_json {
        "Built from JSON"
    } else {
        "Canonical RFC 3986 query"
    };
    let report = decoded_report(pairs);
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&value, 180),
        subtitle: format!(
            "{source} · {pair_count} parameter{} · {unique_keys} unique key{}",
            plural(pair_count),
            plural(unique_keys)
        ),
        preview: format!(
            "{source}\nParameters: {pair_count}\nUnique keys: {unique_keys}\n\nDecoded parameters\n{report}\n\nCanonical query\n{}",
            compact(&value, MAX_VISIBLE_CHARACTERS)
        ),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "link".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Canonical Query", true, true, true),
            action("copy", "Copy Canonical Query", false, false, false),
        ],
    }
}

fn json_result(
    result_id: String,
    value: String,
    pair_count: usize,
    unique_keys: usize,
    score: f32,
) -> SearchResult {
    let visible = visible_text(&compact(&value, MAX_VISIBLE_CHARACTERS));
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&safe_text(&value), 180),
        subtitle: format!(
            "Query parameters as JSON · {pair_count} parameter{} · {unique_keys} unique key{}",
            plural(pair_count),
            plural(unique_keys)
        ),
        preview: format!("Query parameters as JSON\n\n{visible}"),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "curlybraces".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste JSON", true, true, true),
            action("copy", "Copy JSON", false, false, false),
        ],
    }
}

fn decoded_report(pairs: &[Pair]) -> String {
    let mut report = String::new();
    for (index, pair) in pairs.iter().enumerate() {
        let key = safe_text(&pair.key);
        let value = safe_text(&pair.value);
        let _ = writeln!(report, "{}. {key} = {value}", index + 1);
        if report.chars().count() > MAX_VISIBLE_CHARACTERS {
            return compact(&report, MAX_VISIBLE_CHARACTERS);
        }
    }
    report.trim_end().into()
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "query-string:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Query string transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!(
            "Query string transformation unavailable\n{}",
            safe_text(message)
        ),
        kind: ResultKind::Command,
        score: 3_410.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn action(
    id: &str,
    title: &str,
    default: bool,
    closes_window: bool,
    paste_to_previous: bool,
) -> ResultAction {
    ResultAction {
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn safe_text(value: &str) -> String {
    visible_text(value)
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn visible_text(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if matches!(character, '\n' | '\t') {
            output.push(character);
        } else if character.is_control()
            || matches!(
                character,
                '\u{061C}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
            )
        {
            output.push_str(&format!("\\u{{{:04X}}}", character as u32));
        } else {
            output.push(character);
        }
    }
    output
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
    fn parses_explicit_commands_and_nonstealing_short_alias() {
        assert!(parse_request("ordinary search").unwrap().is_none());
        assert!(parse_request("qs weekly report").unwrap().is_none());
        assert_eq!(
            parse_request("qs :: a=1").unwrap().unwrap(),
            Request::Parse("a=1".into())
        );
        assert_eq!(
            parse_request("queryjson: :: {\"a\":1}").unwrap().unwrap(),
            Request::Build("{\"a\":1}".into())
        );
    }

    #[test]
    fn parses_form_encoding_missing_equals_and_empty_keys() {
        let pairs = parse_query_string("?name=Alice+Smith&flag&=empty-key").unwrap();
        assert_eq!(
            pairs,
            [
                Pair {
                    key: "name".into(),
                    value: "Alice Smith".into()
                },
                Pair {
                    key: "flag".into(),
                    value: "".into()
                },
                Pair {
                    key: "".into(),
                    value: "empty-key".into()
                }
            ]
        );
    }

    #[test]
    fn canonicalizes_spaces_unicode_and_reserved_characters() {
        let pairs = parse_query_string("q=hello+世界&next=%2Fdocs%3Fa%3D1").unwrap();
        assert_eq!(
            canonical_query(&pairs).unwrap(),
            "q=hello%20%E4%B8%96%E7%95%8C&next=%2Fdocs%3Fa%3D1"
        );
    }

    #[test]
    fn json_groups_repeated_keys_without_losing_values() {
        let pairs = parse_query_string("tag=rust&name=Nexus&tag=mac").unwrap();
        let json = pairs_to_json(&pairs).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["name"], "Nexus");
        assert_eq!(value["tag"], serde_json::json!(["rust", "mac"]));
    }

    #[test]
    fn builds_pairs_from_json_scalars_arrays_and_null() {
        let pairs =
            pairs_from_json(r#"{"empty":null,"enabled":true,"page":2,"tag":["rust","mac"]}"#)
                .unwrap();
        assert_eq!(
            canonical_query(&pairs).unwrap(),
            "empty=&enabled=true&page=2&tag=rust&tag=mac"
        );
    }

    #[test]
    fn query_json_round_trips_grouped_query_semantics() {
        let original = parse_query_string("a=1&a=2&b=hello+world").unwrap();
        let json = pairs_to_json(&original).unwrap();
        let rebuilt = pairs_from_json(&json).unwrap();
        assert_eq!(
            canonical_query(&rebuilt).unwrap(),
            "a=1&a=2&b=hello%20world"
        );
    }

    #[test]
    fn rejects_malformed_escapes_empty_segments_and_non_utf8() {
        assert!(parse_query_string("a=%").is_err());
        assert!(parse_query_string("a=%GG").is_err());
        assert!(parse_query_string("a=%FF").is_err());
        assert!(parse_query_string("a=1&&b=2").is_err());
        assert!(parse_query_string("?").is_err());
    }

    #[test]
    fn rejects_nested_empty_and_oversized_json_shapes() {
        assert!(pairs_from_json("[]").is_err());
        assert!(pairs_from_json("{}").is_err());
        assert!(pairs_from_json(r#"{"a":[]}"#).is_err());
        assert!(pairs_from_json(r#"{"a":{"nested":true}}"#).is_err());
        assert!(
            pairs_from_json(&format!(
                r#"{{"a":"{}"}}"#,
                "x".repeat(MAX_COMPONENT_BYTES + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn enforces_parameter_and_input_limits() {
        let too_many = std::iter::repeat_n("a=b", MAX_PAIRS + 1)
            .collect::<Vec<_>>()
            .join("&");
        assert!(parse_query_string(&too_many).is_err());
        assert!(
            parse_request(&format!(
                "querystring :: {}",
                "x".repeat(MAX_INPUT_BYTES + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn searches_return_bounded_actions_and_clear_stale_values() {
        let mut plugin = QueryStringPlugin::default();
        let results = plugin
            .search("querystring :: tag=rust&tag=mac&name=Nexus", 8)
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "query-string:canonical");
        assert_eq!(results[1].id, "query-string:json");
        assert_eq!(results[0].actions.len(), 2);
        assert!(results[0].actions[0].default);
        assert!(results[1].preview.contains("\n  \"name\""));
        assert_eq!(plugin.result_values.len(), 2);

        let limited = plugin.search("querystring :: a=1", 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(plugin.result_values.len(), 1);
        let invalid = plugin.search("querystring :: a=%GG", 2).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
    }

    #[test]
    fn visible_reports_escape_controls_and_directional_overrides() {
        let mut plugin = QueryStringPlugin::default();
        let results = plugin
            .search("querystring :: key=line%0Abidi%E2%80%AE", 2)
            .unwrap();
        assert!(results[0].preview.contains("line\\nbidi"));
        assert!(results[0].preview.contains("\\u{202E}"));
        assert!(!results[0].preview.contains('\u{202e}'));
        assert!(results[1].title.contains("\\u{202E}"));
    }
}
