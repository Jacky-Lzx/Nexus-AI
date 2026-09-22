use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.text-diff";
const RESULT_ID: &str = "text-diff:result";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_TOKENS_PER_SIDE: usize = 4_096;
const MAX_MATRIX_CELLS: usize = 1_000_000;
const MAX_REPORT_CHARACTERS: usize = 64_000;
const MAX_RENDERED_TOKEN_CHARACTERS: usize = 4_000;
const MAX_TITLE_CHARACTERS: usize = 160;

fn main() -> io::Result<()> {
    run_plugin(TextDiffPlugin::default())
}

#[derive(Default)]
struct TextDiffPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for TextDiffPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let outcome = match compare(&request) {
            Ok(outcome) => outcome,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result = diff_result(query, &request, &outcome);
        self.action_values
            .insert("copy_left".into(), request.left.clone());
        self.action_values
            .insert("copy_right".into(), request.right.clone());
        if !outcome.identical {
            self.action_values
                .insert("copy_diff".into(), outcome.report.clone());
            self.action_values
                .insert("paste_diff".into(), outcome.report.clone());
        }
        Ok(vec![result])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("text diff is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive Text Diff action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "copy_left" => "Left text copied",
                "copy_right" => "Right text copied",
                _ => "Diff report copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DiffMode {
    #[default]
    Lines,
    Words,
    Characters,
}

impl DiffMode {
    fn label(self) -> &'static str {
        match self {
            Self::Lines => "line",
            Self::Words => "word-token",
            Self::Characters => "Unicode-scalar",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffRequest {
    mode: DiffMode,
    left: String,
    right: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DiffOp {
    Equal(String),
    Remove(String),
    Add(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffOutcome {
    additions: usize,
    removals: usize,
    unchanged: usize,
    identical: bool,
    truncated: bool,
    report: String,
}

fn parse_request(query: &str) -> Result<Option<DiffRequest>, String> {
    let query = query.trim_end();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "diff" | "text-diff") && command != "对比"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let mut arguments = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if arguments.is_empty() {
        return Err("expected '<left> :: <right>' after the diff command".into());
    }
    let mode_end = arguments
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(arguments.len());
    let explicit_mode = match arguments[..mode_end].to_ascii_lowercase().as_str() {
        "line" | "lines" => Some(DiffMode::Lines),
        "word" | "words" => Some(DiffMode::Words),
        "char" | "chars" | "character" | "characters" | "scalar" | "scalars" => {
            Some(DiffMode::Characters)
        }
        _ => None,
    };
    let mode = explicit_mode.unwrap_or_default();
    if explicit_mode.is_some() {
        arguments = arguments[mode_end..]
            .trim_start_matches(|character: char| character.is_whitespace() || character == ':');
    }
    let (left, right) = split_inputs(arguments)?;
    if left.len() > MAX_INPUT_BYTES || right.len() > MAX_INPUT_BYTES {
        return Err("each diff input is limited to 64 KiB".into());
    }
    Ok(Some(DiffRequest {
        mode,
        left: left.to_owned(),
        right: right.to_owned(),
    }))
}

fn split_inputs(arguments: &str) -> Result<(&str, &str), String> {
    if arguments == "::" {
        return Ok(("", ""));
    }
    if let Some((left, right)) = arguments.split_once(" :: ") {
        return Ok((left, right));
    }
    if let Some(right) = arguments.strip_prefix(":: ") {
        return Ok(("", right));
    }
    if let Some(left) = arguments.strip_suffix(" ::") {
        return Ok((left, ""));
    }
    Err("expected ' :: ' between the left and right text".into())
}

fn compare(request: &DiffRequest) -> Result<DiffOutcome, String> {
    if request.left == request.right {
        return Ok(DiffOutcome {
            additions: 0,
            removals: 0,
            unchanged: tokenize(&request.left, request.mode).len(),
            identical: true,
            truncated: false,
            report: "Inputs are identical.".into(),
        });
    }
    let left = tokenize(&request.left, request.mode);
    let right = tokenize(&request.right, request.mode);
    validate_complexity(left.len(), right.len())?;
    let operations = diff_tokens(&left, &right);
    let additions = operations
        .iter()
        .filter(|operation| matches!(operation, DiffOp::Add(_)))
        .count();
    let removals = operations
        .iter()
        .filter(|operation| matches!(operation, DiffOp::Remove(_)))
        .count();
    let unchanged = operations.len() - additions - removals;
    let (report, truncated) = render_report(request.mode, &operations);
    Ok(DiffOutcome {
        additions,
        removals,
        unchanged,
        identical: false,
        truncated,
        report,
    })
}

fn tokenize(value: &str, mode: DiffMode) -> Vec<String> {
    match mode {
        DiffMode::Lines => value.split_inclusive('\n').map(str::to_owned).collect(),
        DiffMode::Characters => value
            .chars()
            .map(|character| character.to_string())
            .collect(),
        DiffMode::Words => tokenize_words(value),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Word,
    Whitespace,
    Punctuation,
}

fn tokenize_words(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_class = None;
    for character in value.chars() {
        let class = if character.is_alphanumeric() || character == '_' {
            WordClass::Word
        } else if character.is_whitespace() {
            WordClass::Whitespace
        } else {
            WordClass::Punctuation
        };
        if current_class.is_some_and(|previous| previous != class)
            || class == WordClass::Punctuation && current_class == Some(WordClass::Punctuation)
        {
            tokens.push(std::mem::take(&mut current));
        }
        current.push(character);
        current_class = Some(class);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn validate_complexity(left: usize, right: usize) -> Result<(), String> {
    if left > MAX_TOKENS_PER_SIDE || right > MAX_TOKENS_PER_SIDE {
        return Err(format!(
            "diff has too many {} units; use line mode or shorter inputs",
            MAX_TOKENS_PER_SIDE
        ));
    }
    let cells = (left + 1)
        .checked_mul(right + 1)
        .ok_or_else(|| "diff comparison is too large".to_owned())?;
    if cells > MAX_MATRIX_CELLS {
        return Err("diff comparison is too complex; use line mode or shorter inputs".into());
    }
    Ok(())
}

fn diff_tokens(left: &[String], right: &[String]) -> Vec<DiffOp> {
    let columns = right.len() + 1;
    let mut lengths = vec![0_u32; (left.len() + 1) * columns];
    for left_index in 1..=left.len() {
        for right_index in 1..=right.len() {
            let index = left_index * columns + right_index;
            lengths[index] = if left[left_index - 1] == right[right_index - 1] {
                lengths[(left_index - 1) * columns + right_index - 1] + 1
            } else {
                lengths[(left_index - 1) * columns + right_index]
                    .max(lengths[left_index * columns + right_index - 1])
            };
        }
    }
    let mut operations = Vec::with_capacity(left.len() + right.len());
    let (mut left_index, mut right_index) = (left.len(), right.len());
    while left_index > 0 || right_index > 0 {
        if left_index > 0 && right_index > 0 && left[left_index - 1] == right[right_index - 1] {
            operations.push(DiffOp::Equal(left[left_index - 1].clone()));
            left_index -= 1;
            right_index -= 1;
        } else if left_index > 0
            && (right_index == 0
                || lengths[(left_index - 1) * columns + right_index]
                    > lengths[left_index * columns + right_index - 1])
        {
            operations.push(DiffOp::Remove(left[left_index - 1].clone()));
            left_index -= 1;
        } else {
            operations.push(DiffOp::Add(right[right_index - 1].clone()));
            right_index -= 1;
        }
    }
    operations.reverse();
    operations
}

fn render_report(mode: DiffMode, operations: &[DiffOp]) -> (String, bool) {
    let mut report = format!(
        "Text Diff · {} comparison\nLegend: '  ' unchanged, '- ' removed, '+ ' added\n\n",
        mode.label()
    );
    let mut characters = report.chars().count();
    let mut truncated = false;
    for operation in operations {
        let (prefix, token) = match operation {
            DiffOp::Equal(token) => ("  ", token),
            DiffOp::Remove(token) => ("- ", token),
            DiffOp::Add(token) => ("+ ", token),
        };
        let (visible, token_truncated) = visible_token(token, mode);
        truncated |= token_truncated;
        let line = format!("{prefix}{visible}\n");
        let line_characters = line.chars().count();
        if characters + line_characters > MAX_REPORT_CHARACTERS {
            truncated = true;
            break;
        }
        report.push_str(&line);
        characters += line_characters;
    }
    if truncated {
        report
            .push_str("\n… diff report truncated; compare shorter inputs for a complete report\n");
    }
    (report, truncated)
}

fn visible_token(value: &str, mode: DiffMode) -> (String, bool) {
    if value.is_empty() {
        return ("⟨empty line⟩".into(), false);
    }
    let whitespace_only = value.chars().all(char::is_whitespace);
    let mut visible = String::new();
    let mut truncated = false;
    for character in value.chars() {
        if visible.chars().count() >= MAX_RENDERED_TOKEN_CHARACTERS {
            truncated = true;
            break;
        }
        match character {
            ' ' if whitespace_only && mode != DiffMode::Lines => visible.push('␠'),
            '\t' => visible.push('⇥'),
            '\n' => visible.push('↵'),
            '\r' => visible.push('␍'),
            character if is_unsafe_invisible(character) => {
                visible.push_str(&format!("\\u{{{:04X}}}", character as u32));
            }
            character => visible.push(character),
        }
    }
    if truncated {
        visible.push('…');
    }
    (visible, truncated)
}

fn is_unsafe_invisible(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

fn diff_result(query: &str, request: &DiffRequest, outcome: &DiffOutcome) -> SearchResult {
    let title = compact(query.trim(), MAX_TITLE_CHARACTERS);
    let subtitle = if outcome.identical {
        format!(
            "No differences · {} comparison · local only",
            request.mode.label()
        )
    } else {
        format!(
            "+{} −{} · {} unchanged · {} comparison{}",
            outcome.additions,
            outcome.removals,
            outcome.unchanged,
            request.mode.label(),
            if outcome.truncated {
                " · bounded report"
            } else {
                ""
            }
        )
    };
    let actions = if outcome.identical {
        vec![
            action("copy_left", "Copy Text", true, false, false),
            action("copy_right", "Copy Right Text", false, false, false),
        ]
    } else {
        vec![
            action("paste_diff", "Paste Diff Report", true, true, true),
            action("copy_diff", "Copy Diff Report", false, false, false),
            action("copy_left", "Copy Left Text", false, false, false),
            action("copy_right", "Copy Right Text", false, false, false),
        ]
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title,
        subtitle,
        preview: format!(
            "Left: {} bytes · Right: {} bytes\n\n{}",
            request.left.len(),
            request.right.len(),
            outcome.report
        ),
        kind: ResultKind::Command,
        score: 3_410.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "plus.forwardslash.minus".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "text-diff:invalid".into(),
        title: compact(query.trim(), MAX_TITLE_CHARACTERS),
        subtitle: format!("Invalid text diff · {}", compact(message, 120)),
        preview: format!("Invalid text diff\n{message}"),
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
    fn parses_explicit_commands_modes_and_ignores_other_queries() {
        assert_eq!(
            parse_request("diff old :: new").unwrap(),
            Some(DiffRequest {
                mode: DiffMode::Lines,
                left: "old".into(),
                right: "new".into(),
            })
        );
        assert_eq!(
            parse_request("text-diff words hello, world :: hello world!")
                .unwrap()
                .unwrap()
                .mode,
            DiffMode::Words
        );
        assert_eq!(
            parse_request("DIFF WORDS hello :: world")
                .unwrap()
                .unwrap()
                .mode,
            DiffMode::Words
        );
        assert_eq!(
            parse_request("对比 chars 👨 :: 👩").unwrap().unwrap().mode,
            DiffMode::Characters
        );
        assert_eq!(parse_request("different text").unwrap(), None);
    }

    #[test]
    fn supports_empty_sides_and_rejects_invalid_or_oversized_inputs() {
        let request = parse_request("diff :: added").unwrap().unwrap();
        assert_eq!(request.left, "");
        assert_eq!(request.right, "added");
        assert_eq!(parse_request("diff removed ::").unwrap().unwrap().right, "");
        assert!(parse_request("diff missing separator").is_err());
        assert!(parse_request(&format!("diff {} :: ok", "x".repeat(MAX_INPUT_BYTES + 1))).is_err());
    }

    #[test]
    fn line_diff_orders_removals_before_replacement_additions() {
        let request = DiffRequest {
            mode: DiffMode::Lines,
            left: "alpha\nbeta\ngamma".into(),
            right: "alpha\ndelta\ngamma".into(),
        };
        let outcome = compare(&request).unwrap();
        assert_eq!(outcome.additions, 1);
        assert_eq!(outcome.removals, 1);
        assert_eq!(outcome.unchanged, 2);
        assert!(outcome.report.contains("- beta↵\n+ delta↵"));

        let empty_to_text = compare(&DiffRequest {
            mode: DiffMode::Lines,
            left: String::new(),
            right: "added".into(),
        })
        .unwrap();
        assert_eq!((empty_to_text.additions, empty_to_text.removals), (1, 0));
    }

    #[test]
    fn word_tokens_keep_runs_and_split_punctuation() {
        assert_eq!(
            tokenize_words("hello,  世界!"),
            vec!["hello", ",", "  ", "世界", "!"]
        );
        let request = DiffRequest {
            mode: DiffMode::Words,
            left: "hello, world".into(),
            right: "hello world!".into(),
        };
        let outcome = compare(&request).unwrap();
        assert_eq!(outcome.additions, 1);
        assert_eq!(outcome.removals, 1);
        assert!(outcome.report.contains("- ,"));
        assert!(outcome.report.contains("+ !"));
    }

    #[test]
    fn character_mode_compares_unicode_scalars_without_splitting_utf8() {
        let request = DiffRequest {
            mode: DiffMode::Characters,
            left: "甲🙂乙".into(),
            right: "甲🙃乙".into(),
        };
        let outcome = compare(&request).unwrap();
        assert_eq!(
            (outcome.additions, outcome.removals, outcome.unchanged),
            (1, 1, 2)
        );
        assert!(outcome.report.contains("- 🙂\n+ 🙃"));
    }

    #[test]
    fn identical_inputs_have_no_diff_actions() {
        let mut plugin = TextDiffPlugin::default();
        let result = plugin.search("diff same :: same", 20).unwrap().remove(0);
        assert!(result.subtitle.starts_with("No differences"));
        assert_eq!(result.actions.len(), 2);
        assert!(!plugin.action_values.contains_key("copy_diff"));
    }

    #[test]
    fn complexity_is_bounded_before_allocating_the_matrix() {
        assert!(validate_complexity(1_000, 1_000).is_err());
        assert!(validate_complexity(MAX_TOKENS_PER_SIDE + 1, 1).is_err());
        assert!(validate_complexity(100, 100).is_ok());
    }

    #[test]
    fn reports_escape_directional_controls_and_bound_long_tokens() {
        let (visible, truncated) = visible_token("a\u{202e}b", DiffMode::Lines);
        assert_eq!(visible, "a\\u{202E}b");
        assert!(!truncated);
        let (_, truncated) = visible_token(
            &"x".repeat(MAX_RENDERED_TOKEN_CHARACTERS + 1),
            DiffMode::Lines,
        );
        assert!(truncated);
    }

    #[test]
    fn results_are_actionable_and_invalidate_stale_values() {
        let mut plugin = TextDiffPlugin::default();
        let result = plugin
            .search("diff lines one\ntwo :: one\nthree", 20)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, RESULT_ID);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(result.metadata["preview_style"], "text");
        assert!(plugin.action_values["copy_diff"].contains("- two\n+ three"));
        plugin.search("ordinary search", 20).unwrap();
        assert!(plugin.action_values.is_empty());
    }

    #[test]
    fn invalid_results_are_visible_but_safe_and_zero_limit_is_empty() {
        let mut plugin = TextDiffPlugin::default();
        assert!(plugin.search("diff a :: b", 0).unwrap().is_empty());
        let result = plugin.search("diff invalid", 20).unwrap().remove(0);
        assert!(result.subtitle.starts_with("Invalid text diff"));
        assert!(result.actions.is_empty());
        assert_eq!(result.metadata["symbol_color"], "#FF9F0A");
    }
}
