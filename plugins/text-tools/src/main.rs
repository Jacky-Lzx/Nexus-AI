use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use ring::digest::{SHA256, digest};
use std::{collections::BTreeMap, collections::HashMap, fmt::Write as _, io};

const PLUGIN_ID: &str = "io.clipkeep.text-tools";
const MAX_INPUT_BYTES: usize = 64 * 1024;

fn main() -> io::Result<()> {
    run_plugin(TextToolsPlugin::default())
}

#[derive(Default)]
struct TextToolsPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for TextToolsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some((operation, input)) = parse_command(query) else {
            return Ok(Vec::new());
        };
        if input.len() > MAX_INPUT_BYTES {
            return Ok(Vec::new());
        }
        let Ok(output) = operation.apply(input) else {
            return Ok(Vec::new());
        };
        let result_id = format!("text-tools:{}", operation.id());
        self.recent_values.insert(result_id.clone(), output.clone());
        Ok(vec![text_result(operation, result_id, output)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown text tools action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "text result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Text copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Uppercase,
    Lowercase,
    Trim,
    UrlEncode,
    UrlDecode,
    Base64Encode,
    Base64Decode,
    JsonPretty,
    Sha256,
    Count,
}

impl Operation {
    const ALL: [Self; 10] = [
        Self::Uppercase,
        Self::Lowercase,
        Self::Trim,
        Self::UrlEncode,
        Self::UrlDecode,
        Self::Base64Encode,
        Self::Base64Decode,
        Self::JsonPretty,
        Self::Sha256,
        Self::Count,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Uppercase => "uppercase",
            Self::Lowercase => "lowercase",
            Self::Trim => "trim",
            Self::UrlEncode => "url-encode",
            Self::UrlDecode => "url-decode",
            Self::Base64Encode => "base64-encode",
            Self::Base64Decode => "base64-decode",
            Self::JsonPretty => "json-pretty",
            Self::Sha256 => "sha256",
            Self::Count => "count",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Uppercase => "Uppercase",
            Self::Lowercase => "Lowercase",
            Self::Trim => "Trim Whitespace",
            Self::UrlEncode => "URL Encode",
            Self::UrlDecode => "URL Decode",
            Self::Base64Encode => "Base64 Encode",
            Self::Base64Decode => "Base64 Decode",
            Self::JsonPretty => "Format JSON",
            Self::Sha256 => "SHA-256",
            Self::Count => "Text Statistics",
        }
    }

    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Uppercase => &["upper", "uppercase"],
            Self::Lowercase => &["lower", "lowercase"],
            Self::Trim => &["trim", "strip"],
            Self::UrlEncode => &["urlencode", "ue"],
            Self::UrlDecode => &["urldecode", "ud"],
            Self::Base64Encode => &["base64", "b64"],
            Self::Base64Decode => &["unbase64", "db64"],
            Self::JsonPretty => &["json", "prettyjson"],
            Self::Sha256 => &["sha", "sha256"],
            Self::Count => &["count", "wc"],
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Uppercase | Self::Lowercase => "textformat",
            Self::Trim => "text.alignleft",
            Self::UrlEncode | Self::UrlDecode => "link",
            Self::Base64Encode | Self::Base64Decode => "lock.doc",
            Self::JsonPretty => "curlybraces",
            Self::Sha256 => "number",
            Self::Count => "character.cursor.ibeam",
        }
    }

    fn apply(self, input: &str) -> Result<String, String> {
        match self {
            Self::Uppercase => Ok(input.to_uppercase()),
            Self::Lowercase => Ok(input.to_lowercase()),
            Self::Trim => Ok(input.trim().to_owned()),
            Self::UrlEncode => Ok(percent_encode(input)),
            Self::UrlDecode => percent_decode(input),
            Self::Base64Encode => Ok(BASE64.encode(input)),
            Self::Base64Decode => {
                let compact: String = input
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect();
                let bytes = BASE64
                    .decode(compact)
                    .map_err(|error| format!("invalid Base64: {error}"))?;
                String::from_utf8(bytes).map_err(|_| "decoded Base64 is not UTF-8 text".into())
            }
            Self::JsonPretty => {
                let value: serde_json::Value = serde_json::from_str(input)
                    .map_err(|error| format!("invalid JSON: {error}"))?;
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
            }
            Self::Sha256 => {
                let hash = digest(&SHA256, input.as_bytes());
                let mut encoded = String::with_capacity(hash.as_ref().len() * 2);
                for byte in hash.as_ref() {
                    write!(encoded, "{byte:02x}").expect("writing to a string cannot fail");
                }
                Ok(encoded)
            }
            Self::Count => Ok(format!(
                "{} words · {} characters · {} bytes",
                input.split_whitespace().count(),
                input.chars().count(),
                input.len()
            )),
        }
    }
}

fn parse_command(query: &str) -> Option<(Operation, &str)> {
    let query = query.trim_start();
    let whitespace = query.find(char::is_whitespace);
    let colon = query.find(':');
    let (command, input) = match (colon, whitespace) {
        (Some(colon), Some(whitespace)) if colon < whitespace => {
            (&query[..colon], &query[colon + 1..])
        }
        (_, Some(whitespace)) => {
            let (command, input) = query.split_at(whitespace);
            (command.trim_end_matches(':'), input.trim_start())
        }
        (Some(colon), None) => (&query[..colon], &query[colon + 1..]),
        (None, None) => return None,
    };
    let command = command.to_ascii_lowercase();
    let operation = Operation::ALL
        .into_iter()
        .find(|operation| operation.aliases().contains(&command.as_str()))?;
    let input = if operation == Operation::Trim {
        input
    } else {
        input.trim()
    };
    (!input.is_empty()).then_some((operation, input))
}

fn text_result(operation: Operation, result_id: String, output: String) -> SearchResult {
    let multiline = output.contains(['\n', '\r']);
    let mut metadata = BTreeMap::from([("symbol_name".into(), operation.symbol().into())]);
    if multiline {
        metadata.insert("preview_style".into(), "text".into());
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact_title(&output, multiline),
        subtitle: operation.title().into(),
        preview: output,
        kind: ResultKind::Command,
        score: 3_000.0,
        metadata,
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste Result".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy Result".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

fn compact_title(value: &str, multiline: bool) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title: String = collapsed.chars().take(80).collect();
    if collapsed.chars().count() > 80 {
        title.push('…');
    }
    if title.is_empty() {
        title.push_str("(empty)");
    }
    if multiline {
        title.push_str(" ↩");
    }
    title
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

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err("incomplete percent escape".into());
        }
        let high =
            hex_value(bytes[index + 1]).ok_or_else(|| "invalid percent escape".to_owned())?;
        let low = hex_value(bytes[index + 2]).ok_or_else(|| "invalid percent escape".to_owned())?;
        decoded.push(high << 4 | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| "decoded URL value is not UTF-8 text".into())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_space_and_colon_commands_without_matching_plain_text() {
        assert_eq!(
            parse_command("upper Hello"),
            Some((Operation::Uppercase, "Hello"))
        );
        assert_eq!(
            parse_command("json:{\"a\":1}"),
            Some((Operation::JsonPretty, "{\"a\":1}"))
        );
        assert_eq!(parse_command("ordinary text"), None);
    }

    #[test]
    fn trim_receives_original_whitespace_after_a_colon() {
        let (operation, input) = parse_command("trim:   hello world   ").unwrap();
        assert_eq!(operation, Operation::Trim);
        assert_eq!(input, "   hello world   ");
        assert_eq!(operation.apply(input).unwrap(), "hello world");
    }

    #[test]
    fn transforms_unicode_case_and_url_values() {
        assert_eq!(
            Operation::Uppercase.apply("Hello 世界").unwrap(),
            "HELLO 世界"
        );
        let encoded = Operation::UrlEncode.apply("上海 & Rust").unwrap();
        assert_eq!(encoded, "%E4%B8%8A%E6%B5%B7%20%26%20Rust");
        assert_eq!(Operation::UrlDecode.apply(&encoded).unwrap(), "上海 & Rust");
        assert!(Operation::UrlDecode.apply("bad%2").is_err());
    }

    #[test]
    fn base64_round_trips_text_and_rejects_binary() {
        let encoded = Operation::Base64Encode.apply("Nexus 插件").unwrap();
        assert_eq!(
            Operation::Base64Decode.apply(&encoded).unwrap(),
            "Nexus 插件"
        );
        assert!(Operation::Base64Decode.apply("/w==").is_err());
    }

    #[test]
    fn formats_json_and_computes_sha256() {
        assert_eq!(
            Operation::JsonPretty.apply("{\"b\":2,\"a\":1}").unwrap(),
            "{\n  \"a\": 1,\n  \"b\": 2\n}"
        );
        assert_eq!(
            Operation::Sha256.apply("abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn reports_words_unicode_characters_and_utf8_bytes() {
        assert_eq!(
            Operation::Count.apply("hello 世界").unwrap(),
            "2 words · 8 characters · 12 bytes"
        );
    }

    #[test]
    fn multiline_results_request_text_preview_and_safe_actions() {
        let mut plugin = TextToolsPlugin::default();
        let result = plugin.search("json {\"a\":1}", 20).unwrap().remove(0);
        assert_eq!(result.metadata["preview_style"], "text");
        assert!(result.title.ends_with(" ↩"));
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values[&result.id], result.preview);
    }
}
