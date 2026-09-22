use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.permission-calculator";
const MAX_ARGUMENT_BYTES: usize = 64;

fn main() -> io::Result<()> {
    run_plugin(PermissionCalculatorPlugin::default())
}

#[derive(Default)]
struct PermissionCalculatorPlugin {
    active_values: BTreeMap<String, String>,
}

impl Plugin for PermissionCalculatorPlugin {
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
        let parsed = match parse_mode(&request.value) {
            Ok(parsed) => parsed,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let octal = format_octal(parsed.mode);
        let symbolic = format_symbolic(parsed.mode);
        let preview = build_report(parsed.mode, &octal, &symbolic);
        let mut outputs = match parsed.source {
            InputKind::Octal => vec![
                Output::new("symbolic", "Symbolic permissions", symbolic.clone()),
                Output::new("octal", "Octal mode", octal.clone()),
            ],
            InputKind::Symbolic => vec![
                Output::new("octal", "Octal mode", octal.clone()),
                Output::new("symbolic", "Symbolic permissions", symbolic.clone()),
            ],
        };
        outputs.truncate(limit);
        Ok(outputs
            .into_iter()
            .map(|output| {
                let result_id = format!("permission-calculator:{}", output.id);
                self.active_values
                    .insert(result_id.clone(), output.value.clone());
                conversion_result(
                    result_id,
                    output,
                    &request.value,
                    &octal,
                    &symbolic,
                    &preview,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Unix Permissions action: {action_id}"));
        }
        let value = self
            .active_values
            .get(result_id)
            .ok_or_else(|| "permission conversion is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Unix permission copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputKind {
    Octal,
    Symbolic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedMode {
    mode: u16,
    source: InputKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Output {
    id: &'static str,
    label: &'static str,
    value: String,
}

impl Output {
    fn new(id: &'static str, label: &'static str, value: String) -> Self {
        Self { id, label, value }
    }
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "chmod" | "perm") && command != "权限" {
        return Ok(None);
    }
    let arguments = query[command_end..]
        .strip_prefix(':')
        .unwrap_or(&query[command_end..])
        .trim();
    if arguments.is_empty() {
        return Err("expected an octal or symbolic mode, such as 755 or rwxr-xr-x".into());
    }
    if arguments.len() > MAX_ARGUMENT_BYTES {
        return Err("permission input exceeds 64 bytes".into());
    }
    if arguments.split_whitespace().count() != 1 {
        return Err("permission input must be one octal or symbolic mode".into());
    }
    Ok(Some(Request {
        value: arguments.to_owned(),
    }))
}

fn parse_mode(value: &str) -> Result<ParsedMode, String> {
    if value.chars().all(|character| character.is_ascii_digit()) {
        return parse_octal(value);
    }
    parse_symbolic(value)
}

fn parse_octal(value: &str) -> Result<ParsedMode, String> {
    if !matches!(value.len(), 3 | 4) || !value.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return Err("octal mode must contain exactly 3 or 4 digits from 0 through 7".into());
    }
    let mode = u16::from_str_radix(value, 8)
        .map_err(|_| "octal permission mode is out of range".to_owned())?;
    Ok(ParsedMode {
        mode,
        source: InputKind::Octal,
    })
}

fn parse_symbolic(value: &str) -> Result<ParsedMode, String> {
    if !value.is_ascii() {
        return Err("symbolic mode must use the ASCII characters rwxstST-".into());
    }
    let permissions = match value.len() {
        9 => value,
        10 if matches!(
            value.as_bytes()[0],
            b'-' | b'd' | b'l' | b'c' | b'b' | b'p' | b's' | b'?'
        ) =>
        {
            &value[1..]
        }
        _ => {
            return Err(
                "symbolic mode must have 9 permission characters, optionally after a file type"
                    .into(),
            );
        }
    };
    let bytes = permissions.as_bytes();
    let mut mode = 0_u16;
    parse_flag(bytes[0], b'r', 0o400, &mut mode, "owner read")?;
    parse_flag(bytes[1], b'w', 0o200, &mut mode, "owner write")?;
    parse_execute(
        bytes[2],
        b's',
        b'S',
        0o100,
        0o4000,
        &mut mode,
        "owner execute",
    )?;
    parse_flag(bytes[3], b'r', 0o040, &mut mode, "group read")?;
    parse_flag(bytes[4], b'w', 0o020, &mut mode, "group write")?;
    parse_execute(
        bytes[5],
        b's',
        b'S',
        0o010,
        0o2000,
        &mut mode,
        "group execute",
    )?;
    parse_flag(bytes[6], b'r', 0o004, &mut mode, "others read")?;
    parse_flag(bytes[7], b'w', 0o002, &mut mode, "others write")?;
    parse_execute(
        bytes[8],
        b't',
        b'T',
        0o001,
        0o1000,
        &mut mode,
        "others execute",
    )?;
    Ok(ParsedMode {
        mode,
        source: InputKind::Symbolic,
    })
}

fn parse_flag(
    value: u8,
    enabled: u8,
    bit: u16,
    mode: &mut u16,
    position: &str,
) -> Result<(), String> {
    match value {
        b'-' => Ok(()),
        value if value == enabled => {
            *mode |= bit;
            Ok(())
        }
        _ => Err(format!("invalid character at {position} position")),
    }
}

fn parse_execute(
    value: u8,
    special_lower: u8,
    special_upper: u8,
    execute_bit: u16,
    special_bit: u16,
    mode: &mut u16,
    position: &str,
) -> Result<(), String> {
    match value {
        b'-' => Ok(()),
        b'x' => {
            *mode |= execute_bit;
            Ok(())
        }
        value if value == special_lower => {
            *mode |= execute_bit | special_bit;
            Ok(())
        }
        value if value == special_upper => {
            *mode |= special_bit;
            Ok(())
        }
        _ => Err(format!("invalid character at {position} position")),
    }
}

fn format_octal(mode: u16) -> String {
    if mode & 0o7000 == 0 {
        format!("{:03o}", mode)
    } else {
        format!("{:04o}", mode)
    }
}

fn format_symbolic(mode: u16) -> String {
    let mut output = [b'-'; 9];
    for (index, bit, character) in [
        (0, 0o400, b'r'),
        (1, 0o200, b'w'),
        (3, 0o040, b'r'),
        (4, 0o020, b'w'),
        (6, 0o004, b'r'),
        (7, 0o002, b'w'),
    ] {
        if mode & bit != 0 {
            output[index] = character;
        }
    }
    output[2] = execute_character(mode, 0o100, 0o4000, b's', b'S');
    output[5] = execute_character(mode, 0o010, 0o2000, b's', b'S');
    output[8] = execute_character(mode, 0o001, 0o1000, b't', b'T');
    String::from_utf8(output.to_vec()).expect("permission symbols are ASCII")
}

fn execute_character(mode: u16, execute_bit: u16, special_bit: u16, lower: u8, upper: u8) -> u8 {
    match (mode & execute_bit != 0, mode & special_bit != 0) {
        (true, true) => lower,
        (false, true) => upper,
        (true, false) => b'x',
        (false, false) => b'-',
    }
}

fn build_report(mode: u16, octal: &str, symbolic: &str) -> String {
    let special = special_permissions(mode);
    format!(
        "Unix permissions\nOctal: {octal}\nSymbolic: {symbolic}\n\n{}\n{}\n{}\n\n{special}",
        access_line("Owner", mode, 0o400, 0o200, 0o100),
        access_line("Group", mode, 0o040, 0o020, 0o010),
        access_line("Others", mode, 0o004, 0o002, 0o001),
    )
}

fn access_line(label: &str, mode: u16, read: u16, write: u16, execute: u16) -> String {
    let mut permissions = Vec::new();
    if mode & read != 0 {
        permissions.push("read");
    }
    if mode & write != 0 {
        permissions.push("write");
    }
    if mode & execute != 0 {
        permissions.push("execute");
    }
    if permissions.is_empty() {
        permissions.push("none");
    }
    format!("{label}: {}", permissions.join(", "))
}

fn special_permissions(mode: u16) -> String {
    let mut special = Vec::new();
    if mode & 0o4000 != 0 {
        special.push("setuid: executables may use the file owner's effective user ID");
    }
    if mode & 0o2000 != 0 {
        special.push("setgid: executables may use the file group; directories inherit its group");
    }
    if mode & 0o1000 != 0 {
        special.push("sticky: directory entries are normally removable only by their owners");
    }
    if special.is_empty() {
        "Special: none".into()
    } else {
        format!("Special:\n{}", special.join("\n"))
    }
}

fn conversion_result(
    result_id: String,
    output: Output,
    source: &str,
    octal: &str,
    symbolic: &str,
    preview: &str,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: output.value,
        subtitle: format!("{} · {source} → {octal} / {symbolic}", output.label),
        preview: preview.into(),
        kind: ResultKind::Command,
        score: 3_395.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "lock.shield".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Value", true, true),
            action("copy", "Copy Value", false, false),
        ],
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "permission-calculator:invalid".into(),
        title: "Unix Permissions".into(),
        subtitle: format!("Invalid permission mode · {message}"),
        preview: format!("Invalid Unix permission mode\n{message}"),
        kind: ResultKind::Command,
        score: 3_395.0,
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
    fn parses_explicit_commands_without_matching_other_queries() {
        assert_eq!(parse_request("chmod 755").unwrap().unwrap().value, "755");
        assert_eq!(
            parse_request("perm:rwxr-xr-x").unwrap().unwrap().value,
            "rwxr-xr-x"
        );
        assert_eq!(parse_request("权限 644").unwrap().unwrap().value, "644");
        assert_eq!(parse_request("calculator 755").unwrap(), None);
        assert!(parse_request("chmod").is_err());
    }

    #[test]
    fn parses_and_normalizes_octal_modes() {
        assert_eq!(parse_mode("755").unwrap().mode, 0o755);
        assert_eq!(parse_mode("0755").unwrap().mode, 0o755);
        assert_eq!(parse_mode("4755").unwrap().mode, 0o4755);
        assert_eq!(format_octal(0o755), "755");
        assert_eq!(format_octal(0o4755), "4755");
        assert_eq!(format_octal(0), "000");
        assert!(parse_mode("77").is_err());
        assert!(parse_mode("888").is_err());
    }

    #[test]
    fn parses_symbolic_modes_with_optional_file_type() {
        assert_eq!(parse_mode("rwxr-xr-x").unwrap().mode, 0o755);
        assert_eq!(parse_mode("-rw-r--r--").unwrap().mode, 0o644);
        assert_eq!(parse_mode("drwxr-x---").unwrap().mode, 0o750);
        assert!(parse_mode("arwxr-xr-x").is_err());
        assert!(parse_mode("rwxr-xr-z").is_err());
    }

    #[test]
    fn special_bits_preserve_execute_state_and_case() {
        assert_eq!(parse_mode("rwsr-sr-t").unwrap().mode, 0o7755);
        assert_eq!(parse_mode("rwSr-Sr-T").unwrap().mode, 0o7644);
        assert_eq!(format_symbolic(0o7755), "rwsr-sr-t");
        assert_eq!(format_symbolic(0o7644), "rwSr-Sr-T");
    }

    #[test]
    fn every_valid_mode_round_trips_through_symbolic_form() {
        for mode in 0_u16..=0o7777 {
            let symbolic = format_symbolic(mode);
            assert_eq!(parse_mode(&symbolic).unwrap().mode, mode, "{mode:o}");
        }
    }

    #[test]
    fn report_explains_access_and_special_bits() {
        let report = build_report(0o4750, "4750", "rwsr-x---");
        assert!(report.contains("Owner: read, write, execute"));
        assert!(report.contains("Group: read, execute"));
        assert!(report.contains("Others: none"));
        assert!(report.contains("Special:\nsetuid:"));
    }

    #[test]
    fn converted_representation_is_ranked_first() {
        let mut plugin = PermissionCalculatorPlugin::default();
        let octal = plugin.search("chmod 755", 10).unwrap();
        assert_eq!(octal[0].id, "permission-calculator:symbolic");
        assert_eq!(octal[0].title, "rwxr-xr-x");
        let symbolic = plugin.search("perm rw-r-----", 10).unwrap();
        assert_eq!(symbolic[0].id, "permission-calculator:octal");
        assert_eq!(symbolic[0].title, "640");
    }

    #[test]
    fn results_are_bounded_actionable_and_invalidate_old_values() {
        let mut plugin = PermissionCalculatorPlugin::default();
        let result = plugin.search("chmod 755", 1).unwrap().remove(0);
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(plugin.active_values.len(), 1);
        plugin.search("ordinary text", 10).unwrap();
        assert!(plugin.active_values.is_empty());
        assert!(
            plugin.search("chmod 888", 10).unwrap()[0]
                .actions
                .is_empty()
        );
    }

    #[test]
    fn rejects_ambiguous_or_oversized_inputs() {
        assert!(parse_request("chmod 755 extra").is_err());
        let oversized = format!("chmod {}", "7".repeat(65));
        assert!(parse_request(&oversized).is_err());
        assert!(parse_mode("u=rwx,g=rx,o=rx").is_err());
    }
}
