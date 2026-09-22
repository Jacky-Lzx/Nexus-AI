use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.path-tools";
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(PathToolsPlugin::default())
}

#[derive(Default)]
struct PathToolsPlugin {
    active_result_id: Option<String>,
    action_values: BTreeMap<String, String>,
}

impl Plugin for PathToolsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_result_id = None;
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let evaluation = match evaluate(&request) {
            Ok(evaluation) => evaluation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!("path-tools:{}", request.operation.id());
        self.active_result_id = Some(result_id.clone());
        self.action_values = evaluation.action_values.clone();
        Ok(vec![path_result(result_id, evaluation)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if self.active_result_id.as_deref() != Some(result_id) {
            return Err("path result is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive Path Tools action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            if action_id == "paste_result" {
                "Path result copied for pasting"
            } else {
                "Path value copied"
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Info,
    Normalize,
    Join,
    Relative,
    Extension,
}

impl Operation {
    fn parse(command: &str) -> Option<Self> {
        match command.to_ascii_lowercase().as_str() {
            "pathinfo" | "path-info" => Some(Self::Info),
            "pathnorm" | "pathnormalize" | "path-normalize" => Some(Self::Normalize),
            "pathjoin" | "path-join" => Some(Self::Join),
            "pathrelative" | "pathrel" | "path-relative" => Some(Self::Relative),
            "pathext" | "path-extension" => Some(Self::Extension),
            _ if command == "路径信息" => Some(Self::Info),
            _ if command == "路径规范化" => Some(Self::Normalize),
            _ if command == "路径拼接" => Some(Self::Join),
            _ if command == "相对路径" => Some(Self::Relative),
            _ if command == "路径扩展名" => Some(Self::Extension),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Normalize => "normalize",
            Self::Join => "join",
            Self::Relative => "relative",
            Self::Extension => "extension",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Info => "POSIX Path Information",
            Self::Normalize => "Normalized POSIX Path",
            Self::Join => "Joined POSIX Path",
            Self::Relative => "Relative POSIX Path",
            Self::Extension => "Updated Path Extension",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    argument: String,
    path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathInspection {
    normalized: String,
    absolute: bool,
    parent: String,
    name: String,
    stem: String,
    extension: Option<String>,
    component_count: usize,
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
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let Some(operation) = Operation::parse(command) else {
        return Ok(None);
    };
    if query.len() > MAX_PATH_BYTES * 2 + 1024 {
        return Err("Path Tools query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let (argument, path) = split_arguments(remainder)?;
    let argument = argument.trim();
    let path = path.trim();
    validate_path(path, "path")?;
    if argument.len() > MAX_PATH_BYTES {
        return Err("Path Tools argument exceeds 16 KiB".into());
    }

    match operation {
        Operation::Info | Operation::Normalize if !argument.is_empty() => {
            return Err(format!(
                "{} does not accept a value before '::'",
                operation.label()
            ));
        }
        Operation::Join | Operation::Relative => validate_path(argument, "base path")?,
        Operation::Extension => validate_extension_argument(argument)?,
        _ => {}
    }

    Ok(Some(Request {
        operation,
        argument: argument.to_owned(),
        path: path.to_owned(),
    }))
}

fn split_arguments(remainder: &str) -> Result<(&str, &str), String> {
    if let Some(path) = remainder.strip_prefix(":: ") {
        return Ok(("", path));
    }
    remainder
        .split_once(" :: ")
        .ok_or_else(|| "expected '[base or extension] :: <path>'".to_owned())
}

fn validate_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(format!("{label} exceeds 16 KiB"));
    }
    if path
        .chars()
        .any(|character| character.is_control() || is_directional_control(character))
    {
        return Err(format!(
            "{label} cannot contain control or directional characters"
        ));
    }
    Ok(())
}

fn validate_extension_argument(extension: &str) -> Result<(), String> {
    if extension.is_empty() {
        return Err("an extension or 'none' is required before '::'".into());
    }
    let lowered = extension.to_ascii_lowercase();
    if matches!(lowered.as_str(), "none" | "remove" | "-") {
        return Ok(());
    }
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    if extension.is_empty()
        || extension == "."
        || extension == ".."
        || extension.contains('/')
        || extension
            .chars()
            .any(|character| character.is_control() || is_directional_control(character))
    {
        return Err("extension must be a nonempty path-free value or 'none'".into());
    }
    Ok(())
}

fn evaluate(request: &Request) -> Result<Evaluation, String> {
    match request.operation {
        Operation::Info => evaluate_info(&request.path),
        Operation::Normalize => Ok(evaluate_output(
            request.operation,
            normalize(&request.path),
            "Repeated separators and '.'/'..' components resolved lexically",
        )),
        Operation::Join => {
            if request.path.starts_with('/') {
                return Err("child path must be relative when joining".into());
            }
            let joined = normalize(&format!("{}/{}", request.argument, request.path));
            Ok(evaluate_output(
                request.operation,
                joined,
                "Base and relative child joined, then normalized lexically",
            ))
        }
        Operation::Relative => {
            let relative = relative_path(&request.argument, &request.path)?;
            Ok(evaluate_output(
                request.operation,
                relative,
                "First path is treated as the source directory",
            ))
        }
        Operation::Extension => {
            let updated = replace_extension(&request.path, &request.argument)?;
            Ok(evaluate_output(
                request.operation,
                updated,
                "Only the final filename extension is changed",
            ))
        }
    }
}

fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.last().is_some_and(|last| *last != "..") {
                    components.pop();
                } else if !absolute {
                    components.push("..");
                }
            }
            normal => components.push(normal),
        }
    }
    let joined = components.join("/");
    if absolute {
        if joined.is_empty() {
            "/".into()
        } else {
            format!("/{joined}")
        }
    } else if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

fn inspect(path: &str) -> PathInspection {
    let normalized = normalize(path);
    let absolute = normalized.starts_with('/');
    let components = path_components(&normalized);
    let component_count = components.len();
    let name = if normalized == "/" {
        "/".to_owned()
    } else if normalized == "." {
        ".".to_owned()
    } else {
        components.last().copied().unwrap_or(".").to_owned()
    };
    let parent = if normalized == "/" {
        "/".to_owned()
    } else if components.len() <= 1 {
        if absolute { "/" } else { "." }.to_owned()
    } else {
        let joined = components[..components.len() - 1].join("/");
        if absolute {
            format!("/{joined}")
        } else {
            joined
        }
    };
    let (stem, extension) = split_extension(&name);
    PathInspection {
        normalized,
        absolute,
        parent,
        name,
        stem,
        extension,
        component_count,
    }
}

fn path_components(path: &str) -> Vec<&str> {
    match path {
        "/" | "." => Vec::new(),
        _ => path.trim_start_matches('/').split('/').collect(),
    }
}

fn split_extension(name: &str) -> (String, Option<String>) {
    if matches!(name, "/" | "." | "..") {
        return (name.to_owned(), None);
    }
    let Some(index) = name.rfind('.') else {
        return (name.to_owned(), None);
    };
    if index == 0 || index + 1 == name.len() {
        return (name.to_owned(), None);
    }
    (name[..index].to_owned(), Some(name[index + 1..].to_owned()))
}

fn relative_path(from: &str, target: &str) -> Result<String, String> {
    let from = normalize(from);
    let target = normalize(target);
    if from.starts_with('/') != target.starts_with('/') {
        return Err("source and target must both be absolute or both be relative".into());
    }
    let from_components = path_components(&from);
    let target_components = path_components(&target);
    let shared = from_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    let mut output = vec![".."; from_components.len() - shared];
    output.extend_from_slice(&target_components[shared..]);
    if output.is_empty() {
        Ok(".".into())
    } else {
        Ok(output.join("/"))
    }
}

fn replace_extension(path: &str, extension: &str) -> Result<String, String> {
    let inspection = inspect(path);
    if matches!(inspection.normalized.as_str(), "/" | "." | "..") {
        return Err("path must end in a filename to change its extension".into());
    }
    let remove = matches!(
        extension.to_ascii_lowercase().as_str(),
        "none" | "remove" | "-"
    );
    let base_name = if inspection.extension.is_some() {
        inspection.stem.as_str()
    } else if inspection.name.ends_with('.') {
        &inspection.name[..inspection.name.len() - 1]
    } else {
        inspection.name.as_str()
    };
    let name = if remove {
        base_name.to_owned()
    } else {
        let extension = extension.strip_prefix('.').unwrap_or(extension);
        format!("{base_name}.{extension}")
    };
    Ok(join_parent(&inspection.parent, &name))
}

fn join_parent(parent: &str, name: &str) -> String {
    match parent {
        "/" => format!("/{name}"),
        "." => name.to_owned(),
        _ => format!("{parent}/{name}"),
    }
}

fn evaluate_info(path: &str) -> Result<Evaluation, String> {
    let inspection = inspect(path);
    let kind = if inspection.absolute {
        "absolute"
    } else {
        "relative"
    };
    let extension = inspection.extension.as_deref().unwrap_or("None");
    let report = format!(
        "POSIX path information\nNormalized: {}\nType: {kind}\nParent: {}\nName: {}\nStem: {}\nExtension: {}\nComponents: {}\n\nLexical only: filesystem, symlinks, and '~' expansion are not consulted.",
        visible_text(&inspection.normalized),
        visible_text(&inspection.parent),
        visible_text(&inspection.name),
        visible_text(&inspection.stem),
        visible_text(extension),
        inspection.component_count
    );
    let mut action_values = BTreeMap::from([
        ("paste_result".into(), inspection.normalized.clone()),
        ("copy_result".into(), inspection.normalized.clone()),
        ("copy_parent".into(), inspection.parent.clone()),
        ("copy_name".into(), inspection.name.clone()),
        ("copy_stem".into(), inspection.stem.clone()),
        ("copy_report".into(), report.clone()),
    ]);
    let mut actions = vec![
        action("paste_result", "Paste Normalized Path", true, true, true),
        action("copy_result", "Copy Normalized Path", false, false, false),
        action("copy_parent", "Copy Parent", false, false, false),
        action("copy_name", "Copy Name", false, false, false),
        action("copy_stem", "Copy Stem", false, false, false),
    ];
    if let Some(extension) = &inspection.extension {
        action_values.insert("copy_extension".into(), extension.clone());
        actions.push(action(
            "copy_extension",
            "Copy Extension",
            false,
            false,
            false,
        ));
    }
    actions.push(action(
        "copy_report",
        "Copy Path Information",
        false,
        false,
        false,
    ));
    Ok(Evaluation {
        title: inspection.normalized,
        subtitle: format!(
            "POSIX Path Information · {kind} · {} components",
            inspection.component_count
        ),
        preview: report,
        action_values,
        actions,
    })
}

fn evaluate_output(operation: Operation, output: String, detail: &str) -> Evaluation {
    let preview = format!(
        "{}\n{}\n\nResult\n{}\n\nLexical only: filesystem, symlinks, and '~' expansion are not consulted.",
        operation.label(),
        detail,
        visible_text(&output)
    );
    Evaluation {
        title: output.clone(),
        subtitle: format!("{} · local lexical transformation", operation.label()),
        preview,
        action_values: BTreeMap::from([
            ("paste_result".into(), output.clone()),
            ("copy_result".into(), output),
        ]),
        actions: vec![
            action("paste_result", "Paste Result", true, true, true),
            action("copy_result", "Copy Result", false, false, false),
        ],
    }
}

fn path_result(result_id: String, evaluation: Evaluation) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&safe_text(&evaluation.title), 200),
        subtitle: evaluation.subtitle,
        preview: compact(&evaluation.preview, MAX_VISIBLE_CHARACTERS),
        kind: ResultKind::Command,
        score: 3_325.0,
        metadata: BTreeMap::from([
            (
                "symbol_name".into(),
                "point.bottomleft.forward.to.point.topright.scurvepath".into(),
            ),
            ("preview_style".into(), "text".into()),
        ]),
        actions: evaluation.actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "path-tools:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Path transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Path transformation unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_325.0,
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
    fn parses_commands_arguments_chinese_aliases_and_nonmatches() {
        assert!(parse_request("path to file").unwrap().is_none());
        let join = parse_request("pathjoin /Users/alice :: docs/report.md")
            .unwrap()
            .unwrap();
        assert_eq!(join.operation, Operation::Join);
        assert_eq!(join.argument, "/Users/alice");
        assert_eq!(join.path, "docs/report.md");
        assert_eq!(
            parse_request("相对路径: /a/b :: /a/c/file")
                .unwrap()
                .unwrap()
                .operation,
            Operation::Relative
        );
    }

    #[test]
    fn normalizes_absolute_and_relative_paths_without_filesystem_access() {
        assert_eq!(normalize("/a//b/./c/../file"), "/a/b/file");
        assert_eq!(normalize("../../a/../b"), "../../b");
        assert_eq!(normalize("a/../../b"), "../b");
        assert_eq!(normalize("/../../a"), "/a");
        assert_eq!(normalize("./"), ".");
        assert_eq!(normalize("///"), "/");
    }

    #[test]
    fn inspects_root_hidden_and_multi_extension_names() {
        let root = inspect("/");
        assert_eq!(root.parent, "/");
        assert_eq!(root.name, "/");
        assert_eq!(root.component_count, 0);

        let hidden = inspect("config/.env");
        assert_eq!(hidden.stem, ".env");
        assert_eq!(hidden.extension, None);

        let archive = inspect("/tmp/archive.tar.gz");
        assert_eq!(archive.parent, "/tmp");
        assert_eq!(archive.name, "archive.tar.gz");
        assert_eq!(archive.stem, "archive.tar");
        assert_eq!(archive.extension.as_deref(), Some("gz"));
    }

    #[test]
    fn joins_only_relative_children_and_normalizes_the_result() {
        let request = parse_request("pathjoin /Users/alice/work :: ../docs/readme.md")
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluate(&request).unwrap().title,
            "/Users/alice/docs/readme.md"
        );
        let absolute_child = parse_request("pathjoin /base :: /replacement")
            .unwrap()
            .unwrap();
        assert!(evaluate(&absolute_child).is_err());
    }

    #[test]
    fn computes_relative_paths_from_directories() {
        assert_eq!(
            relative_path("/a/b/c", "/a/d/file.txt").unwrap(),
            "../../d/file.txt"
        );
        assert_eq!(relative_path("/a/b", "/a/b").unwrap(), ".");
        assert_eq!(relative_path("../a", "../b/file").unwrap(), "../b/file");
        assert!(relative_path("/absolute", "relative").is_err());
    }

    #[test]
    fn replaces_removes_and_adds_extensions() {
        assert_eq!(
            replace_extension("dir/archive.tar.gz", "md").unwrap(),
            "dir/archive.tar.md"
        );
        assert_eq!(
            replace_extension("dir/archive.tar.gz", "none").unwrap(),
            "dir/archive.tar"
        );
        assert_eq!(
            replace_extension(".env", ".example").unwrap(),
            ".env.example"
        );
        assert_eq!(replace_extension("file", "tar.gz").unwrap(), "file.tar.gz");
        assert_eq!(replace_extension("file.", "md").unwrap(), "file.md");
        assert_eq!(replace_extension("file.", "none").unwrap(), "file");
        assert!(replace_extension("/", "txt").is_err());
    }

    #[test]
    fn rejects_bad_shapes_paths_and_extension_values() {
        assert!(parse_request("pathnorm path").is_err());
        assert!(parse_request("pathinfo option :: path").is_err());
        assert!(parse_request("pathjoin :: child").is_err());
        assert!(parse_request("pathext /bad :: file").is_err());
        assert!(parse_request("pathext . :: file").is_err());
        assert!(parse_request("pathnorm ::").is_err());
        assert!(parse_request("pathnorm :: bad\npath").is_err());
        assert!(parse_request("pathnorm :: bad\u{202e}path").is_err());
    }

    #[test]
    fn enforces_path_argument_and_query_limits() {
        assert!(parse_request(&format!("pathnorm :: {}", "x".repeat(MAX_PATH_BYTES + 1))).is_err());
        assert!(
            parse_request(&format!(
                "pathjoin {} :: child",
                "x".repeat(MAX_PATH_BYTES + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn info_results_expose_component_actions() {
        let evaluation = evaluate_info("/tmp/archive.tar.gz").unwrap();
        assert_eq!(evaluation.action_values["copy_parent"], "/tmp");
        assert_eq!(evaluation.action_values["copy_name"], "archive.tar.gz");
        assert_eq!(evaluation.action_values["copy_stem"], "archive.tar");
        assert_eq!(evaluation.action_values["copy_extension"], "gz");
        assert!(evaluation.preview.contains("filesystem, symlinks"));
        assert_eq!(evaluation.actions.len(), 7);
    }

    #[test]
    fn searches_are_actionable_and_clear_stale_values() {
        let mut plugin = PathToolsPlugin::default();
        let result = plugin
            .search("pathrelative /a/b :: /a/c/file", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, "path-tools:relative");
        assert_eq!(plugin.action_values["paste_result"], "../c/file");
        assert!(result.actions[0].default);

        let invalid = plugin
            .search("pathjoin /base :: /child", 1)
            .unwrap()
            .remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(plugin.active_result_id.is_none());
        assert!(plugin.search("pathnorm :: value", 0).unwrap().is_empty());
    }

    #[test]
    fn invalid_visible_fields_escape_directional_characters() {
        let mut plugin = PathToolsPlugin::default();
        let result = plugin
            .search("pathnorm :: safe\u{202e}hidden", 1)
            .unwrap()
            .remove(0);
        assert!(result.title.contains("\\u{202E}"));
        assert!(!result.title.contains('\u{202e}'));
        assert!(!result.preview.contains('\u{202e}'));
    }
}
