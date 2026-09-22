use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.unicode-inspector";
const MAX_ARGUMENT_BYTES: usize = 256;
const MAX_SCALARS: usize = 64;

fn main() -> io::Result<()> {
    run_plugin(UnicodeInspectorPlugin::default())
}

#[derive(Default)]
struct UnicodeInspectorPlugin {
    active_values: BTreeMap<(String, String), String>,
}

impl Plugin for UnicodeInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let parsed = match parse_input(&request.value) {
            Ok(parsed) => parsed,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let inspection = inspect_text(parsed);
        let report = build_report(&inspection);
        let entries = result_entries(&inspection, &report);
        Ok(entries
            .into_iter()
            .take(limit)
            .map(|entry| {
                for (action_id, value) in entry.values {
                    self.active_values
                        .insert((entry.result.id.clone(), action_id), value);
                }
                entry.result
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let value = self
            .active_values
            .get(&(result_id.to_owned(), action_id.to_owned()))
            .ok_or_else(|| "Unicode inspection result or action is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Unicode value copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedInput {
    text: String,
    decoded_notation: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inspection {
    text: String,
    decoded_notation: bool,
    utf8_bytes: usize,
    utf16_units: usize,
    rust_escape: String,
    json_escape: String,
    scalars: Vec<ScalarInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScalarInfo {
    character: char,
    display: String,
    codepoint: String,
    decimal: u32,
    utf8: String,
    utf16: String,
    rust_escape: String,
    json_escape: String,
    properties: String,
}

struct ResultEntry {
    result: SearchResult,
    values: Vec<(String, String)>,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "unicode" | "codepoint"
    ) && command != "字符"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let arguments = remainder
        .strip_prefix(':')
        .unwrap_or_else(|| remainder.trim_start_matches(char::is_whitespace));
    if arguments.is_empty() {
        return Err(
            "expected text or code points, such as 'unicode 😀' or 'unicode U+1F600'".into(),
        );
    }
    if arguments.len() > MAX_ARGUMENT_BYTES {
        return Err("Unicode input exceeds 256 UTF-8 bytes".into());
    }
    Ok(Some(Request {
        value: arguments.to_owned(),
    }))
}

fn parse_input(value: &str) -> Result<ParsedInput, String> {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let decoded_notation =
        !tokens.is_empty() && tokens.iter().all(|token| looks_like_codepoint(token));
    let text = if decoded_notation {
        tokens
            .into_iter()
            .map(parse_codepoint)
            .collect::<Result<String, _>>()?
    } else {
        value.to_owned()
    };
    let scalar_count = text.chars().count();
    if scalar_count == 0 {
        return Err("Unicode input contains no scalar values".into());
    }
    if scalar_count > MAX_SCALARS {
        return Err(format!(
            "Unicode input contains {scalar_count} scalars; the limit is {MAX_SCALARS}"
        ));
    }
    Ok(ParsedInput {
        text,
        decoded_notation,
    })
}

fn looks_like_codepoint(value: &str) -> bool {
    value.get(..2).is_some_and(|prefix| {
        prefix.eq_ignore_ascii_case("u+") || prefix.eq_ignore_ascii_case("0x")
    }) || ((value.starts_with("\\u{") || value.starts_with("\\U{")) && value.ends_with('}'))
}

fn parse_codepoint(value: &str) -> Result<char, String> {
    let digits = if value.get(..2).is_some_and(|prefix| {
        prefix.eq_ignore_ascii_case("u+") || prefix.eq_ignore_ascii_case("0x")
    }) {
        &value[2..]
    } else if (value.starts_with("\\u{") || value.starts_with("\\U{")) && value.ends_with('}') {
        &value[3..value.len() - 1]
    } else {
        return Err(format!("invalid code-point notation '{value}'"));
    };
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("invalid hexadecimal code point '{value}'"));
    }
    let number = u32::from_str_radix(digits, 16)
        .map_err(|_| format!("invalid hexadecimal code point '{value}'"))?;
    char::from_u32(number).ok_or_else(|| {
        format!("U+{number:04X} is not a Unicode scalar value (surrogates are invalid)")
    })
}

fn inspect_text(parsed: ParsedInput) -> Inspection {
    let scalars: Vec<ScalarInfo> = parsed.text.chars().map(inspect_scalar).collect();
    Inspection {
        utf8_bytes: parsed.text.len(),
        utf16_units: parsed.text.encode_utf16().count(),
        rust_escape: parsed
            .text
            .chars()
            .map(rust_escape)
            .collect::<Vec<_>>()
            .concat(),
        json_escape: parsed.text.chars().map(json_escape).collect(),
        text: parsed.text,
        decoded_notation: parsed.decoded_notation,
        scalars,
    }
}

fn inspect_scalar(character: char) -> ScalarInfo {
    let mut utf8_buffer = [0_u8; 4];
    let encoded_utf8 = character.encode_utf8(&mut utf8_buffer);
    let utf8 = encoded_utf8
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut utf16_buffer = [0_u16; 2];
    let encoded_utf16 = character.encode_utf16(&mut utf16_buffer);
    let utf16 = encoded_utf16
        .iter()
        .map(|unit| format!("{unit:04X}"))
        .collect::<Vec<_>>()
        .join(" ");
    ScalarInfo {
        display: display_character(character),
        codepoint: format!("U+{:04X}", character as u32),
        decimal: character as u32,
        utf8,
        utf16,
        rust_escape: rust_escape(character),
        json_escape: json_escape(character),
        properties: character_properties(character),
        character,
    }
}

fn json_escape(character: char) -> String {
    match character {
        '"' => "\\\"".into(),
        '\\' => "\\\\".into(),
        '\u{08}' => "\\b".into(),
        '\u{0C}' => "\\f".into(),
        '\n' => "\\n".into(),
        '\r' => "\\r".into(),
        '\t' => "\\t".into(),
        ' ' => "\\u0020".into(),
        character if character.is_ascii() && !character.is_control() => character.to_string(),
        character if (character as u32) <= 0xFFFF => format!("\\u{:04X}", character as u32),
        character => {
            let adjusted = character as u32 - 0x1_0000;
            let high = 0xD800 + (adjusted >> 10);
            let low = 0xDC00 + (adjusted & 0x3FF);
            format!("\\u{high:04X}\\u{low:04X}")
        }
    }
}

fn rust_escape(character: char) -> String {
    if character == ' ' {
        "\\u{20}".into()
    } else {
        character.escape_default().collect()
    }
}

fn display_character(character: char) -> String {
    match character {
        '\0' => "␀ NULL".into(),
        '\t' => "⇥ TAB".into(),
        '\n' => "␊ LINE FEED".into(),
        '\r' => "␍ CARRIAGE RETURN".into(),
        ' ' => "␠ SPACE".into(),
        '\u{00A0}' => "⍽ NO-BREAK SPACE".into(),
        '\u{200B}' => "<ZERO WIDTH SPACE>".into(),
        '\u{200C}' => "<ZWNJ>".into(),
        '\u{200D}' => "<ZWJ>".into(),
        '\u{2060}' => "<WORD JOINER>".into(),
        '\u{FEFF}' => "<ZERO WIDTH NO-BREAK SPACE>".into(),
        '\u{034F}' => "<COMBINING GRAPHEME JOINER>".into(),
        '\u{061C}' => "<ARABIC LETTER MARK>".into(),
        '\u{180E}' => "<MONGOLIAN VOWEL SEPARATOR>".into(),
        '\u{200E}' => "<LEFT-TO-RIGHT MARK>".into(),
        '\u{200F}' => "<RIGHT-TO-LEFT MARK>".into(),
        '\u{2028}' => "<LINE SEPARATOR>".into(),
        '\u{2029}' => "<PARAGRAPH SEPARATOR>".into(),
        '\u{202A}' => "<LEFT-TO-RIGHT EMBEDDING>".into(),
        '\u{202B}' => "<RIGHT-TO-LEFT EMBEDDING>".into(),
        '\u{202C}' => "<POP DIRECTIONAL FORMATTING>".into(),
        '\u{202D}' => "<LEFT-TO-RIGHT OVERRIDE>".into(),
        '\u{202E}' => "<RIGHT-TO-LEFT OVERRIDE>".into(),
        '\u{2066}' => "<LEFT-TO-RIGHT ISOLATE>".into(),
        '\u{2067}' => "<RIGHT-TO-LEFT ISOLATE>".into(),
        '\u{2068}' => "<FIRST STRONG ISOLATE>".into(),
        '\u{2069}' => "<POP DIRECTIONAL ISOLATE>".into(),
        character if is_variation_selector(character) => "<VARIATION SELECTOR>".into(),
        character if is_tag_character(character) => "<TAG CHARACTER>".into(),
        character if is_default_ignorable(character) => "<DEFAULT-IGNORABLE>".into(),
        character if is_combining_mark(character) => format!("◌{character}"),
        character if character.is_control() => {
            format!("<CONTROL {}>", character.escape_unicode())
        }
        character => character.to_string(),
    }
}

fn is_combining_mark(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
    )
}

fn is_variation_selector(character: char) -> bool {
    matches!(character as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

fn is_tag_character(character: char) -> bool {
    matches!(character as u32, 0xE0000..=0xE007F)
}

fn is_default_ignorable(character: char) -> bool {
    matches!(
        character as u32,
        0x00AD
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

fn character_properties(character: char) -> String {
    let mut properties = Vec::new();
    if character.is_ascii() {
        properties.push("ASCII");
    }
    if character.is_alphabetic() {
        properties.push("alphabetic");
    }
    if character.is_numeric() {
        properties.push("numeric");
    }
    if character.is_uppercase() {
        properties.push("uppercase");
    }
    if character.is_lowercase() {
        properties.push("lowercase");
    }
    if character.is_whitespace() {
        properties.push("whitespace");
    }
    if character.is_control() {
        properties.push("control");
    }
    if is_combining_mark(character) {
        properties.push("combining mark");
    }
    if is_variation_selector(character) {
        properties.push("variation selector");
    }
    if is_default_ignorable(character) {
        properties.push("default-ignorable");
    }
    if properties.is_empty() {
        "symbol or uncategorized scalar".into()
    } else {
        properties.join(", ")
    }
}

fn build_report(inspection: &Inspection) -> String {
    let origin = if inspection.decoded_notation {
        "Decoded from code-point notation\n"
    } else {
        ""
    };
    let details = inspection
        .scalars
        .iter()
        .enumerate()
        .map(|(index, scalar)| {
            format!(
                "#{} · {} · {}\nDecimal: {}\nUTF-8: {}\nUTF-16: {}\nRust escape: {}\nJSON escape: {}\nProperties: {}",
                index + 1,
                scalar.display,
                scalar.codepoint,
                scalar.decimal,
                scalar.utf8,
                scalar.utf16,
                scalar.rust_escape,
                scalar.json_escape,
                scalar.properties,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Unicode inspection\n{origin}Scalars: {}\nUTF-8 bytes: {}\nUTF-16 units: {}\nRust escapes: {}\nJSON escapes: {}\n\n{details}",
        inspection.scalars.len(),
        inspection.utf8_bytes,
        inspection.utf16_units,
        inspection.rust_escape,
        inspection.json_escape,
    )
}

fn result_entries(inspection: &Inspection, report: &str) -> Vec<ResultEntry> {
    let summary_id = "unicode-inspector:summary".to_owned();
    let summary_title = if inspection.decoded_notation {
        format!(
            "Decoded text · {} Unicode scalars",
            inspection.scalars.len()
        )
    } else {
        format!("{} Unicode scalars", inspection.scalars.len())
    };
    let mut entries = vec![ResultEntry {
        result: SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: summary_id.clone(),
            title: summary_title,
            subtitle: format!(
                "{} UTF-8 bytes · {} UTF-16 units",
                inspection.utf8_bytes, inspection.utf16_units
            ),
            preview: report.into(),
            kind: ResultKind::Command,
            score: 3_393.0,
            metadata: result_metadata(),
            actions: vec![
                action("paste_text", "Paste Text", true, true),
                action("copy_text", "Copy Text", false, false),
                action("copy_escape", "Copy Rust Escapes", false, false),
            ],
        },
        values: vec![
            ("paste_text".into(), inspection.text.clone()),
            ("copy_text".into(), inspection.text.clone()),
            ("copy_escape".into(), inspection.rust_escape.clone()),
        ],
    }];
    entries.extend(
        inspection
            .scalars
            .iter()
            .enumerate()
            .map(|(index, scalar)| {
                let result_id = format!("unicode-inspector:scalar:{index}");
                ResultEntry {
                    result: SearchResult {
                        plugin_id: PLUGIN_ID.into(),
                        id: result_id,
                        title: format!("{}. {} · {}", index + 1, scalar.display, scalar.codepoint),
                        subtitle: format!("UTF-8 {} · UTF-16 {}", scalar.utf8, scalar.utf16),
                        preview: report.into(),
                        kind: ResultKind::Command,
                        score: 3_392.0 - index as f32,
                        metadata: result_metadata(),
                        actions: vec![
                            action("paste_character", "Paste Character", true, true),
                            action("copy_character", "Copy Character", false, false),
                            action("copy_codepoint", "Copy Code Point", false, false),
                            action("copy_escape", "Copy Rust Escape", false, false),
                        ],
                    },
                    values: vec![
                        ("paste_character".into(), scalar.character.to_string()),
                        ("copy_character".into(), scalar.character.to_string()),
                        ("copy_codepoint".into(), scalar.codepoint.clone()),
                        ("copy_escape".into(), scalar.rust_escape.clone()),
                    ],
                }
            }),
    );
    entries
}

fn result_metadata() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("symbol_name".into(), "character.book.closed".into()),
        ("preview_style".into(), "text".into()),
    ])
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "unicode-inspector:invalid".into(),
        title: "Unicode Inspector".into(),
        subtitle: format!("Invalid Unicode input · {message}"),
        preview: format!("Invalid Unicode input\n{message}"),
        kind: ResultKind::Command,
        score: 3_393.0,
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
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window: paste_to_previous,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_commands_and_preserves_colon_whitespace() {
        assert_eq!(parse_request("unicode 😀").unwrap().unwrap().value, "😀");
        assert_eq!(
            parse_request("codepoint:U+1F600").unwrap().unwrap().value,
            "U+1F600"
        );
        assert_eq!(parse_request("字符: ").unwrap().unwrap().value, " ");
        assert_eq!(parse_request("ordinary text").unwrap(), None);
        assert!(parse_request("unicode").is_err());
    }

    #[test]
    fn decodes_supported_codepoint_notations() {
        let parsed = parse_input("U+1F468 0x200D \\u{1F4BB}").unwrap();
        assert_eq!(parsed.text, "👨‍💻");
        assert!(parsed.decoded_notation);
        assert_eq!(parse_input("U+0041").unwrap().text, "A");
    }

    #[test]
    fn rejects_surrogates_invalid_hex_and_out_of_range_values() {
        assert!(parse_input("U+D800").is_err());
        assert!(parse_input("U+110000").is_err());
        assert!(parse_input("U+XYZ").is_err());
        assert!(parse_input("\\u{}").is_err());
    }

    #[test]
    fn reports_utf8_utf16_and_escape_encodings() {
        let scalar = inspect_scalar('😀');
        assert_eq!(scalar.codepoint, "U+1F600");
        assert_eq!(scalar.decimal, 128_512);
        assert_eq!(scalar.utf8, "F0 9F 98 80");
        assert_eq!(scalar.utf16, "D83D DE00");
        assert_eq!(scalar.rust_escape, "\\u{1f600}");
        assert_eq!(scalar.json_escape, "\\uD83D\\uDE00");
        assert_eq!(inspect_scalar(' ').rust_escape, "\\u{20}");
        assert_eq!(inspect_scalar(' ').json_escape, "\\u0020");
    }

    #[test]
    fn invisible_and_directional_characters_use_safe_labels() {
        assert_eq!(display_character('\u{200D}'), "<ZWJ>");
        assert_eq!(display_character('\u{202E}'), "<RIGHT-TO-LEFT OVERRIDE>");
        assert_eq!(display_character('\u{FE0F}'), "<VARIATION SELECTOR>");
        assert_eq!(display_character('\u{2063}'), "<DEFAULT-IGNORABLE>");
        assert!(character_properties('\u{2063}').contains("default-ignorable"));
        assert!(!display_character('\u{202E}').contains('\u{202E}'));
    }

    #[test]
    fn combining_marks_are_rendered_with_a_dotted_circle() {
        assert_eq!(display_character('\u{0301}'), "◌\u{0301}");
        assert!(character_properties('\u{0301}').contains("combining mark"));
    }

    #[test]
    fn character_properties_use_standard_library_predicates() {
        assert_eq!(character_properties('A'), "ASCII, alphabetic, uppercase");
        assert_eq!(character_properties(' '), "ASCII, whitespace");
        assert!(character_properties('٣').contains("numeric"));
    }

    #[test]
    fn report_never_embeds_raw_bidi_controls() {
        let inspection = inspect_text(parse_input("A\u{202E}B").unwrap());
        let report = build_report(&inspection);
        assert!(!report.contains('\u{202E}'));
        assert!(report.contains("<RIGHT-TO-LEFT OVERRIDE>"));
        assert!(report.contains("\\u202E"));
    }

    #[test]
    fn results_are_actionable_bounded_and_invalidate_stale_values() {
        let mut plugin = UnicodeInspectorPlugin::default();
        let results = plugin.search("unicode U+0041 U+1F600", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].title.starts_with("Decoded text"));
        assert_eq!(results[0].actions.len(), 3);
        assert!(results[0].actions[0].default);
        assert_eq!(plugin.active_values.len(), 7);
        plugin.search("ordinary text", 10).unwrap();
        assert!(plugin.active_values.is_empty());
    }

    #[test]
    fn input_limits_are_enforced() {
        let too_many = format!("unicode:{}", "a".repeat(MAX_SCALARS + 1));
        assert!(parse_input(&parse_request(&too_many).unwrap().unwrap().value).is_err());
        let oversized = format!("unicode:{}", "😀".repeat(65));
        assert!(parse_request(&oversized).is_err());
    }
}
