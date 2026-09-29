use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use serde_json::Value;
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.json-query";
const RESULT_ID: &str = "json-query:result";
const MAX_JSON_BYTES: usize = 128 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_LEAVES: usize = 200;
const MAX_DEPTH: usize = 64;
const MAX_PREVIEW_CHARACTERS: usize = 32 * 1024;
const MAX_LEAF_VALUE_CHARACTERS: usize = 240;

fn main() -> io::Result<()> {
    run_plugin(JsonQueryPlugin::default())
}

#[derive(Default)]
struct JsonQueryPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for JsonQueryPlugin {
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
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let result = match evaluate(&request) {
            Ok(result) => result,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        self.action_values = result.action_values.clone();
        Ok(vec![search_result(result)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("JSON query is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive JSON Query action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "copy_path" => "JSON path copied",
                "paste_value" | "copy_value" => "JSON query result copied",
                _ => "JSON query output copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Get,
    Keys,
    Paths,
}

impl Operation {
    fn parse(command: &str) -> Option<Self> {
        match command.to_ascii_lowercase().as_str() {
            "jget" | "json-get" => Some(Self::Get),
            "jkeys" | "json-keys" => Some(Self::Keys),
            "jpaths" | "json-paths" => Some(Self::Paths),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    path: String,
    json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathToken {
    Key(String),
    Index(usize),
    Pointer(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Evaluation {
    title: String,
    subtitle: String,
    preview: String,
    action_values: BTreeMap<String, String>,
    actions: Vec<ResultAction>,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let Some(operation) = Operation::parse(&query[..command_end]) else {
        return Ok(None);
    };
    let remainder = &query[command_end..];
    let arguments = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    let (path, json) = if let Some(json) = arguments.strip_prefix(":: ") {
        ("", json)
    } else {
        arguments.split_once(" :: ").ok_or_else(|| {
            "expected '<path> :: <json>'; leave the path empty to query the root".to_owned()
        })?
    };
    let path = path.trim();
    let json = json.trim();
    if path.len() > MAX_PATH_BYTES {
        return Err("JSON path exceeds 4 KiB".into());
    }
    if json.is_empty() {
        return Err("JSON input cannot be empty".into());
    }
    if json.len() > MAX_JSON_BYTES {
        return Err("JSON input exceeds 128 KiB".into());
    }
    Ok(Some(Request {
        operation,
        path: path.to_owned(),
        json: json.to_owned(),
    }))
}

fn evaluate(request: &Request) -> Result<Evaluation, String> {
    let root: Value = serde_json::from_str(&request.json).map_err(|error| {
        format!(
            "invalid JSON at line {}, column {}: {}",
            error.line(),
            error.column(),
            error
        )
    })?;
    let tokens = parse_path(&request.path)?;
    let canonical_path = canonical_pointer(&tokens);
    let selected = resolve(&root, &tokens)?;
    match request.operation {
        Operation::Get => evaluate_get(selected, &canonical_path),
        Operation::Keys => evaluate_keys(selected, &canonical_path),
        Operation::Paths => evaluate_paths(selected, &canonical_path),
    }
}

fn evaluate_get(value: &Value, path: &str) -> Result<Evaluation, String> {
    let copy_value = copy_value(value)?;
    let pretty = pretty_value(value)?;
    let mut action_values = BTreeMap::from([
        ("paste_value".into(), copy_value.clone()),
        ("copy_value".into(), copy_value),
        ("copy_path".into(), path.to_owned()),
    ]);
    if path.is_empty() {
        action_values.remove("copy_path");
    }
    let mut actions = vec![
        action("paste_value", "Paste Value", true, true),
        action("copy_value", "Copy Value", false, false),
    ];
    if !path.is_empty() {
        actions.push(action("copy_path", "Copy JSON Pointer", false, false));
    }
    Ok(Evaluation {
        title: format!("JSON Value · {}", compact(display_path(path), 160)),
        subtitle: format!("{} · local JSON query", type_label(value)),
        preview: compact(&pretty, MAX_PREVIEW_CHARACTERS),
        action_values,
        actions,
    })
}

fn evaluate_keys(value: &Value, path: &str) -> Result<Evaluation, String> {
    let entries: Vec<(String, String)> = match value {
        Value::Object(object) => object
            .keys()
            .map(|key| (key.clone(), append_pointer(path, key)))
            .collect(),
        Value::Array(array) => (0..array.len())
            .map(|index| (index.to_string(), append_pointer(path, &index.to_string())))
            .collect(),
        _ => return Err(format!("{} has no child keys", display_path(path))),
    };
    let copy = entries
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let preview = if entries.is_empty() {
        "No child keys.".into()
    } else {
        entries
            .iter()
            .map(|(key, pointer)| format!("{} · {pointer}", compact(key, 240)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut action_values = BTreeMap::from([
        ("paste_output".into(), copy.clone()),
        ("copy_output".into(), copy),
        ("copy_path".into(), path.to_owned()),
    ]);
    let mut actions = if entries.is_empty() {
        Vec::new()
    } else {
        vec![
            action("paste_output", "Paste Keys", true, true),
            action("copy_output", "Copy Keys", false, false),
        ]
    };
    if !path.is_empty() {
        actions.push(action("copy_path", "Copy JSON Pointer", false, false));
    } else {
        action_values.remove("copy_path");
    }
    Ok(Evaluation {
        title: format!("JSON Keys · {}", compact(display_path(path), 160)),
        subtitle: format!("{} child keys · local JSON query", entries.len()),
        preview: compact(&preview, MAX_PREVIEW_CHARACTERS),
        action_values,
        actions,
    })
}

fn evaluate_paths(value: &Value, path: &str) -> Result<Evaluation, String> {
    let mut leaves = Vec::new();
    let mut truncated = false;
    collect_leaves(value, path, 0, &mut leaves, &mut truncated)?;
    let mut output = leaves
        .iter()
        .map(|(pointer, value)| {
            format!(
                "{} = {}",
                display_path(pointer),
                compact(&inline_value(value), MAX_LEAF_VALUE_CHARACTERS)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if truncated {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("… limited to {MAX_LEAVES} leaf paths"));
    }
    if output.is_empty() {
        output.push_str("No leaf paths.");
    }
    let mut action_values = BTreeMap::from([
        ("paste_output".into(), output.clone()),
        ("copy_output".into(), output.clone()),
        ("copy_path".into(), path.to_owned()),
    ]);
    let mut actions = vec![
        action("paste_output", "Paste Leaf Paths", true, true),
        action("copy_output", "Copy Leaf Paths", false, false),
    ];
    if !path.is_empty() {
        actions.push(action("copy_path", "Copy JSON Pointer", false, false));
    } else {
        action_values.remove("copy_path");
    }
    Ok(Evaluation {
        title: format!("JSON Paths · {}", compact(display_path(path), 160)),
        subtitle: format!(
            "{} leaf paths{} · local JSON query",
            leaves.len(),
            if truncated { " (first 200)" } else { "" }
        ),
        preview: compact(&output, MAX_PREVIEW_CHARACTERS),
        action_values,
        actions,
    })
}

fn parse_path(path: &str) -> Result<Vec<PathToken>, String> {
    let path = path.trim();
    if path.is_empty() || path == "$" {
        return Ok(Vec::new());
    }
    if let Some(pointer) = path.strip_prefix('/') {
        return pointer
            .split('/')
            .map(decode_pointer_token)
            .map(|token| token.map(PathToken::Pointer))
            .collect();
    }
    parse_dot_path(path)
}

fn parse_dot_path(path: &str) -> Result<Vec<PathToken>, String> {
    let mut rest = path;
    if let Some(after_root) = rest.strip_prefix('$') {
        if !after_root.is_empty() && !after_root.starts_with(['.', '[']) {
            return Err("expected '.' or '[' after '$'".into());
        }
        rest = after_root;
    }
    let mut tokens = Vec::new();
    let mut need_separator = false;
    while !rest.is_empty() {
        if let Some(after_dot) = rest.strip_prefix('.') {
            if after_dot.is_empty() || after_dot.starts_with(['.', '[']) {
                return Err("empty key in JSON dot path".into());
            }
            rest = after_dot;
        } else if need_separator && !rest.starts_with('[') {
            return Err("expected '.' or '[' between JSON path segments".into());
        }
        if rest.starts_with('[') {
            let (token, consumed) = parse_bracket(rest)?;
            tokens.push(token);
            rest = &rest[consumed..];
            need_separator = true;
            continue;
        }
        let end = rest
            .char_indices()
            .find_map(|(index, character)| matches!(character, '.' | '[').then_some(index))
            .unwrap_or(rest.len());
        let key = &rest[..end];
        if key.is_empty() {
            return Err("empty key in JSON dot path".into());
        }
        tokens.push(PathToken::Key(key.to_owned()));
        rest = &rest[end..];
        need_separator = true;
    }
    Ok(tokens)
}

fn parse_bracket(input: &str) -> Result<(PathToken, usize), String> {
    let inner = &input[1..];
    if let Some(after_quote) = inner.strip_prefix('"') {
        let mut escaped = false;
        let mut quote_end = None;
        for (index, character) in after_quote.char_indices() {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quote_end = Some(index + 2);
                break;
            }
        }
        let quote_end = quote_end.ok_or_else(|| "unterminated quoted JSON path key".to_owned())?;
        if input.as_bytes().get(quote_end + 1) != Some(&b']') {
            return Err("expected ']' after quoted JSON path key".into());
        }
        let quoted = &input[1..=quote_end];
        let key: String = serde_json::from_str(quoted)
            .map_err(|error| format!("invalid quoted JSON path key: {error}"))?;
        return Ok((PathToken::Key(key), quote_end + 2));
    }
    let close = inner
        .find(']')
        .ok_or_else(|| "unterminated JSON array index".to_owned())?;
    let value = &inner[..close];
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err("JSON array index must be a non-negative integer".into());
    }
    let index = value
        .parse::<usize>()
        .map_err(|_| "JSON array index is too large".to_owned())?;
    Ok((PathToken::Index(index), close + 2))
}

fn decode_pointer_token(token: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => return Err("invalid JSON Pointer escape; use '~0' or '~1'".into()),
        }
    }
    Ok(decoded)
}

fn resolve<'a>(root: &'a Value, tokens: &[PathToken]) -> Result<&'a Value, String> {
    let mut current = root;
    let mut traversed = String::new();
    for token in tokens {
        let segment = match token {
            PathToken::Key(key) | PathToken::Pointer(key) => key.as_str(),
            PathToken::Index(index) => {
                let segment = index.to_string();
                traversed = append_pointer(&traversed, &segment);
                current = current
                    .as_array()
                    .ok_or_else(|| format!("{} is not an array", parent_path(&traversed)))?
                    .get(*index)
                    .ok_or_else(|| {
                        format!(
                            "array index {index} is out of range at {}",
                            parent_path(&traversed)
                        )
                    })?;
                continue;
            }
        };
        traversed = append_pointer(&traversed, segment);
        current = match (token, current) {
            (PathToken::Pointer(_), Value::Array(array)) => {
                let index = segment.parse::<usize>().map_err(|_| {
                    format!(
                        "array segment '{segment}' is not an index at {}",
                        parent_path(&traversed)
                    )
                })?;
                array.get(index).ok_or_else(|| {
                    format!(
                        "array index {index} is out of range at {}",
                        parent_path(&traversed)
                    )
                })?
            }
            (_, Value::Object(object)) => object.get(segment).ok_or_else(|| {
                format!(
                    "object key '{segment}' was not found at {}",
                    parent_path(&traversed)
                )
            })?,
            (_, other) => {
                return Err(format!(
                    "{} is {}, not an object",
                    parent_path(&traversed),
                    type_label(other)
                ));
            }
        };
    }
    Ok(current)
}

fn canonical_pointer(tokens: &[PathToken]) -> String {
    tokens.iter().fold(String::new(), |path, token| {
        let segment = match token {
            PathToken::Key(key) | PathToken::Pointer(key) => key.clone(),
            PathToken::Index(index) => index.to_string(),
        };
        append_pointer(&path, &segment)
    })
}

fn append_pointer(path: &str, segment: &str) -> String {
    format!("{path}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or(
        "(root)",
        |(parent, _)| {
            if parent.is_empty() { "(root)" } else { parent }
        },
    )
}

fn collect_leaves<'a>(
    value: &'a Value,
    path: &str,
    depth: usize,
    output: &mut Vec<(String, &'a Value)>,
    truncated: &mut bool,
) -> Result<(), String> {
    if output.len() == MAX_LEAVES {
        *truncated = true;
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(format!("JSON nesting exceeds {MAX_DEPTH} levels"));
    }
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, child) in object {
                collect_leaves(
                    child,
                    &append_pointer(path, key),
                    depth + 1,
                    output,
                    truncated,
                )?;
                if *truncated {
                    break;
                }
            }
        }
        Value::Array(array) if !array.is_empty() => {
            for (index, child) in array.iter().enumerate() {
                collect_leaves(
                    child,
                    &append_pointer(path, &index.to_string()),
                    depth + 1,
                    output,
                    truncated,
                )?;
                if *truncated {
                    break;
                }
            }
        }
        _ => output.push((path.to_owned(), value)),
    }
    Ok(())
}

fn copy_value(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Object(_) | Value::Array(_) => pretty_value(value),
        _ => Ok(value.to_string()),
    }
}

fn pretty_value(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

fn inline_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".into())
}

fn type_label(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn display_path(path: &str) -> &str {
    if path.is_empty() { "(root)" } else { path }
}

fn search_result(evaluation: Evaluation) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: evaluation.title,
        subtitle: evaluation.subtitle,
        preview: evaluation.preview,
        kind: ResultKind::Command,
        score: 3_402.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "curlybraces".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: evaluation.actions,
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "json-query:invalid".into(),
        title: "JSON Query".into(),
        subtitle: format!("Invalid JSON query · {}", compact(message, 160)),
        preview: format!("Invalid JSON query\n{message}"),
        kind: ResultKind::Command,
        score: 3_402.0,
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
        host_action: None,
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window: paste_to_previous,
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

    fn request(operation: Operation, path: &str, json: &str) -> Request {
        Request {
            operation,
            path: path.into(),
            json: json.into(),
        }
    }

    #[test]
    fn parses_commands_without_conflicting_with_plain_json_formatting() {
        assert_eq!(
            parse_request(r#"jget user.name :: {"user":{"name":"Ada"}}"#).unwrap(),
            Some(request(
                Operation::Get,
                "user.name",
                r#"{"user":{"name":"Ada"}}"#
            ))
        );
        assert_eq!(
            parse_request(r#"json-keys: :: {"a":1}"#).unwrap(),
            Some(request(Operation::Keys, "", r#"{"a":1}"#))
        );
        assert_eq!(parse_request(r#"json {"a":1}"#).unwrap(), None);
        assert!(parse_request("jpaths missing separator").is_err());
    }

    #[test]
    fn parses_json_pointer_escapes_and_dot_bracket_paths() {
        assert_eq!(
            parse_path("/a~1b/~0key/0").unwrap(),
            vec![
                PathToken::Pointer("a/b".into()),
                PathToken::Pointer("~key".into()),
                PathToken::Pointer("0".into())
            ]
        );
        assert_eq!(
            parse_path(r#"$.users[0]["first.name"]"#).unwrap(),
            vec![
                PathToken::Key("users".into()),
                PathToken::Index(0),
                PathToken::Key("first.name".into())
            ]
        );
        assert!(parse_path("/bad~2escape").is_err());
        assert!(parse_path("$users").is_err());
    }

    #[test]
    fn gets_scalars_and_objects_with_canonical_paths() {
        let scalar = evaluate(&request(
            Operation::Get,
            "users[0].name",
            r#"{"users":[{"name":"Ada"}]}"#,
        ))
        .unwrap();
        assert_eq!(scalar.title, "JSON Value · /users/0/name");
        assert_eq!(scalar.subtitle, "string · local JSON query");
        assert_eq!(scalar.action_values["copy_value"], "Ada");
        assert_eq!(scalar.action_values["copy_path"], "/users/0/name");

        let object = evaluate(&request(Operation::Get, "", r#"{"b":2,"a":1}"#)).unwrap();
        assert_eq!(object.title, "JSON Value · (root)");
        assert!(object.action_values["copy_value"].contains("\n"));
        assert!(!object.action_values.contains_key("copy_path"));
    }

    #[test]
    fn resolves_pointer_segments_by_container_type() {
        let root: Value = serde_json::from_str(r#"{"0":"object","items":["array"]}"#).unwrap();
        assert_eq!(
            resolve(&root, &parse_path("/0").unwrap()).unwrap(),
            "object"
        );
        assert_eq!(
            resolve(&root, &parse_path("/items/0").unwrap()).unwrap(),
            "array"
        );
        assert!(resolve(&root, &parse_path("/items/name").unwrap()).is_err());
    }

    #[test]
    fn lists_object_keys_and_array_indices() {
        let object = evaluate(&request(
            Operation::Keys,
            "/user",
            r#"{"user":{"a":1,"b":2}}"#,
        ))
        .unwrap();
        assert_eq!(object.action_values["copy_output"], "a\nb");
        assert!(object.preview.contains("a · /user/a"));

        let array = evaluate(&request(Operation::Keys, "items", r#"{"items":[1,2]}"#)).unwrap();
        assert_eq!(array.action_values["copy_output"], "0\n1");
        assert!(evaluate(&request(Operation::Keys, "", "42")).is_err());
    }

    #[test]
    fn enumerates_escaped_leaf_paths_and_limits_output() {
        let result = evaluate(&request(
            Operation::Paths,
            "",
            r#"{"a/b":{"~key":true},"empty":[]}"#,
        ))
        .unwrap();
        assert!(result.preview.contains("/a~1b/~0key = true"));
        assert!(result.preview.contains("/empty = []"));

        let values = Value::Array((0..205).map(Value::from).collect());
        let result = evaluate(&request(Operation::Paths, "", &values.to_string())).unwrap();
        assert!(result.subtitle.contains("first 200"));
        assert!(result.preview.contains("limited to 200 leaf paths"));
    }

    #[test]
    fn visible_errors_have_no_actions() {
        let mut plugin = JsonQueryPlugin::default();
        let invalid_json = plugin.search("jget :: {bad}", 20).unwrap().remove(0);
        assert!(invalid_json.subtitle.starts_with("Invalid JSON query · "));
        assert!(invalid_json.actions.is_empty());
        assert_eq!(invalid_json.metadata["symbol_color"], "#FF9F0A");

        let missing = plugin
            .search(r#"jget /missing :: {"a":1}"#, 20)
            .unwrap()
            .remove(0);
        assert!(missing.preview.contains("was not found"));
        assert!(missing.actions.is_empty());
    }

    #[test]
    fn results_are_rankable_and_invalidate_old_actions() {
        let mut plugin = JsonQueryPlugin::default();
        let result = plugin
            .search(r#"jget /name :: {"name":"Ada"}"#, 20)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, RESULT_ID);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(plugin.action_values["copy_value"], "Ada");

        plugin.search("ordinary text", 20).unwrap();
        assert!(plugin.action_values.is_empty());
    }

    #[test]
    fn bounds_and_compaction_are_unicode_safe() {
        assert_eq!(compact("甲乙丙丁", 3), "甲乙丙…");
        let long_path = "a".repeat(MAX_PATH_BYTES + 1);
        assert!(parse_request(&format!("jget {long_path} :: null")).is_err());
        let long_json = format!("\"{}\"", "x".repeat(MAX_JSON_BYTES));
        assert!(parse_request(&format!("jget :: {long_json}")).is_err());

        let long_key = "甲".repeat(200);
        let json = serde_json::json!({ long_key.clone(): true }).to_string();
        let result = evaluate(&request(Operation::Get, &long_key, &json)).unwrap();
        assert!(result.title.ends_with('…'));
        assert_eq!(result.action_values["copy_path"].chars().count(), 201);
    }
}
