use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.markdown-tools";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_ARGUMENT_BYTES: usize = 8 * 1024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(MarkdownToolsPlugin::default())
}

#[derive(Default)]
struct MarkdownToolsPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for MarkdownToolsPlugin {
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
        let output = match transform(&request) {
            Ok(output) => output,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!("markdown-tools:{}", request.operation.id());
        self.result_values.insert(result_id.clone(), output.clone());
        Ok(vec![markdown_result(result_id, &request, output)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Markdown Tools action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "Markdown result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Markdown result copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Slug,
    Escape,
    Link,
    Image,
    Code,
    Quote,
}

impl Operation {
    fn parse(command: &str) -> Option<Self> {
        match command.to_ascii_lowercase().as_str() {
            "mdslug" | "markdown-slug" => Some(Self::Slug),
            "mdescape" | "markdown-escape" => Some(Self::Escape),
            "mdlink" | "markdown-link" => Some(Self::Link),
            "mdimage" | "markdown-image" => Some(Self::Image),
            "mdcode" | "markdown-code" => Some(Self::Code),
            "mdquote" | "markdown-quote" => Some(Self::Quote),
            _ if command == "Markdown锚点" => Some(Self::Slug),
            _ if command == "Markdown转义" => Some(Self::Escape),
            _ if command == "Markdown链接" => Some(Self::Link),
            _ if command == "Markdown图片" => Some(Self::Image),
            _ if command == "Markdown代码块" => Some(Self::Code),
            _ if command == "Markdown引用" => Some(Self::Quote),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Slug => "slug",
            Self::Escape => "escape",
            Self::Link => "link",
            Self::Image => "image",
            Self::Code => "code",
            Self::Quote => "quote",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Slug => "Markdown Anchor Slug",
            Self::Escape => "Escaped Markdown Text",
            Self::Link => "Markdown Link",
            Self::Image => "Markdown Image",
            Self::Code => "Markdown Code Block",
            Self::Quote => "Markdown Block Quote",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    argument: String,
    input: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let Some(operation) = Operation::parse(command) else {
        return Ok(None);
    };
    if query.len() > MAX_INPUT_BYTES + MAX_ARGUMENT_BYTES + 1024 {
        return Err("Markdown Tools query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let (argument, input) = split_arguments(remainder)?;
    if argument.len() > MAX_ARGUMENT_BYTES {
        return Err("Markdown option or destination exceeds 8 KiB".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("Markdown input exceeds 64 KiB".into());
    }

    match operation {
        Operation::Link | Operation::Image => {
            if argument.trim().is_empty() {
                return Err("a link or image destination is required before '::'".into());
            }
        }
        Operation::Code => validate_info_string(argument.trim())?,
        _ if !argument.trim().is_empty() => {
            return Err(format!(
                "{} does not accept options before '::'",
                operation.label()
            ));
        }
        _ => {}
    }
    if operation == Operation::Slug && input.trim().is_empty() {
        return Err("heading text cannot be empty".into());
    }
    if operation == Operation::Link && input.is_empty() {
        return Err("link label cannot be empty".into());
    }

    Ok(Some(Request {
        operation,
        argument: argument.trim().to_owned(),
        input: input.to_owned(),
    }))
}

fn split_arguments(remainder: &str) -> Result<(&str, &str), String> {
    if remainder == "::" {
        return Ok(("", ""));
    }
    if let Some(input) = remainder.strip_prefix(":: ") {
        return Ok(("", input));
    }
    if let Some(argument) = remainder.strip_suffix(" ::") {
        return Ok((argument, ""));
    }
    remainder
        .split_once(" :: ")
        .ok_or_else(|| "expected '[option or destination] :: <text>'".to_owned())
}

fn validate_info_string(info: &str) -> Result<(), String> {
    if info.len() > 64 {
        return Err("code-block language exceeds 64 bytes".into());
    }
    if !info.chars().all(|character| {
        character.is_alphanumeric() || matches!(character, '_' | '-' | '+' | '.' | '#')
    }) {
        return Err(
            "code-block language may contain only letters, numbers, '_', '-', '+', '.', or '#'"
                .into(),
        );
    }
    Ok(())
}

fn transform(request: &Request) -> Result<String, String> {
    match request.operation {
        Operation::Slug => slugify(&request.input),
        Operation::Escape => Ok(escape_markdown(&request.input)),
        Operation::Link => markdown_link(&request.input, &request.argument, false),
        Operation::Image => markdown_link(&request.input, &request.argument, true),
        Operation::Code => Ok(code_block(&request.input, &request.argument)),
        Operation::Quote => Ok(block_quote(&request.input)),
    }
}

fn slugify(input: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut pending_separator = false;
    for character in input.trim().chars() {
        if character.is_alphanumeric() {
            if pending_separator && !output.is_empty() {
                output.push('-');
            }
            output.extend(character.to_lowercase());
            pending_separator = false;
        } else if character.is_whitespace() || matches!(character, '-' | '_') {
            pending_separator = !output.is_empty();
        }
    }
    if output.is_empty() {
        Err("heading does not contain any letters or numbers".into())
    } else {
        Ok(output)
    }
}

fn escape_markdown(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        if character.is_ascii_punctuation() {
            output.push('\\');
        }
        output.push(character);
    }
    output
}

fn markdown_link(label: &str, destination: &str, image: bool) -> Result<String, String> {
    let destination = safe_destination(destination)?;
    let prefix = if image { "!" } else { "" };
    Ok(format!(
        "{prefix}[{}](<{destination}>)",
        escape_markdown(label)
    ))
}

fn safe_destination(destination: &str) -> Result<String, String> {
    let mut output = String::with_capacity(destination.len());
    for character in destination.chars() {
        if character.is_control() || is_directional_control(character) {
            return Err("link destination cannot contain control or directional characters".into());
        }
        match character {
            ' ' => output.push_str("%20"),
            '<' => output.push_str("%3C"),
            '>' => output.push_str("%3E"),
            '\\' => output.push_str("%5C"),
            _ => output.push(character),
        }
    }
    Ok(output)
}

fn code_block(input: &str, info: &str) -> String {
    let fence = "`".repeat(longest_run(input, '`').saturating_add(1).max(3));
    let mut output = format!("{fence}{info}\n{input}");
    if !input.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&fence);
    output
}

fn longest_run(input: &str, needle: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in input.chars() {
        if character == needle {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn block_quote(input: &str) -> String {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown_result(result_id: String, request: &Request, output: String) -> SearchResult {
    let visible = visible_text(&compact(&output, MAX_VISIBLE_CHARACTERS));
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&safe_text(&output), 180),
        subtitle: format!(
            "{} · {} characters · local",
            request.operation.label(),
            output.chars().count()
        ),
        preview: format!(
            "{}\nInput: {} characters · {} bytes\nOutput: {} characters · {} bytes\n\nResult\n{visible}",
            request.operation.label(),
            request.input.chars().count(),
            request.input.len(),
            output.chars().count(),
            output.len()
        ),
        kind: ResultKind::Command,
        score: 3_350.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "text.document".into()),
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
        id: "markdown-tools:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Markdown transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!(
            "Markdown transformation unavailable\n{}",
            safe_text(message)
        ),
        kind: ResultKind::Command,
        score: 3_350.0,
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
    visible_text(value)
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn visible_text(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if matches!(character, '\n' | '\t') {
            output.push(character);
        } else if character.is_control() || is_directional_control(character) {
            output.push_str(&format!("\\u{{{:04X}}}", character as u32));
        } else {
            output.push(character);
        }
    }
    output
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
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
    fn parses_commands_arguments_and_chinese_aliases() {
        assert!(parse_request("markdown document").unwrap().is_none());
        let link = parse_request("mdlink https://example.com/a b :: A [link]")
            .unwrap()
            .unwrap();
        assert_eq!(link.operation, Operation::Link);
        assert_eq!(link.argument, "https://example.com/a b");
        assert_eq!(link.input, "A [link]");
        assert_eq!(
            parse_request("Markdown代码块: rust :: fn main() {}")
                .unwrap()
                .unwrap()
                .argument,
            "rust"
        );
    }

    #[test]
    fn creates_readable_unicode_slugs() {
        assert_eq!(
            slugify("  Hello,  世界 — Rust_2026! ").unwrap(),
            "hello-世界-rust-2026"
        );
        assert_eq!(slugify("What's New?").unwrap(), "whats-new");
        assert!(slugify("✨ ---").is_err());
    }

    #[test]
    fn escapes_all_commonmark_ascii_punctuation() {
        assert_eq!(
            escape_markdown("# [A&B] *x* / y."),
            "\\# \\[A\\&B\\] \\*x\\* \\/ y\\."
        );
        assert_eq!(escape_markdown("中文 😀"), "中文 😀");
    }

    #[test]
    fn creates_links_and_images_with_safe_destinations() {
        assert_eq!(
            markdown_link("A [link]", "https://example.com/a b?q=<x>", false).unwrap(),
            "[A \\[link\\]](<https://example.com/a%20b?q=%3Cx%3E>)"
        );
        assert_eq!(
            markdown_link("", "images/photo one.png", true).unwrap(),
            "![](<images/photo%20one.png>)"
        );
        assert!(markdown_link("x", "https://example.com/\nnext", false).is_err());
        assert!(markdown_link("x", "https://example.com/\u{202e}", false).is_err());
    }

    #[test]
    fn code_blocks_choose_a_nonconflicting_fence() {
        assert_eq!(code_block("let x = 1;", "rust"), "```rust\nlet x = 1;\n```");
        assert_eq!(
            code_block("before ``` after\n", ""),
            "````\nbefore ``` after\n````"
        );
    }

    #[test]
    fn quotes_multiline_text_and_normalizes_line_endings() {
        assert_eq!(block_quote("one\r\n\r\ntwo\r"), "> one\n>\n> two\n>");
        assert_eq!(block_quote(""), ">");
    }

    #[test]
    fn rejects_missing_delimiters_bad_arguments_and_languages() {
        assert!(parse_request("mdslug heading").is_err());
        assert!(parse_request("mdescape option :: text").is_err());
        assert!(parse_request("mdlink :: label").is_err());
        assert!(parse_request("mdlink https://example.com ::").is_err());
        assert!(parse_request("mdcode rust script :: code").is_err());
        assert!(parse_request("mdcode `rust` :: code").is_err());
        assert!(parse_request("mdslug ::").is_err());
    }

    #[test]
    fn permits_empty_escape_image_alt_code_and_quote_inputs() {
        assert_eq!(
            transform(&parse_request("mdescape ::").unwrap().unwrap()).unwrap(),
            ""
        );
        assert_eq!(
            transform(&parse_request("mdimage icon.png ::").unwrap().unwrap()).unwrap(),
            "![](<icon.png>)"
        );
        assert_eq!(
            transform(&parse_request("mdcode text ::").unwrap().unwrap()).unwrap(),
            "```text\n\n```"
        );
        assert_eq!(
            transform(&parse_request("mdquote ::").unwrap().unwrap()).unwrap(),
            ">"
        );
    }

    #[test]
    fn enforces_query_input_argument_and_language_limits() {
        assert!(
            parse_request(&format!("mdescape :: {}", "x".repeat(MAX_INPUT_BYTES + 1))).is_err()
        );
        assert!(
            parse_request(&format!(
                "mdlink {} :: label",
                "x".repeat(MAX_ARGUMENT_BYTES + 1)
            ))
            .is_err()
        );
        assert!(parse_request(&format!("mdcode {} :: body", "x".repeat(65))).is_err());
    }

    #[test]
    fn results_are_actionable_and_searches_clear_stale_values() {
        let mut plugin = MarkdownToolsPlugin::default();
        let result = plugin
            .search("mdlink https://example.com :: Docs", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, "markdown-tools:link");
        assert_eq!(
            plugin.result_values[&result.id],
            "[Docs](<https://example.com>)"
        );
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);

        let invalid = plugin.search("mdslug :: ✨", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
        assert!(plugin.search("mdescape :: text", 0).unwrap().is_empty());
    }

    #[test]
    fn controls_and_bidi_are_safe_in_visible_fields_but_output_is_exact() {
        let mut plugin = MarkdownToolsPlugin::default();
        let result = plugin
            .search("mdquote :: line\ntext\u{202e}\u{0001}", 1)
            .unwrap()
            .remove(0);
        assert_eq!(
            plugin.result_values[&result.id],
            "> line\n> text\u{202e}\u{0001}"
        );
        assert!(result.title.contains("\\n"));
        assert!(result.title.contains("\\u{202E}"));
        assert!(result.preview.contains("> line\n> text\\u{202E}\\u{0001}"));
        assert!(!result.preview.contains('\u{202e}'));
        assert!(!result.preview.contains('\u{0001}'));
    }
}
