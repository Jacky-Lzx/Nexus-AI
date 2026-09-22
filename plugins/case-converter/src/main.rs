use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.case-converter";
const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_WORDS: usize = 2_048;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(CaseConverterPlugin::default())
}

#[derive(Default)]
struct CaseConverterPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for CaseConverterPlugin {
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
        let words = match split_words(&request.input) {
            Ok(words) => words,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let styles = request
            .style
            .map(|style| vec![style])
            .unwrap_or_else(|| Style::ALL.to_vec());
        let mut results = Vec::new();
        for (index, style) in styles.into_iter().take(limit).enumerate() {
            let value = style.convert(&words);
            if value.len() > MAX_OUTPUT_BYTES {
                self.result_values.clear();
                return Ok(vec![invalid_result(
                    query,
                    "converted identifier exceeds 64 KiB",
                )]);
            }
            let result_id = format!("case-converter:{}", style.id());
            self.result_values.insert(result_id.clone(), value.clone());
            results.push(case_result(
                style,
                result_id,
                value,
                words.len(),
                3_440.0 - index as f32,
            ));
        }
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Case Converter action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "case conversion is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Converted text copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Camel,
    Pascal,
    Snake,
    Kebab,
    Constant,
    Dot,
    Title,
    Words,
}

impl Style {
    const ALL: [Self; 8] = [
        Self::Camel,
        Self::Pascal,
        Self::Snake,
        Self::Kebab,
        Self::Constant,
        Self::Dot,
        Self::Title,
        Self::Words,
    ];

    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "camel" | "camelcase" => Ok(Self::Camel),
            "pascal" | "pascalcase" => Ok(Self::Pascal),
            "snake" | "snake_case" => Ok(Self::Snake),
            "kebab" | "kebab-case" => Ok(Self::Kebab),
            "constant" | "constant_case" | "screaming-snake" => Ok(Self::Constant),
            "dot" | "dot.case" => Ok(Self::Dot),
            "title" | "title-case" => Ok(Self::Title),
            "words" | "lower-words" => Ok(Self::Words),
            _ => Err(
                "style must be camel, pascal, snake, kebab, constant, dot, title, or words".into(),
            ),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Camel => "camel",
            Self::Pascal => "pascal",
            Self::Snake => "snake",
            Self::Kebab => "kebab",
            Self::Constant => "constant",
            Self::Dot => "dot",
            Self::Title => "title",
            Self::Words => "words",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Camel => "camelCase",
            Self::Pascal => "PascalCase",
            Self::Snake => "snake_case",
            Self::Kebab => "kebab-case",
            Self::Constant => "CONSTANT_CASE",
            Self::Dot => "dot.case",
            Self::Title => "Title Case",
            Self::Words => "lowercase words",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Camel | Self::Pascal => "textformat.abc",
            Self::Snake | Self::Constant => "underline",
            Self::Kebab => "minus",
            Self::Dot => "ellipsis",
            Self::Title | Self::Words => "textformat",
        }
    }

    fn convert(self, words: &[String]) -> String {
        match self {
            Self::Camel => {
                let mut output = words[0].clone();
                for word in &words[1..] {
                    output.push_str(&capitalize(word));
                }
                output
            }
            Self::Pascal => words
                .iter()
                .map(|word| capitalize(word))
                .collect::<Vec<_>>()
                .join(""),
            Self::Snake => words.join("_"),
            Self::Kebab => words.join("-"),
            Self::Constant => words
                .iter()
                .map(|word| word.to_uppercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::Dot => words.join("."),
            Self::Title => words
                .iter()
                .map(|word| capitalize(word))
                .collect::<Vec<_>>()
                .join(" "),
            Self::Words => words.join(" "),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    style: Option<Style>,
    input: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let short_alias = command_lower == "case";
    if !matches!(command_lower.as_str(), "caseconv" | "caseconvert")
        && !short_alias
        && command != "命名转换"
    {
        return Ok(None);
    }
    if short_alias && !query.contains("::") {
        return Ok(None);
    }
    if query.len() > MAX_INPUT_BYTES + 1024 {
        return Err("Case Converter query exceeds its bounded input size".into());
    }
    let remainder = query[command_end..].trim_start_matches(':').trim_start();
    let Some((style_text, input)) = remainder.split_once("::") else {
        return Err("expected '::' before the text to convert".into());
    };
    let style_text = style_text.trim();
    let style = if style_text.is_empty() {
        None
    } else if style_text.split_whitespace().count() == 1 {
        Some(Style::parse(style_text)?)
    } else {
        return Err("expected at most one target naming style".into());
    };
    let input = input.strip_prefix(' ').unwrap_or(input);
    if input.is_empty() {
        return Err("text to convert cannot be empty".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("text to convert exceeds 16 KiB".into());
    }
    Ok(Some(Request {
        style,
        input: input.into(),
    }))
}

fn split_words(input: &str) -> Result<Vec<String>, String> {
    let characters = input.chars().collect::<Vec<_>>();
    let mut words = Vec::<String>::new();
    let mut current = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            push_word(&mut words, &mut current)?;
            continue;
        }
        let previous = current.chars().last();
        let next = characters.get(index + 1).copied();
        let boundary = previous.is_some_and(|previous| {
            (previous.is_lowercase() && character.is_uppercase())
                || (previous.is_alphabetic() && character.is_numeric())
                || (previous.is_numeric() && character.is_alphabetic())
                || (previous.is_uppercase()
                    && character.is_uppercase()
                    && next.is_some_and(char::is_lowercase))
        });
        if boundary {
            push_word(&mut words, &mut current)?;
        }
        current.push(character);
    }
    push_word(&mut words, &mut current)?;
    if words.is_empty() {
        return Err("text does not contain any Unicode letters or numbers".into());
    }
    Ok(words)
}

fn push_word(words: &mut Vec<String>, current: &mut String) -> Result<(), String> {
    if current.is_empty() {
        return Ok(());
    }
    if words.len() == MAX_WORDS {
        return Err(format!("text exceeds {MAX_WORDS} detected words"));
    }
    words.push(current.to_lowercase());
    current.clear();
    Ok(())
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_uppercase().chain(characters).collect::<String>()
}

fn case_result(
    style: Style,
    result_id: String,
    value: String,
    word_count: usize,
    score: f32,
) -> SearchResult {
    let visible = compact(&value, MAX_VISIBLE_CHARACTERS);
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&value, 180),
        subtitle: format!(
            "{} · {word_count} detected word{}",
            style.label(),
            if word_count == 1 { "" } else { "s" }
        ),
        preview: format!(
            "Case conversion\nStyle: {}\nDetected words: {word_count}\n\nResult\n{visible}",
            style.label()
        ),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([
            ("symbol_name".into(), style.symbol().into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Converted Text", true, true, true),
            action("copy", "Copy Converted Text", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "case-converter:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Case conversion unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Case conversion unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_440.0,
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
    fn parses_explicit_targets_all_styles_and_nonstealing_aliases() {
        assert!(parse_request("ordinary words").unwrap().is_none());
        assert!(parse_request("case study").unwrap().is_none());
        let all = parse_request("caseconv :: user profile ID")
            .unwrap()
            .unwrap();
        assert_eq!(all.style, None);
        let snake = parse_request("case snake :: userProfileID")
            .unwrap()
            .unwrap();
        assert_eq!(snake.style, Some(Style::Snake));
        assert_eq!(snake.input, "userProfileID");
    }

    #[test]
    fn splits_separators_lowercase_uppercase_and_acronym_boundaries() {
        assert_eq!(
            split_words("hello-world_some value").unwrap(),
            ["hello", "world", "some", "value"]
        );
        assert_eq!(
            split_words("XMLHttpRequest").unwrap(),
            ["xml", "http", "request"]
        );
        assert_eq!(split_words("userIDValue").unwrap(), ["user", "id", "value"]);
    }

    #[test]
    fn splits_letter_number_transitions() {
        assert_eq!(
            split_words("HTTP2Client42").unwrap(),
            ["http", "2", "client", "42"]
        );
        assert_eq!(
            split_words("version2beta").unwrap(),
            ["version", "2", "beta"]
        );
    }

    #[test]
    fn retains_unicode_alphanumeric_words_and_casing() {
        let words = split_words("ÜberStraße 東京2026").unwrap();
        assert_eq!(words, ["über", "straße", "東京", "2026"]);
        assert_eq!(Style::Pascal.convert(&words), "ÜberStraße東京2026");
        assert_eq!(Style::Snake.convert(&words), "über_straße_東京_2026");
    }

    #[test]
    fn converts_every_supported_style() {
        let words = vec!["user".into(), "profile".into(), "id".into()];
        let expected = [
            "userProfileId",
            "UserProfileId",
            "user_profile_id",
            "user-profile-id",
            "USER_PROFILE_ID",
            "user.profile.id",
            "User Profile Id",
            "user profile id",
        ];
        for (style, expected) in Style::ALL.into_iter().zip(expected) {
            assert_eq!(style.convert(&words), expected);
        }
    }

    #[test]
    fn targeted_search_returns_one_result_and_all_styles_respect_limit() {
        let mut plugin = CaseConverterPlugin::default();
        let targeted = plugin.search("caseconv kebab :: HelloWorld", 10).unwrap();
        assert_eq!(targeted.len(), 1);
        assert_eq!(targeted[0].title, "hello-world");
        let limited = plugin.search("caseconv :: HelloWorld", 3).unwrap();
        assert_eq!(limited.len(), 3);
        assert_eq!(limited[0].subtitle.split(' ').next(), Some("camelCase"));
    }

    #[test]
    fn rejects_missing_bad_empty_and_oversized_inputs() {
        assert!(parse_request("caseconv snake text").is_err());
        assert!(parse_request("caseconv unknown :: text").is_err());
        assert!(parse_request("caseconv snake extra :: text").is_err());
        assert!(parse_request("caseconv snake ::").is_err());
        assert!(
            parse_request(&format!("caseconv :: {}", "x".repeat(MAX_INPUT_BYTES + 1))).is_err()
        );
        assert!(split_words("---___").is_err());
    }

    #[test]
    fn enforces_detected_word_limit() {
        let input = std::iter::repeat_n("x", MAX_WORDS + 1)
            .collect::<Vec<_>>()
            .join("-");
        assert!(input.len() < MAX_INPUT_BYTES);
        assert!(split_words(&input).is_err());
    }

    #[test]
    fn results_are_actionable_bounded_and_invalidate_stale_values() {
        let mut plugin = CaseConverterPlugin::default();
        let result = plugin
            .search("caseconv snake :: HelloWorld", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, "case-converter:snake");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert_eq!(plugin.result_values[&result.id], "hello_world");
        let invalid = plugin.search("caseconv snake :: ---", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
        assert!(plugin.search("caseconv :: text", 0).unwrap().is_empty());
    }

    #[test]
    fn invalid_results_escape_directional_controls() {
        let mut plugin = CaseConverterPlugin::default();
        let result = plugin
            .search("caseconv nope :: value\u{202E}", 1)
            .unwrap()
            .remove(0);
        assert!(!result.title.contains('\u{202E}'));
        assert!(result.title.contains("\\u{202E}"));
    }
}
