use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use regex::{Regex, RegexBuilder};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

const PLUGIN_ID: &str = "io.clipkeep.regex-tester";
const MAX_PATTERN_BYTES: usize = 4 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_MATCHES: usize = 100;
const MAX_TITLE_CHARACTERS: usize = 160;
const MAX_VALUE_PREVIEW_CHARACTERS: usize = 120;

fn main() -> io::Result<()> {
    run_plugin(RegexTesterPlugin::default())
}

#[derive(Default)]
struct RegexTesterPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for RegexTesterPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let outcome = match evaluate(&request) {
            Ok(outcome) => outcome,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = "regex-tester:matches".to_owned();
        if !outcome.copy_value.is_empty() {
            self.recent_values
                .insert(result_id.clone(), outcome.copy_value.clone());
        }
        Ok(vec![match_result(query, request, outcome, result_id)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown regex tester action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "regex matches are no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Regex matches copied".into()))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RegexFlags {
    case_insensitive: bool,
    multi_line: bool,
    dot_matches_new_line: bool,
    swap_greed: bool,
    ignore_whitespace: bool,
}

impl RegexFlags {
    fn parse(value: &str) -> Result<Self, String> {
        let mut flags = Self::default();
        for flag in value.chars() {
            match flag {
                'i' => flags.case_insensitive = true,
                'm' => flags.multi_line = true,
                's' => flags.dot_matches_new_line = true,
                'U' => flags.swap_greed = true,
                'x' => flags.ignore_whitespace = true,
                'g' | 'u' => {}
                _ => return Err(format!("unsupported regex flag '{flag}'")),
            }
        }
        Ok(flags)
    }

    fn display(self) -> String {
        let mut value = String::new();
        if self.case_insensitive {
            value.push('i');
        }
        if self.multi_line {
            value.push('m');
        }
        if self.dot_matches_new_line {
            value.push('s');
        }
        if self.swap_greed {
            value.push('U');
        }
        if self.ignore_whitespace {
            value.push('x');
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegexRequest {
    pattern: String,
    flags: RegexFlags,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegexOutcome {
    match_count: usize,
    truncated: bool,
    copy_value: String,
    report: String,
}

fn parse_request(query: &str) -> Result<Option<RegexRequest>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "regex" | "re") && command != "正则" {
        return Ok(None);
    }
    let arguments = query[command_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace());
    if arguments.is_empty() {
        return Err("expected /pattern/flags text or pattern :: text".into());
    }
    let request = if arguments.starts_with('/') {
        parse_delimited(arguments)?
    } else {
        parse_double_colon(arguments)?
    };
    validate_request(request).map(Some)
}

fn parse_delimited(arguments: &str) -> Result<RegexRequest, String> {
    let closing = closing_slash(arguments)
        .ok_or_else(|| "missing closing '/' for regex pattern".to_owned())?;
    let pattern = arguments[1..closing].to_owned();
    let remainder = &arguments[closing + 1..];
    let separator = remainder
        .find(char::is_whitespace)
        .ok_or_else(|| "expected text after /pattern/flags".to_owned())?;
    let flags = RegexFlags::parse(&remainder[..separator])?;
    let text = remainder[separator..].trim_start().to_owned();
    if text.is_empty() {
        return Err("regex test text cannot be empty".into());
    }
    Ok(RegexRequest {
        pattern,
        flags,
        text,
    })
}

fn closing_slash(arguments: &str) -> Option<usize> {
    let bytes = arguments.as_bytes();
    for index in 1..bytes.len() {
        if bytes[index] != b'/' {
            continue;
        }
        let mut backslashes = 0;
        let mut previous = index;
        while previous > 0 && bytes[previous - 1] == b'\\' {
            backslashes += 1;
            previous -= 1;
        }
        if backslashes % 2 == 0 {
            return Some(index);
        }
    }
    None
}

fn parse_double_colon(arguments: &str) -> Result<RegexRequest, String> {
    let (pattern, text) = arguments
        .split_once(" :: ")
        .ok_or_else(|| "expected ' :: ' between pattern and test text".to_owned())?;
    if pattern.trim().is_empty() {
        return Err("regex pattern cannot be empty in 'pattern :: text' syntax".into());
    }
    if text.trim().is_empty() {
        return Err("regex test text cannot be empty".into());
    }
    Ok(RegexRequest {
        pattern: pattern.trim().to_owned(),
        flags: RegexFlags::default(),
        text: text.trim().to_owned(),
    })
}

fn validate_request(request: RegexRequest) -> Result<RegexRequest, String> {
    if request.pattern.len() > MAX_PATTERN_BYTES {
        return Err("regex pattern exceeds 4 KiB".into());
    }
    if request.text.len() > MAX_TEXT_BYTES {
        return Err("regex test text exceeds 64 KiB".into());
    }
    Ok(request)
}

fn evaluate(request: &RegexRequest) -> Result<RegexOutcome, String> {
    let regex = compile(request)?;
    let capture_names: Vec<Option<String>> = regex
        .capture_names()
        .map(|name| name.map(str::to_owned))
        .collect();
    let mut report_lines = Vec::new();
    let mut matched_values = Vec::new();
    let mut truncated = false;
    for (index, captures) in regex.captures_iter(&request.text).enumerate() {
        if index == MAX_MATCHES {
            truncated = true;
            break;
        }
        let whole = captures
            .get(0)
            .expect("regex capture zero is always the full match");
        matched_values.push(whole.as_str().to_owned());
        let (line, column) = line_and_column(&request.text, whole.start());
        report_lines.push(format!(
            "{}. \"{}\" · bytes {}–{} · line {}, column {}",
            index + 1,
            escaped_preview(whole.as_str(), MAX_VALUE_PREVIEW_CHARACTERS),
            whole.start(),
            whole.end(),
            line,
            column
        ));
        for group_index in 1..captures.len() {
            if let Some(group) = captures.get(group_index) {
                let label = capture_names
                    .get(group_index)
                    .and_then(Option::as_deref)
                    .map(|name| format!("<{name}>"))
                    .unwrap_or_else(|| group_index.to_string());
                report_lines.push(format!(
                    "   group {label}: \"{}\" · bytes {}–{}",
                    escaped_preview(group.as_str(), MAX_VALUE_PREVIEW_CHARACTERS),
                    group.start(),
                    group.end()
                ));
            }
        }
    }
    if report_lines.is_empty() {
        report_lines.push("No matches.".into());
    } else if truncated {
        report_lines.push(format!("… report limited to {MAX_MATCHES} matches"));
    }
    Ok(RegexOutcome {
        match_count: matched_values.len(),
        truncated,
        copy_value: matched_values.join("\n"),
        report: report_lines.join("\n"),
    })
}

fn compile(request: &RegexRequest) -> Result<Regex, String> {
    RegexBuilder::new(&request.pattern)
        .case_insensitive(request.flags.case_insensitive)
        .multi_line(request.flags.multi_line)
        .dot_matches_new_line(request.flags.dot_matches_new_line)
        .swap_greed(request.flags.swap_greed)
        .ignore_whitespace(request.flags.ignore_whitespace)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|error| format!("invalid Rust regex: {error}"))
}

fn line_and_column(text: &str, byte_index: usize) -> (usize, usize) {
    let prefix = &text[..byte_index];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

fn escaped_preview(value: &str, maximum_characters: usize) -> String {
    let escaped: String = value.chars().flat_map(char::escape_default).collect();
    compact(&escaped, maximum_characters)
}

fn match_result(
    query: &str,
    request: RegexRequest,
    outcome: RegexOutcome,
    result_id: String,
) -> SearchResult {
    let flags = request.flags.display();
    let limit_note = if outcome.truncated {
        " (first 100)"
    } else {
        ""
    };
    let count_label = if outcome.match_count == 1 {
        "1 match".to_owned()
    } else {
        format!("{} matches", outcome.match_count)
    };
    let subtitle = if flags.is_empty() {
        format!("{count_label}{limit_note} · Rust regex")
    } else {
        format!("{count_label}{limit_note} · flags {flags} · Rust regex")
    };
    let actions = if outcome.copy_value.is_empty() {
        Vec::new()
    } else {
        vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste Matches".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy Matches".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ]
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(query.trim(), MAX_TITLE_CHARACTERS),
        subtitle,
        preview: format!(
            "Pattern: /{}/{}\nText: {} bytes\n\n{}",
            request.pattern,
            flags,
            request.text.len(),
            outcome.report
        ),
        kind: ResultKind::Command,
        score: 3_405.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "text.magnifyingglass".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "regex-tester:invalid".into(),
        title: compact(query.trim(), MAX_TITLE_CHARACTERS),
        subtitle: format!("Invalid regex test · {}", compact(message, 120)),
        preview: format!("Invalid regex test\n{message}"),
        kind: ResultKind::Command,
        score: 3_405.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
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
    fn parses_delimited_patterns_flags_and_escaped_slashes() {
        assert_eq!(
            parse_request(r"regex /(?P<word>\w+)/im Hello World").unwrap(),
            Some(RegexRequest {
                pattern: r"(?P<word>\w+)".into(),
                flags: RegexFlags {
                    case_insensitive: true,
                    multi_line: true,
                    ..RegexFlags::default()
                },
                text: "Hello World".into(),
            })
        );
        let escaped = parse_request(r"re /https:\/\/[^ ]+/ https://example.com")
            .unwrap()
            .unwrap();
        assert_eq!(escaped.pattern, r"https:\/\/[^ ]+");
        assert_eq!(escaped.text, "https://example.com");
    }

    #[test]
    fn parses_double_colon_syntax_and_rejects_bad_inputs() {
        let request = parse_request(r"正则 (?P<key>\w+)=(?P<value>\d+) :: age=42")
            .unwrap()
            .unwrap();
        assert_eq!(request.pattern, r"(?P<key>\w+)=(?P<value>\d+)");
        assert_eq!(request.text, "age=42");
        assert!(parse_request("regex /unterminated text").is_err());
        assert!(parse_request("regex /ok/z text").is_err());
        assert!(parse_request("regex no separator").is_err());
        assert_eq!(parse_request("ordinary text").unwrap(), None);
    }

    #[test]
    fn reports_match_positions_numbered_and_named_captures() {
        let request = RegexRequest {
            pattern: r"(?P<key>\w+)=(\d+)".into(),
            flags: RegexFlags::default(),
            text: "name=7\nage=42".into(),
        };
        let outcome = evaluate(&request).unwrap();
        assert_eq!(outcome.match_count, 2);
        assert_eq!(outcome.copy_value, "name=7\nage=42");
        assert!(outcome.report.contains("line 1, column 1"));
        assert!(outcome.report.contains("line 2, column 1"));
        assert!(outcome.report.contains("group <key>: \"name\""));
        assert!(outcome.report.contains("group 2: \"42\""));
    }

    #[test]
    fn applies_common_flags() {
        let request = RegexRequest {
            pattern: "^hello.$".into(),
            flags: RegexFlags {
                case_insensitive: true,
                multi_line: true,
                dot_matches_new_line: true,
                ..RegexFlags::default()
            },
            text: "HELLO\n".into(),
        };
        assert_eq!(evaluate(&request).unwrap().match_count, 1);
    }

    #[test]
    fn no_match_has_no_copy_actions_and_invalid_regex_is_visible() {
        let mut plugin = RegexTesterPlugin::default();
        let no_match = plugin.search("regex /z+/ abc", 20).unwrap();
        assert_eq!(no_match.len(), 1);
        assert_eq!(no_match[0].subtitle, "0 matches · Rust regex");
        assert!(no_match[0].actions.is_empty());

        let invalid = plugin.search("regex /(/ text", 20).unwrap();
        assert_eq!(invalid.len(), 1);
        assert!(invalid[0].subtitle.starts_with("Invalid regex test · "));
        assert_eq!(invalid[0].metadata["symbol_color"], "#FF9F0A");
    }

    #[test]
    fn limits_reports_to_one_hundred_matches() {
        let request = RegexRequest {
            pattern: ".".into(),
            flags: RegexFlags::default(),
            text: "a".repeat(150),
        };
        let outcome = evaluate(&request).unwrap();
        assert_eq!(outcome.match_count, 100);
        assert!(outcome.truncated);
        assert!(outcome.report.ends_with("… report limited to 100 matches"));
    }

    #[test]
    fn search_result_is_rankable_and_invalidates_old_values() {
        let mut plugin = RegexTesterPlugin::default();
        let query = "regex /(?P<n>\\d+)/ item 42";
        let results = plugin.search(query, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, query);
        assert_eq!(results[0].subtitle, "1 match · Rust regex");
        assert!(results[0].preview.contains("group <n>: \"42\""));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values[&results[0].id], "42");

        plugin.search("not a command", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }

    #[test]
    fn compact_and_positions_are_unicode_safe() {
        assert_eq!(compact("甲乙丙丁", 3), "甲乙丙…");
        assert_eq!(line_and_column("甲乙\n丙", "甲乙\n".len()), (2, 1));
    }
}
