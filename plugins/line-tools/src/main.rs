use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

const PLUGIN_ID: &str = "io.clipkeep.line-tools";
const RESULT_ID: &str = "line-tools:result";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_LINES: usize = 10_000;
const MAX_PREVIEW_CHARACTERS: usize = 16 * 1024;

fn main() -> io::Result<()> {
    run_plugin(LineToolsPlugin::default())
}

#[derive(Default)]
struct LineToolsPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for LineToolsPlugin {
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
        let transformation = match transform(&request) {
            Ok(transformation) => transformation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values
            .insert("paste_output".into(), transformation.output.clone());
        self.action_values
            .insert("copy_output".into(), transformation.output.clone());
        self.action_values
            .insert("copy_report".into(), transformation.report.clone());
        Ok(vec![transformation_result(&transformation)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("line transformation is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive Line Tools action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            if action_id == "copy_report" {
                "Line processing report copied"
            } else {
                "Processed lines copied"
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Sort,
    Unique,
    SortUnique,
    Reverse,
    Frequency,
}

impl Operation {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "sort" => Ok(Self::Sort),
            "unique" | "dedupe" => Ok(Self::Unique),
            "sort-unique" | "sortunique" => Ok(Self::SortUnique),
            "reverse" => Ok(Self::Reverse),
            "frequency" | "freq" => Ok(Self::Frequency),
            _ => Err("operation must be sort, unique, sort-unique, reverse, or frequency".into()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sort => "Sort",
            Self::Unique => "Unique",
            Self::SortUnique => "Sort unique",
            Self::Reverse => "Reverse",
            Self::Frequency => "Frequency",
        }
    }

    fn result_label(self) -> &'static str {
        match self {
            Self::Sort => "Sorted",
            Self::Unique => "Deduplicated",
            Self::SortUnique => "Sorted unique",
            Self::Reverse => "Reversed",
            Self::Frequency => "Frequency table",
        }
    }

    fn supports_order(self) -> bool {
        matches!(self, Self::Sort | Self::SortUnique | Self::Frequency)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Ascending,
    Descending,
}

impl Direction {
    fn label(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Options {
    direction: Option<Direction>,
    fold: bool,
    trim: bool,
    nonempty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    options: Options,
    input: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let short_alias = command_lower == "lines";
    if command_lower != "linetools" && !short_alias && command != "行工具" {
        return Ok(None);
    }
    if short_alias && !query.contains("::") {
        return Ok(None);
    }
    if query.len() > MAX_INPUT_BYTES + 1024 {
        return Err("Line Tools query exceeds its bounded input size".into());
    }
    let remainder = query[command_end..].trim_start_matches(':').trim_start();
    let Some((specification, input)) = remainder.split_once("::") else {
        return Err("expected '::' between options and multiline input".into());
    };
    let mut tokens = specification.split_whitespace();
    let operation = Operation::parse(
        tokens
            .next()
            .ok_or("expected a line operation before '::'")?,
    )?;
    let mut options = Options::default();
    for token in tokens {
        match token.to_ascii_lowercase().as_str() {
            "asc" | "ascending" if options.direction.is_none() => {
                options.direction = Some(Direction::Ascending);
            }
            "desc" | "descending" if options.direction.is_none() => {
                options.direction = Some(Direction::Descending);
            }
            "asc" | "ascending" | "desc" | "descending" => {
                return Err("line ordering may be specified only once".into());
            }
            "fold" | "ignore-case" if !options.fold => options.fold = true,
            "trim" if !options.trim => options.trim = true,
            "nonempty" | "drop-empty" if !options.nonempty => options.nonempty = true,
            "fold" | "ignore-case" | "trim" | "nonempty" | "drop-empty" => {
                return Err(format!("duplicate Line Tools option: {token}"));
            }
            _ => return Err(format!("unknown Line Tools option: {}", safe_text(token))),
        }
    }
    if options.direction.is_some() && !operation.supports_order() {
        return Err(format!("{} does not accept asc or desc", operation.label()));
    }
    let input = input.strip_prefix(' ').unwrap_or(input);
    if input.is_empty() {
        return Err("multiline input cannot be empty".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("multiline input exceeds 64 KiB".into());
    }
    Ok(Some(Request {
        operation,
        options,
        input: input.into(),
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LineInput {
    lines: Vec<String>,
    final_newline: bool,
}

fn split_lines(input: &str) -> LineInput {
    let bytes = input.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(input[start..index].into());
                index += 1;
                start = index;
            }
            b'\r' => {
                lines.push(input[start..index].into());
                index += 1;
                if index < bytes.len() && bytes[index] == b'\n' {
                    index += 1;
                }
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < input.len() {
        lines.push(input[start..].into());
    }
    LineInput {
        lines,
        final_newline: start == input.len() && start > 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LineItem {
    value: String,
    key: String,
    first_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transformation {
    operation: Operation,
    options: Options,
    input_lines: usize,
    output_lines: usize,
    trimmed_lines: usize,
    removed_empty_lines: usize,
    removed_duplicates: usize,
    final_newline: bool,
    output: String,
    report: String,
    preview: String,
}

fn transform(request: &Request) -> Result<Transformation, String> {
    let split = split_lines(&request.input);
    if split.lines.len() > MAX_LINES {
        return Err(format!("multiline input exceeds {MAX_LINES} logical lines"));
    }
    let input_lines = split.lines.len();
    let mut trimmed_lines = 0;
    let mut removed_empty_lines = 0;
    let mut items = Vec::with_capacity(input_lines);
    for (first_index, line) in split.lines.into_iter().enumerate() {
        let value = if request.options.trim {
            let trimmed = line.trim();
            trimmed_lines += usize::from(trimmed != line);
            trimmed.into()
        } else {
            line
        };
        if request.options.nonempty && value.is_empty() {
            removed_empty_lines += 1;
            continue;
        }
        let key = if request.options.fold {
            value.to_lowercase()
        } else {
            value.clone()
        };
        items.push(LineItem {
            value,
            key,
            first_index,
        });
    }

    let candidate_lines = items.len();
    let output_lines = match request.operation {
        Operation::Sort => {
            sort_items(
                &mut items,
                request.options.direction.unwrap_or(Direction::Ascending),
            );
            items.into_iter().map(|item| item.value).collect()
        }
        Operation::Unique => unique_items(items)
            .into_iter()
            .map(|item| item.value)
            .collect(),
        Operation::SortUnique => {
            let mut unique = unique_items(items);
            sort_items(
                &mut unique,
                request.options.direction.unwrap_or(Direction::Ascending),
            );
            unique.into_iter().map(|item| item.value).collect()
        }
        Operation::Reverse => items.into_iter().rev().map(|item| item.value).collect(),
        Operation::Frequency => frequency_lines(
            items,
            request.options.direction.unwrap_or(Direction::Descending),
        ),
    };
    let removed_duplicates = candidate_lines.saturating_sub(output_lines.len());
    let mut output = output_lines.join("\n");
    if split.final_newline && !output.is_empty() {
        output.push('\n');
    }
    if output.len() > MAX_OUTPUT_BYTES {
        return Err("processed output exceeds 256 KiB".into());
    }
    let report = build_report(
        request,
        input_lines,
        output_lines.len(),
        trimmed_lines,
        removed_empty_lines,
        removed_duplicates,
        split.final_newline,
    );
    let preview_output = compact(&visible_output(&output), MAX_PREVIEW_CHARACTERS);
    let preview = format!("{report}\n\nOutput\n{preview_output}");
    Ok(Transformation {
        operation: request.operation,
        options: request.options,
        input_lines,
        output_lines: output_lines.len(),
        trimmed_lines,
        removed_empty_lines,
        removed_duplicates,
        final_newline: split.final_newline,
        output,
        report,
        preview,
    })
}

fn sort_items(items: &mut [LineItem], direction: Direction) {
    items.sort_by(|left, right| match direction {
        Direction::Ascending => left.key.cmp(&right.key),
        Direction::Descending => right.key.cmp(&left.key),
    });
}

fn unique_items(items: Vec<LineItem>) -> Vec<LineItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.key.clone()))
        .collect()
}

fn frequency_lines(items: Vec<LineItem>, direction: Direction) -> Vec<String> {
    let mut positions = HashMap::<String, usize>::new();
    let mut counts = Vec::<(LineItem, usize)>::new();
    for item in items {
        if let Some(index) = positions.get(&item.key).copied() {
            counts[index].1 += 1;
        } else {
            positions.insert(item.key.clone(), counts.len());
            counts.push((item, 1));
        }
    }
    counts.sort_by(|(left_item, left_count), (right_item, right_count)| {
        let count_order = match direction {
            Direction::Ascending => left_count.cmp(right_count),
            Direction::Descending => right_count.cmp(left_count),
        };
        count_order.then_with(|| left_item.first_index.cmp(&right_item.first_index))
    });
    counts
        .into_iter()
        .map(|(item, count)| format!("{count}\t{}", item.value))
        .collect()
}

fn build_report(
    request: &Request,
    input_lines: usize,
    output_lines: usize,
    trimmed_lines: usize,
    removed_empty_lines: usize,
    removed_duplicates: usize,
    final_newline: bool,
) -> String {
    let order = if request.operation.supports_order() {
        Some(
            request
                .options
                .direction
                .unwrap_or(if request.operation == Operation::Frequency {
                    Direction::Descending
                } else {
                    Direction::Ascending
                })
                .label(),
        )
    } else {
        None
    };
    let options = [
        order.map(|value| format!("order {value}")),
        request
            .options
            .fold
            .then(|| "Unicode lowercase matching".into()),
        request.options.trim.then(|| "trim lines".into()),
        request
            .options
            .nonempty
            .then(|| "remove empty lines".into()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    [
        "Line Tools report".into(),
        format!("Operation: {}", request.operation.label()),
        format!(
            "Options: {}",
            if options.is_empty() {
                "none".into()
            } else {
                options.join(", ")
            }
        ),
        format!("Input lines: {input_lines}"),
        format!("Output lines: {output_lines}"),
        format!("Trimmed lines: {trimmed_lines}"),
        format!("Removed empty lines: {removed_empty_lines}"),
        format!("Consolidated duplicates: {removed_duplicates}"),
        format!(
            "Final newline: {}",
            if final_newline { "preserved" } else { "none" }
        ),
        "Output line endings: LF".into(),
    ]
    .join("\n")
}

fn transformation_result(transformation: &Transformation) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!(
            "{} · {} line{}",
            transformation.operation.result_label(),
            transformation.output_lines,
            if transformation.output_lines == 1 {
                ""
            } else {
                "s"
            }
        ),
        subtitle: format!(
            "{} input line{} · {} duplicate{} consolidated",
            transformation.input_lines,
            if transformation.input_lines == 1 {
                ""
            } else {
                "s"
            },
            transformation.removed_duplicates,
            if transformation.removed_duplicates == 1 {
                ""
            } else {
                "s"
            }
        ),
        preview: transformation.preview.clone(),
        kind: ResultKind::Command,
        score: 3_440.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "list.number".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste_output", "Paste Processed Lines", true, true, true),
            action("copy_output", "Copy Processed Lines", false, false, false),
            action("copy_report", "Copy Line Report", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "line-tools:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Line Tools unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Line Tools unavailable\n{}", safe_text(message)),
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

fn visible_output(value: &str) -> String {
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

fn safe_text(value: &str) -> String {
    visible_output(value)
        .replace('\n', "\\n")
        .replace('\t', "\\t")
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

    fn request(specification: &str, input: &str) -> Request {
        parse_request(&format!("linetools {specification} ::{input}"))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn parses_only_explicit_delimited_commands_and_options() {
        assert!(parse_request("ordinary sort :: b\na").unwrap().is_none());
        assert!(parse_request("lines of poetry").unwrap().is_none());
        assert!(parse_request("linetools sort b a").is_err());
        let parsed = parse_request("lines sort desc fold trim nonempty :: b\n A ")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.operation, Operation::Sort);
        assert_eq!(parsed.options.direction, Some(Direction::Descending));
        assert!(parsed.options.fold && parsed.options.trim && parsed.options.nonempty);
        assert_eq!(parsed.input, "b\n A ");
    }

    #[test]
    fn splits_lf_crlf_and_cr_while_preserving_final_newline_state() {
        let split = split_lines("a\r\nb\rc\n\n");
        assert_eq!(split.lines, ["a", "b", "c", ""]);
        assert!(split.final_newline);
        let no_final = split_lines("a\nb");
        assert_eq!(no_final.lines, ["a", "b"]);
        assert!(!no_final.final_newline);
    }

    #[test]
    fn sorts_ascending_descending_and_stably_with_folded_keys() {
        let ascending = transform(&request("sort", "z\na\nb")).unwrap();
        assert_eq!(ascending.output, "a\nb\nz");
        let descending = transform(&request("sort desc", "z\na\nb")).unwrap();
        assert_eq!(descending.output, "z\nb\na");
        let folded = transform(&request("sort fold", "B\nb\nA\na")).unwrap();
        assert_eq!(folded.output, "A\na\nB\nb");
    }

    #[test]
    fn deduplicates_in_first_seen_order_with_optional_folding() {
        let exact = transform(&request("unique", "A\na\nA\nb")).unwrap();
        assert_eq!(exact.output, "A\na\nb");
        assert_eq!(exact.removed_duplicates, 1);
        let folded = transform(&request("unique fold", "A\na\nB\nb")).unwrap();
        assert_eq!(folded.output, "A\nB");
        assert_eq!(folded.removed_duplicates, 2);
    }

    #[test]
    fn sorts_unique_trimmed_nonempty_lines() {
        let transformed = transform(&request(
            "sort-unique fold trim nonempty",
            " b \n\nA\na\n b",
        ))
        .unwrap();
        assert_eq!(transformed.output, "A\nb");
        assert_eq!(transformed.trimmed_lines, 2);
        assert_eq!(transformed.removed_empty_lines, 1);
        assert_eq!(transformed.removed_duplicates, 2);
    }

    #[test]
    fn reverses_lines_and_preserves_a_final_newline() {
        let transformed = transform(&request("reverse", "a\r\nb\n")).unwrap();
        assert_eq!(transformed.output, "b\na\n");
        assert!(transformed.final_newline);
        assert_eq!(transformed.output_lines, 2);
    }

    #[test]
    fn builds_frequency_tables_with_stable_ties_and_direction() {
        let descending = transform(&request(
            "frequency",
            "pear\napple\npear\nbanana\napple\npear",
        ))
        .unwrap();
        assert_eq!(descending.output, "3\tpear\n2\tapple\n1\tbanana");
        assert_eq!(descending.removed_duplicates, 3);
        let ascending = transform(&request("frequency asc fold", "A\na\nb\nC")).unwrap();
        assert_eq!(ascending.output, "1\tb\n1\tC\n2\tA");
    }

    #[test]
    fn rejects_unknown_duplicate_inapplicable_and_oversized_inputs() {
        assert!(parse_request("linetools sort nope :: a").is_err());
        assert!(parse_request("linetools sort asc desc :: a").is_err());
        assert!(parse_request("linetools reverse desc :: a").is_err());
        assert!(parse_request("linetools sort fold fold :: a").is_err());
        assert!(
            parse_request(&format!(
                "linetools sort ::{}",
                "x".repeat(MAX_INPUT_BYTES + 1)
            ))
            .is_err()
        );
        let many = format!("{}\n", "x\n".repeat(MAX_LINES));
        assert!(transform(&request("sort", &many)).is_err());
    }

    #[test]
    fn preview_escapes_controls_and_directional_characters_but_output_is_exact() {
        let transformed = transform(&request("sort", "b\u{202E}\na\0")).unwrap();
        assert!(transformed.output.contains('\u{202E}'));
        assert!(transformed.output.contains('\0'));
        assert!(!transformed.preview.contains('\u{202E}'));
        assert!(!transformed.preview.contains('\0'));
        assert!(transformed.preview.contains("\\u{202E}"));
        assert!(transformed.preview.contains("\\u{0000}"));
    }

    #[test]
    fn results_are_actionable_bounded_and_invalidate_stale_values() {
        let mut plugin = LineToolsPlugin::default();
        let result = plugin
            .search("linetools sort :: b\na", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, RESULT_ID);
        assert_eq!(result.actions.len(), 3);
        assert!(result.actions[0].default);
        assert_eq!(plugin.action_values["copy_output"], "a\nb");
        assert!(result.preview.chars().count() <= MAX_PREVIEW_CHARACTERS + 500);
        let invalid = plugin
            .search("linetools sort nope :: a", 1)
            .unwrap()
            .remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(plugin.search("linetools sort :: a", 0).unwrap().is_empty());
    }
}
