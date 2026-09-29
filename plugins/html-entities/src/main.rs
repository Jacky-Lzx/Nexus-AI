use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.html-entities";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;
const MAX_ENTITY_BODY_BYTES: usize = 32;

fn main() -> io::Result<()> {
    run_plugin(HtmlEntitiesPlugin::default())
}

#[derive(Default)]
struct HtmlEntitiesPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for HtmlEntitiesPlugin {
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
        let output = match request.operation.apply(&request.input) {
            Ok(output) => output,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!("html-entities:{}", request.operation.id());
        self.result_values.insert(result_id.clone(), output.clone());
        Ok(vec![entity_result(request.operation, result_id, output)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown HTML Entity Tools action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "HTML entity result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("HTML entity result copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EncodeMode {
    Minimal,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Encode(EncodeMode),
    Decode,
}

impl Operation {
    fn id(self) -> &'static str {
        match self {
            Self::Encode(EncodeMode::Minimal) => "encode",
            Self::Encode(EncodeMode::Ascii) => "encode-ascii",
            Self::Decode => "decode",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Encode(EncodeMode::Minimal) => "HTML Encode",
            Self::Encode(EncodeMode::Ascii) => "HTML Encode · ASCII-only",
            Self::Decode => "HTML Decode · strict semicolon syntax",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Encode(_) => "chevron.left.forwardslash.chevron.right",
            Self::Decode => "text.badge.checkmark",
        }
    }

    fn apply(self, input: &str) -> Result<String, String> {
        match self {
            Self::Encode(mode) => encode_html(input, mode),
            Self::Decode => decode_html(input),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    input: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let operation_kind = match command_lower.as_str() {
        "htmlencode" | "htmlenc" | "html-encode" => Some(true),
        "htmldecode" | "htmldec" | "html-decode" => Some(false),
        _ if command == "HTML编码" => Some(true),
        _ if command == "HTML解码" => Some(false),
        _ => None,
    };
    let Some(encode) = operation_kind else {
        return Ok(None);
    };
    if query.len() > MAX_INPUT_BYTES + 1024 {
        return Err("HTML entity query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let Some((option, input)) = remainder.split_once("::") else {
        return Err("expected '::' before the text to transform".into());
    };
    let option = option.trim();
    let operation = if encode {
        match option.to_ascii_lowercase().as_str() {
            "" | "minimal" => Operation::Encode(EncodeMode::Minimal),
            "ascii" | "ascii-only" => Operation::Encode(EncodeMode::Ascii),
            _ => return Err("encode mode must be minimal or ascii".into()),
        }
    } else {
        if !option.is_empty() {
            return Err("HTML decode does not accept a mode before '::'".into());
        }
        Operation::Decode
    };
    let input = input.strip_prefix(' ').unwrap_or(input);
    if input.is_empty() {
        return Err("text to transform cannot be empty".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("text to transform exceeds 64 KiB".into());
    }
    Ok(Some(Request {
        operation,
        input: input.into(),
    }))
}

fn encode_html(input: &str, mode: EncodeMode) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ if mode == EncodeMode::Ascii && !character.is_ascii() => {
                output.push_str(&format!("&#x{:X};", character as u32));
            }
            _ => output.push(character),
        }
        if output.len() > MAX_OUTPUT_BYTES {
            return Err("encoded text exceeds 512 KiB".into());
        }
    }
    Ok(output)
}

fn decode_html(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'&' {
            let character = input[index..]
                .chars()
                .next()
                .expect("index always points inside valid UTF-8");
            output.push(character);
            index += character.len_utf8();
            continue;
        }

        let scan_end = bytes.len().min(index + 1 + MAX_ENTITY_BODY_BYTES + 1);
        let semicolon = bytes[index + 1..scan_end]
            .iter()
            .position(|byte| *byte == b';')
            .map(|offset| index + 1 + offset);
        let Some(semicolon) = semicolon else {
            output.push('&');
            index += 1;
            continue;
        };
        let body = &input[index + 1..semicolon];
        let looks_like_entity = body.starts_with('#')
            || (!body.is_empty() && body.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        if !looks_like_entity {
            output.push('&');
            index += 1;
            continue;
        }
        let character = decode_entity(body, index)?;
        output.push(character);
        index = semicolon + 1;
    }
    Ok(output)
}

fn decode_entity(body: &str, byte_offset: usize) -> Result<char, String> {
    let named = match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00A0}'),
        "copy" => Some('©'),
        "reg" => Some('®'),
        "trade" => Some('™'),
        "hellip" => Some('…'),
        "ndash" => Some('–'),
        "mdash" => Some('—'),
        "laquo" => Some('«'),
        "raquo" => Some('»'),
        "middot" => Some('·'),
        "bull" => Some('•'),
        "euro" => Some('€'),
        _ => None,
    };
    if let Some(character) = named {
        return Ok(character);
    }
    if let Some(numeric) = body.strip_prefix('#') {
        let (radix, digits) = numeric
            .strip_prefix(['x', 'X'])
            .map_or((10, numeric), |digits| (16, digits));
        let valid_digits = match radix {
            10 => digits.bytes().all(|byte| byte.is_ascii_digit()),
            16 => digits.bytes().all(|byte| byte.is_ascii_hexdigit()),
            _ => unreachable!("numeric entities only use decimal or hexadecimal"),
        };
        if digits.is_empty() || !valid_digits {
            return Err(format!(
                "invalid numeric character reference at byte {byte_offset}"
            ));
        }
        let value = u32::from_str_radix(digits, radix).map_err(|_| {
            format!("numeric character reference is out of range at byte {byte_offset}")
        })?;
        if value == 0 {
            return Err(format!(
                "numeric character reference cannot encode NUL at byte {byte_offset}"
            ));
        }
        return char::from_u32(value).ok_or_else(|| {
            format!("numeric character reference is not a Unicode scalar at byte {byte_offset}")
        });
    }
    Err(format!(
        "unsupported named entity '&{body};' at byte {byte_offset}"
    ))
}

fn entity_result(operation: Operation, result_id: String, output: String) -> SearchResult {
    let visible = safe_text(&compact(&output, MAX_VISIBLE_CHARACTERS));
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&safe_text(&output), 180),
        subtitle: operation.label().into(),
        preview: format!("{}\n\nResult\n{visible}", operation.label()),
        kind: ResultKind::Command,
        score: 3_430.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), operation.symbol().into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Result", true, true, true),
            action("copy", "Copy Result", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "html-entities:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "HTML entity transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!(
            "HTML entity transformation unavailable\n{}",
            safe_text(message)
        ),
        kind: ResultKind::Command,
        score: 3_430.0,
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
        host_action: None,
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

fn safe_text(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_control()
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
    fn parses_explicit_commands_modes_and_nonmatching_queries() {
        assert!(parse_request("ordinary HTML text").unwrap().is_none());
        let minimal = parse_request("htmlencode :: <p>").unwrap().unwrap();
        assert_eq!(minimal.operation, Operation::Encode(EncodeMode::Minimal));
        assert_eq!(minimal.input, "<p>");
        let ascii = parse_request("htmlenc ascii :: café").unwrap().unwrap();
        assert_eq!(ascii.operation, Operation::Encode(EncodeMode::Ascii));
        let decode = parse_request("HTML解码: :: &amp;").unwrap().unwrap();
        assert_eq!(decode.operation, Operation::Decode);
    }

    #[test]
    fn minimally_encodes_html_sensitive_characters() {
        assert_eq!(
            encode_html("<p title=\"Tom's\">A & B ©</p>", EncodeMode::Minimal).unwrap(),
            "&lt;p title=&quot;Tom&#39;s&quot;&gt;A &amp; B ©&lt;/p&gt;"
        );
    }

    #[test]
    fn ascii_mode_encodes_every_non_ascii_scalar() {
        assert_eq!(
            encode_html("café 東京 😀", EncodeMode::Ascii).unwrap(),
            "caf&#xE9; &#x6771;&#x4EAC; &#x1F600;"
        );
    }

    #[test]
    fn decodes_common_named_entities() {
        assert_eq!(
            decode_html("&lt;b&gt;Tom &amp; Jerry&lt;/b&gt; &copy; &euro;").unwrap(),
            "<b>Tom & Jerry</b> © €"
        );
        assert_eq!(
            decode_html("a&nbsp;b&mdash;c&hellip;").unwrap(),
            "a\u{a0}b—c…"
        );
    }

    #[test]
    fn decodes_decimal_and_hex_unicode_references() {
        assert_eq!(decode_html("&#65; &#x1F600; &#X6771;").unwrap(), "A 😀 東");
    }

    #[test]
    fn decoding_is_one_pass_and_preserves_literal_ampersands() {
        assert_eq!(
            decode_html("&amp;lt; AT&T A & B").unwrap(),
            "&lt; AT&T A & B"
        );
    }

    #[test]
    fn rejects_unknown_malformed_and_non_scalar_references() {
        assert!(decode_html("&unknown;").is_err());
        assert!(decode_html("&#xZZ;").is_err());
        assert!(decode_html("&#0;").is_err());
        assert!(decode_html("&#xD800;").is_err());
        assert!(decode_html("&#x110000;").is_err());
    }

    #[test]
    fn rejects_bad_shapes_empty_and_oversized_inputs() {
        assert!(parse_request("htmlencode text").is_err());
        assert!(parse_request("htmlencode unknown :: text").is_err());
        assert!(parse_request("htmldecode ascii :: text").is_err());
        assert!(parse_request("htmldecode ::").is_err());
        assert!(
            parse_request(&format!(
                "htmlencode :: {}",
                "x".repeat(MAX_INPUT_BYTES + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn results_are_actionable_and_searches_invalidate_stale_values() {
        let mut plugin = HtmlEntitiesPlugin::default();
        let result = plugin.search("htmlencode :: <tag>", 1).unwrap().remove(0);
        assert_eq!(result.id, "html-entities:encode");
        assert_eq!(result.title, "&lt;tag&gt;");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert_eq!(plugin.result_values[&result.id], "&lt;tag&gt;");

        let invalid = plugin
            .search("htmldecode :: &unknown;", 1)
            .unwrap()
            .remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
        assert!(plugin.search("htmlencode :: x", 0).unwrap().is_empty());
    }

    #[test]
    fn decoded_controls_are_escaped_only_in_visible_fields() {
        let mut plugin = HtmlEntitiesPlugin::default();
        let result = plugin
            .search("htmldecode :: before&#10;after&#x202E;", 1)
            .unwrap()
            .remove(0);
        assert_eq!(plugin.result_values[&result.id], "before\nafter\u{202e}");
        assert!(result.title.contains("\\u{000A}"));
        assert!(result.title.contains("\\u{202E}"));
        assert!(!result.title.contains('\u{202e}'));
    }

    #[test]
    fn entity_scanning_is_bounded_and_long_literals_survive() {
        let input = format!("start &{}; end", "a".repeat(MAX_ENTITY_BODY_BYTES + 1));
        assert_eq!(decode_html(&input).unwrap(), input);
    }
}
