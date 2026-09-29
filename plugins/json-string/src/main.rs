use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, fmt::Write as _, io};

const PLUGIN_ID: &str = "io.clipkeep.json-string";
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_LITERAL_BYTES: usize = MAX_TEXT_BYTES * 6 + 2;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(JsonStringPlugin::default())
}

#[derive(Default)]
struct JsonStringPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for JsonStringPlugin {
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
        let transformation = match transform(&request) {
            Ok(transformation) => transformation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!(
            "json-string:{}-{}-{}",
            request.operation.direction.id(),
            request.operation.form.id(),
            request.operation.unicode_mode.id()
        );
        self.result_values
            .insert(result_id.clone(), transformation.output.clone());
        Ok(vec![string_result(
            result_id,
            request.operation,
            transformation,
        )])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown JSON String Tools action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "JSON string result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("JSON string result copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Escape,
    Unescape,
}

impl Direction {
    fn id(self) -> &'static str {
        match self {
            Self::Escape => "escape",
            Self::Unescape => "unescape",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Escape => "JSON Escape",
            Self::Unescape => "JSON Unescape",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Literal,
    Fragment,
}

impl Form {
    fn id(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Fragment => "fragment",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Literal => "complete string literal",
            Self::Fragment => "string-content fragment",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnicodeMode {
    Native,
    Ascii,
}

impl UnicodeMode {
    fn id(self) -> &'static str {
        match self {
            Self::Native => "unicode",
            Self::Ascii => "ascii",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Native => "native Unicode",
            Self::Ascii => "ASCII-only Unicode escapes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Operation {
    direction: Direction,
    form: Form,
    unicode_mode: UnicodeMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    input: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transformation {
    output: String,
    input_characters: usize,
    input_bytes: usize,
    output_characters: usize,
    output_bytes: usize,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let direction = match command_lower.as_str() {
        "jsonescape" | "jsonquote" | "json-string-escape" => Some(Direction::Escape),
        "jsonunescape" | "jsonunquote" | "json-string-unescape" => Some(Direction::Unescape),
        _ if command == "JSON转义" => Some(Direction::Escape),
        _ if command == "JSON反转义" => Some(Direction::Unescape),
        _ => None,
    };
    let Some(direction) = direction else {
        return Ok(None);
    };
    if query.len() > MAX_LITERAL_BYTES + 1024 {
        return Err("JSON String Tools query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let Some((options, input)) = remainder.split_once("::") else {
        return Err("expected '::' before the text or JSON string".into());
    };
    let mut form = None;
    let mut unicode_mode = UnicodeMode::Native;
    let mut unicode_option_seen = false;
    for option in options.split_whitespace() {
        match option.to_ascii_lowercase().as_str() {
            "literal" => set_form(&mut form, Form::Literal)?,
            "fragment" | "content" => set_form(&mut form, Form::Fragment)?,
            "ascii" if direction == Direction::Escape => {
                if unicode_option_seen {
                    return Err("Unicode output mode was specified more than once".into());
                }
                unicode_option_seen = true;
                unicode_mode = UnicodeMode::Ascii;
            }
            "unicode" | "native" if direction == Direction::Escape => {
                if unicode_option_seen {
                    return Err("Unicode output mode was specified more than once".into());
                }
                unicode_option_seen = true;
            }
            "ascii" | "unicode" | "native" => {
                return Err("Unicode output modes only apply to JSON escaping".into());
            }
            _ => return Err(format!("unknown JSON string option '{option}'")),
        }
    }
    let operation = Operation {
        direction,
        form: form.unwrap_or(Form::Literal),
        unicode_mode,
    };
    let input = input.strip_prefix(' ').unwrap_or(input);
    let maximum = if direction == Direction::Escape {
        MAX_TEXT_BYTES
    } else {
        MAX_LITERAL_BYTES
    };
    if input.len() > maximum {
        return Err(if direction == Direction::Escape {
            "text to escape exceeds 64 KiB".into()
        } else {
            "JSON string input exceeds its 384 KiB bound".into()
        });
    }
    Ok(Some(Request {
        operation,
        input: input.into(),
    }))
}

fn set_form(current: &mut Option<Form>, form: Form) -> Result<(), String> {
    if current.replace(form).is_some() {
        return Err("JSON string form was specified more than once".into());
    }
    Ok(())
}

fn transform(request: &Request) -> Result<Transformation, String> {
    let output = match request.operation.direction {
        Direction::Escape => escape_json_string(
            &request.input,
            request.operation.form,
            request.operation.unicode_mode,
        )?,
        Direction::Unescape => unescape_json_string(&request.input, request.operation.form)?,
    };
    if output.len() > MAX_LITERAL_BYTES {
        return Err("JSON string result exceeds its bounded output size".into());
    }
    Ok(Transformation {
        input_characters: request.input.chars().count(),
        input_bytes: request.input.len(),
        output_characters: output.chars().count(),
        output_bytes: output.len(),
        output,
    })
}

fn escape_json_string(
    input: &str,
    form: Form,
    unicode_mode: UnicodeMode,
) -> Result<String, String> {
    let literal = serde_json::to_string(input).map_err(|error| error.to_string())?;
    let literal = if unicode_mode == UnicodeMode::Ascii {
        ascii_only_literal(&literal)
    } else {
        literal
    };
    Ok(match form {
        Form::Literal => literal,
        Form::Fragment => literal[1..literal.len() - 1].into(),
    })
}

fn ascii_only_literal(literal: &str) -> String {
    let mut output = String::with_capacity(literal.len());
    for character in literal.chars() {
        if character.is_ascii() {
            output.push(character);
            continue;
        }
        let scalar = character as u32;
        if scalar <= 0xFFFF {
            write!(output, "\\u{scalar:04X}").expect("writing to a string cannot fail");
        } else {
            let supplementary = scalar - 0x1_0000;
            let high = 0xD800 + (supplementary >> 10);
            let low = 0xDC00 + (supplementary & 0x3FF);
            write!(output, "\\u{high:04X}\\u{low:04X}").expect("writing to a string cannot fail");
        }
    }
    output
}

fn unescape_json_string(input: &str, form: Form) -> Result<String, String> {
    let literal = match form {
        Form::Literal => input.into(),
        Form::Fragment => format!("\"{input}\""),
    };
    let decoded: String =
        serde_json::from_str(&literal).map_err(|error| format!("invalid JSON string: {error}"))?;
    if decoded.len() > MAX_TEXT_BYTES {
        return Err("decoded JSON string exceeds 64 KiB".into());
    }
    Ok(decoded)
}

fn string_result(
    result_id: String,
    operation: Operation,
    transformation: Transformation,
) -> SearchResult {
    let visible = visible_text(&compact(&transformation.output, MAX_VISIBLE_CHARACTERS));
    let title = compact(&safe_text(&transformation.output), 180);
    let direction = operation.direction.label();
    let form = operation.form.label();
    let unicode = operation.unicode_mode.label();
    let mode = if operation.direction == Direction::Escape {
        format!("{form} · {unicode}")
    } else {
        form.into()
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{direction} · {mode}"),
        preview: format!(
            "{direction}\nForm: {form}\n{}Input: {} characters · {} bytes\nOutput: {} characters · {} bytes\n\nResult\n{visible}",
            if operation.direction == Direction::Escape {
                format!("Unicode: {unicode}\n")
            } else {
                String::new()
            },
            transformation.input_characters,
            transformation.input_bytes,
            transformation.output_characters,
            transformation.output_bytes
        ),
        kind: ResultKind::Command,
        score: 3_400.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "quote.opening".into()),
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
        id: "json-string:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "JSON string transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!(
            "JSON string transformation unavailable\n{}",
            safe_text(message)
        ),
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
    fn parses_commands_options_and_nonmatching_queries() {
        assert!(parse_request("JSON document").unwrap().is_none());
        let escape = parse_request("jsonescape ascii fragment :: café")
            .unwrap()
            .unwrap();
        assert_eq!(escape.operation.direction, Direction::Escape);
        assert_eq!(escape.operation.form, Form::Fragment);
        assert_eq!(escape.operation.unicode_mode, UnicodeMode::Ascii);
        let unescape = parse_request("JSON反转义: fragment :: hello\\nworld")
            .unwrap()
            .unwrap();
        assert_eq!(unescape.operation.direction, Direction::Unescape);
        assert_eq!(unescape.operation.form, Form::Fragment);
    }

    #[test]
    fn escapes_complete_json_literals() {
        assert_eq!(
            escape_json_string("line\n\"quote\"\\end", Form::Literal, UnicodeMode::Native).unwrap(),
            "\"line\\n\\\"quote\\\"\\\\end\""
        );
    }

    #[test]
    fn ascii_mode_escapes_bmp_and_supplementary_unicode() {
        assert_eq!(
            escape_json_string("café 東京 😀", Form::Literal, UnicodeMode::Ascii).unwrap(),
            "\"caf\\u00E9 \\u6771\\u4EAC \\uD83D\\uDE00\""
        );
        assert_eq!(
            escape_json_string("é", Form::Fragment, UnicodeMode::Ascii).unwrap(),
            "\\u00E9"
        );
    }

    #[test]
    fn unescapes_literals_surrogate_pairs_and_fragments() {
        assert_eq!(
            unescape_json_string(r#""line\n\u6771\uD83D\uDE00""#, Form::Literal).unwrap(),
            "line\n東😀"
        );
        assert_eq!(
            unescape_json_string(r#"tab\tquote\""#, Form::Fragment).unwrap(),
            "tab\tquote\""
        );
    }

    #[test]
    fn round_trips_controls_quotes_and_unicode() {
        let original = "zero\0\n\t\"\\東京😀";
        let literal = escape_json_string(original, Form::Literal, UnicodeMode::Native).unwrap();
        assert_eq!(
            unescape_json_string(&literal, Form::Literal).unwrap(),
            original
        );
        let fragment = escape_json_string(original, Form::Fragment, UnicodeMode::Ascii).unwrap();
        assert_eq!(
            unescape_json_string(&fragment, Form::Fragment).unwrap(),
            original
        );
    }

    #[test]
    fn rejects_non_strings_bad_escapes_and_unpaired_surrogates() {
        assert!(unescape_json_string("123", Form::Literal).is_err());
        assert!(unescape_json_string(r#""ok" trailing"#, Form::Literal).is_err());
        assert!(unescape_json_string(r#""\x20""#, Form::Literal).is_err());
        assert!(unescape_json_string(r#""\uD800""#, Form::Literal).is_err());
        assert!(unescape_json_string("actual\nnewline", Form::Fragment).is_err());
    }

    #[test]
    fn empty_strings_are_valid_and_missing_delimiters_are_not() {
        let escape = parse_request("jsonescape ::").unwrap().unwrap();
        assert_eq!(transform(&escape).unwrap().output, "\"\"");
        let fragment = parse_request("jsonunescape fragment ::").unwrap().unwrap();
        assert_eq!(transform(&fragment).unwrap().output, "");
        assert!(parse_request("jsonescape text").is_err());
    }

    #[test]
    fn rejects_duplicate_unknown_and_inapplicable_options() {
        assert!(parse_request("jsonescape ascii unicode :: text").is_err());
        assert!(parse_request("jsonescape fragment literal :: text").is_err());
        assert!(parse_request("jsonunescape ascii :: \"text\"").is_err());
        assert!(parse_request("jsonescape unknown :: text").is_err());
    }

    #[test]
    fn enforces_input_and_decoded_output_limits() {
        assert!(
            parse_request(&format!("jsonescape :: {}", "x".repeat(MAX_TEXT_BYTES + 1))).is_err()
        );
        let oversized_literal = format!("\"{}\"", "x".repeat(MAX_TEXT_BYTES + 1));
        assert!(unescape_json_string(&oversized_literal, Form::Literal).is_err());
    }

    #[test]
    fn results_are_actionable_and_searches_clear_stale_values() {
        let mut plugin = JsonStringPlugin::default();
        let result = plugin
            .search("jsonescape ascii :: café", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, "json-string:escape-literal-ascii");
        assert_eq!(plugin.result_values[&result.id], "\"caf\\u00E9\"");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);

        let invalid = plugin
            .search("jsonunescape :: not-a-string", 1)
            .unwrap()
            .remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
        assert!(plugin.search("jsonescape :: text", 0).unwrap().is_empty());
    }

    #[test]
    fn decoded_controls_and_bidi_are_safe_in_visible_fields() {
        let mut plugin = JsonStringPlugin::default();
        let result = plugin
            .search(r#"jsonunescape :: "line\nbidi\u202E""#, 1)
            .unwrap()
            .remove(0);
        assert_eq!(plugin.result_values[&result.id], "line\nbidi\u{202e}");
        assert!(result.title.contains("line\\nbidi"));
        assert!(result.title.contains("\\u{202E}"));
        assert!(result.preview.contains("line\nbidi\\u{202E}"));
        assert!(!result.preview.contains('\u{202e}'));
    }
}
