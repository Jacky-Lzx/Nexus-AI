use chrono::{DateTime, Local};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::SystemTime,
};
use url::Url;

#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};

const PLUGIN_ID: &str = "io.clipkeep.file-info";
const RESULT_ID: &str = "file-info:result";
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_VISIBLE_PATH_CHARACTERS: usize = 1_024;

fn main() -> io::Result<()> {
    run_plugin(FileInfoPlugin::default())
}

#[derive(Default)]
struct FileInfoPlugin {
    active_path: Option<PathBuf>,
    action_values: BTreeMap<String, String>,
}

impl Plugin for FileInfoPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_path = None;
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let path = match parse_request(query) {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let inspection = match inspect_path(&path) {
            Ok(inspection) => inspection,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.active_path = Some(inspection.path.clone());
        self.action_values
            .insert("paste_path".into(), inspection.path_text.clone());
        self.action_values
            .insert("copy_path".into(), inspection.path_text.clone());
        self.action_values
            .insert("copy_report".into(), inspection.report.clone());
        if let Some(mode) = &inspection.mode {
            self.action_values
                .insert("copy_mode".into(), mode.octal.clone());
        }
        Ok(vec![inspection_result(&inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("file information is no longer active".into());
        }
        if matches!(action_id, "open" | "reveal") {
            let path = self
                .active_path
                .as_ref()
                .ok_or_else(|| "file information is no longer active".to_owned())?;
            fs::symlink_metadata(path)
                .map_err(|error| format!("file is no longer available: {error}"))?;
            let mut command = Command::new("/usr/bin/open");
            let message = if action_id == "reveal" {
                command.arg("-R").arg(path);
                "Revealed in Finder"
            } else {
                command.arg(path);
                "Opened"
            };
            let status = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("could not launch Finder: {error}"))?;
            if !status.success() {
                return Err(format!("Finder returned {status}"));
            }
            return Ok(Some(message.into()));
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive File Info action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_path" | "copy_path" => "File path copied",
                "copy_report" => "File information copied",
                "copy_mode" => "Unix mode copied",
                _ => "File value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UnixMode {
    symbolic: String,
    octal: String,
    executable: bool,
    uid: u32,
    gid: u32,
    inode: u64,
    device: u64,
    hard_links: u64,
    allocated_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileInspection {
    path: PathBuf,
    path_text: String,
    resolved_path: Option<String>,
    name: String,
    kind: String,
    size: u64,
    readonly: bool,
    hidden: bool,
    mode: Option<UnixMode>,
    created: Option<String>,
    modified: Option<String>,
    accessed: Option<String>,
    symlink_target: Option<String>,
    target_status: Option<String>,
    report: String,
}

fn parse_request(query: &str) -> Result<Option<PathBuf>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "fileinfo" | "finfo")
        && command != "文件信息"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let value = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if value.is_empty() {
        return Err("expected a local path after 'fileinfo', 'finfo', or '文件信息'".into());
    }
    parse_path(value).map(Some)
}

fn parse_path(value: &str) -> Result<PathBuf, String> {
    if value.len() > MAX_PATH_BYTES {
        return Err("file path exceeds 16 KiB".into());
    }
    let value = strip_paired_quotes(value.trim());
    if value.is_empty() {
        return Err("file path cannot be empty".into());
    }
    let path = if value.starts_with("file://") {
        let url = Url::parse(value).map_err(|error| format!("invalid file URL: {error}"))?;
        if url.host_str().is_some_and(|host| host != "localhost") {
            return Err("remote file URL hosts are not supported".into());
        }
        url.to_file_path()
            .map_err(|_| "file URL could not be converted to a local path".to_owned())?
    } else if value == "~" || value.starts_with("~/") {
        let home = env::var_os("HOME").ok_or_else(|| "home directory is unavailable".to_owned())?;
        let mut path = PathBuf::from(home);
        if value.len() > 2 {
            path.push(&value[2..]);
        }
        path
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("file path must be absolute, start with ~/, or use file://".into());
    }
    Ok(path)
}

fn strip_paired_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if matches!(
            (bytes[0], bytes[value.len() - 1]),
            (b'\'', b'\'') | (b'"', b'"')
        ) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn inspect_path(path: &Path) -> Result<FileInspection, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", safe_path(path)))?;
    let file_type = metadata.file_type();
    let kind = file_kind(&file_type).to_owned();
    let path_text = path.to_string_lossy().into_owned();
    let resolved_path = fs::canonicalize(path)
        .ok()
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .filter(|resolved| resolved != &path_text);
    let name = path
        .file_name()
        .map(|name| safe_text(&name.to_string_lossy()))
        .unwrap_or_else(|| safe_path(path));
    let hidden = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.') && name != "." && name != "..");
    let symlink_target = if file_type.is_symlink() {
        Some(
            fs::read_link(path)
                .map(|target| safe_path(&target))
                .unwrap_or_else(|error| format!("unreadable: {error}")),
        )
    } else {
        None
    };
    let target_status = if file_type.is_symlink() {
        Some(match fs::metadata(path) {
            Ok(target) => {
                let size = if target.is_file() {
                    format!(", {}", format_bytes(target.len()))
                } else {
                    String::new()
                };
                format!("{}{size}", file_kind(&target.file_type()))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => "missing target".into(),
            Err(error) => format!("unavailable: {error}"),
        })
    } else {
        None
    };
    let mode = unix_mode(&metadata, &file_type);
    let created = metadata.created().ok().map(format_time);
    let modified = metadata.modified().ok().map(format_time);
    let accessed = metadata.accessed().ok().map(format_time);
    let readonly = metadata.permissions().readonly();
    let mut inspection = FileInspection {
        path: path.to_path_buf(),
        path_text,
        resolved_path,
        name,
        kind,
        size: metadata.len(),
        readonly,
        hidden,
        mode,
        created,
        modified,
        accessed,
        symlink_target,
        target_status,
        report: String::new(),
    };
    inspection.report = build_report(&inspection);
    Ok(inspection)
}

fn file_kind(file_type: &fs::FileType) -> &'static str {
    if file_type.is_file() {
        "Regular file"
    } else if file_type.is_dir() {
        "Directory"
    } else if file_type.is_symlink() {
        "Symbolic link"
    } else {
        unix_special_kind(file_type).unwrap_or("Special file")
    }
}

#[cfg(unix)]
fn unix_special_kind(file_type: &fs::FileType) -> Option<&'static str> {
    if file_type.is_socket() {
        Some("Unix socket")
    } else if file_type.is_fifo() {
        Some("Named pipe")
    } else if file_type.is_block_device() {
        Some("Block device")
    } else if file_type.is_char_device() {
        Some("Character device")
    } else {
        None
    }
}

#[cfg(not(unix))]
fn unix_special_kind(_: &fs::FileType) -> Option<&'static str> {
    None
}

#[cfg(unix)]
fn unix_mode(metadata: &fs::Metadata, file_type: &fs::FileType) -> Option<UnixMode> {
    let mode = metadata.mode();
    Some(UnixMode {
        symbolic: symbolic_mode(file_type, mode),
        octal: format!("{:04o}", mode & 0o7777),
        executable: mode & 0o111 != 0,
        uid: metadata.uid(),
        gid: metadata.gid(),
        inode: metadata.ino(),
        device: metadata.dev(),
        hard_links: metadata.nlink(),
        allocated_bytes: metadata.blocks().saturating_mul(512),
    })
}

#[cfg(not(unix))]
fn unix_mode(_: &fs::Metadata, _: &fs::FileType) -> Option<UnixMode> {
    None
}

#[cfg(unix)]
fn symbolic_mode(file_type: &fs::FileType, mode: u32) -> String {
    let kind = if file_type.is_file() {
        '-'
    } else if file_type.is_dir() {
        'd'
    } else if file_type.is_symlink() {
        'l'
    } else if file_type.is_socket() {
        's'
    } else if file_type.is_fifo() {
        'p'
    } else if file_type.is_block_device() {
        'b'
    } else if file_type.is_char_device() {
        'c'
    } else {
        '?'
    };
    let mut output = String::with_capacity(10);
    output.push(kind);
    output.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    output.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    output.push(execute_character(mode, 0o100, 0o4000, 's', 'S'));
    output.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    output.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    output.push(execute_character(mode, 0o010, 0o2000, 's', 'S'));
    output.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    output.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    output.push(execute_character(mode, 0o001, 0o1000, 't', 'T'));
    output
}

#[cfg(unix)]
fn execute_character(
    mode: u32,
    execute_bit: u32,
    special_bit: u32,
    both: char,
    special_only: char,
) -> char {
    match (mode & execute_bit != 0, mode & special_bit != 0) {
        (true, true) => both,
        (false, true) => special_only,
        (true, false) => 'x',
        (false, false) => '-',
    }
}

fn build_report(inspection: &FileInspection) -> String {
    let mut lines = vec![
        "File information".to_owned(),
        format!("Path: {}", visible(&inspection.path_text)),
    ];
    if let Some(resolved) = &inspection.resolved_path {
        lines.push(format!("Resolved path: {}", visible(resolved)));
    }
    lines.extend([
        format!("Name: {}", inspection.name),
        format!("Type: {}", inspection.kind),
        format!(
            "Size: {} ({} bytes)",
            format_bytes(inspection.size),
            inspection.size
        ),
        format!("Hidden name: {}", yes_no(inspection.hidden)),
        format!("Read-only: {}", yes_no(inspection.readonly)),
    ]);
    if let Some(mode) = &inspection.mode {
        lines.extend([
            format!("Unix mode: {} ({})", mode.symbolic, mode.octal),
            format!("Executable: {}", yes_no(mode.executable)),
            format!("Owner IDs: UID {}, GID {}", mode.uid, mode.gid),
            format!(
                "Allocated: {} ({} bytes)",
                format_bytes(mode.allocated_bytes),
                mode.allocated_bytes
            ),
            format!("Device / inode: {} / {}", mode.device, mode.inode),
            format!("Hard links: {}", mode.hard_links),
        ]);
    }
    if let Some(target) = &inspection.symlink_target {
        lines.push(format!("Link target: {}", visible(target)));
    }
    if let Some(status) = &inspection.target_status {
        lines.push(format!("Target status: {}", safe_text(status)));
    }
    lines.extend([
        format!("Created: {}", optional_value(&inspection.created)),
        format!("Modified: {}", optional_value(&inspection.modified)),
        format!("Accessed: {}", optional_value(&inspection.accessed)),
    ]);
    lines.join("\n")
}

fn inspection_result(inspection: &FileInspection) -> SearchResult {
    let mode = inspection
        .mode
        .as_ref()
        .map(|mode| format!(" · {} ({})", mode.symbolic, mode.octal))
        .unwrap_or_default();
    let size = if inspection.kind == "Directory" {
        String::new()
    } else {
        format!(" · {}", format_bytes(inspection.size))
    };
    let mut actions = vec![
        action("paste_path", "Paste File Path", true, true, true),
        action("open", "Open", false, true, false),
        action("reveal", "Reveal in Finder", false, true, false),
        action("copy_path", "Copy File Path", false, false, false),
        action("copy_report", "Copy File Information", false, false, false),
    ];
    if inspection.mode.is_some() {
        actions.push(action("copy_mode", "Copy Unix Mode", false, false, false));
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!("{} · {}", compact(&inspection.name, 180), inspection.kind),
        subtitle: format!(
            "{}{}{} · modified {}",
            inspection.kind,
            size,
            mode,
            optional_value(&inspection.modified)
        ),
        preview: inspection.report.clone(),
        kind: ResultKind::File,
        score: 3_430.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), symbol_name(&inspection.kind).into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "file-info:invalid".into(),
        title: compact(query.trim(), 200),
        subtitle: format!("File information unavailable · {}", compact(message, 120)),
        preview: format!("File information unavailable\n{message}"),
        kind: ResultKind::File,
        score: 3_430.0,
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

fn symbol_name(kind: &str) -> &'static str {
    match kind {
        "Directory" => "folder.fill",
        "Symbolic link" => "link",
        "Unix socket" => "network",
        "Named pipe" => "arrow.left.arrow.right",
        _ => "doc.fill",
    }
}

fn format_time(value: SystemTime) -> String {
    let local: DateTime<Local> = value.into();
    local.format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn optional_value(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("not available")
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn safe_path(path: &Path) -> String {
    visible(&path.to_string_lossy())
}

fn visible(value: &str) -> String {
    compact(&safe_text(value), MAX_VISIBLE_PATH_CHARACTERS)
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
    use std::time::{Duration, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn test_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "nexus-file-info-{}-{name}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_explicit_commands_quotes_tilde_and_file_urls() {
        assert_eq!(
            parse_request("fileinfo \"/tmp/a b\"").unwrap(),
            Some(PathBuf::from("/tmp/a b"))
        );
        assert_eq!(
            parse_request("文件信息: file:///tmp/a%20b").unwrap(),
            Some(PathBuf::from("/tmp/a b"))
        );
        let home = PathBuf::from(env::var_os("HOME").unwrap());
        assert_eq!(
            parse_request("finfo ~/file").unwrap(),
            Some(home.join("file"))
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn rejects_missing_relative_remote_and_oversized_paths() {
        assert!(parse_request("fileinfo").is_err());
        assert!(parse_request("fileinfo relative/path").is_err());
        assert!(parse_request("fileinfo file://remote.example/tmp/a").is_err());
        assert!(parse_request(&format!("fileinfo /{}", "x".repeat(MAX_PATH_BYTES))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn inspects_regular_files_permissions_sizes_and_actions() {
        let directory = test_directory("regular");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("sample.txt");
        fs::write(&path, b"hello").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let inspection = inspect_path(&path).unwrap();
        assert_eq!(inspection.kind, "Regular file");
        assert_eq!(inspection.size, 5);
        assert_eq!(inspection.mode.as_ref().unwrap().octal, "0640");
        assert_eq!(inspection.mode.as_ref().unwrap().symbolic, "-rw-r-----");
        assert!(inspection.report.contains("Size: 5 B (5 bytes)"));
        let result = inspection_result(&inspection);
        assert_eq!(result.kind, ResultKind::File);
        assert_eq!(result.actions[0].id, "paste_path");
        assert!(result.actions[0].default);
        assert!(result.actions.iter().any(|action| action.id == "reveal"));
        assert!(result.actions.iter().any(|action| action.id == "copy_mode"));

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn inspects_live_and_dangling_symbolic_links_without_following_them() {
        let directory = test_directory("links");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("target"), b"abc").unwrap();
        let live = directory.join("live");
        let dangling = directory.join("dangling");
        symlink("target", &live).unwrap();
        symlink("missing", &dangling).unwrap();

        let live_info = inspect_path(&live).unwrap();
        assert_eq!(live_info.kind, "Symbolic link");
        assert_eq!(live_info.symlink_target.as_deref(), Some("target"));
        assert_eq!(
            live_info.target_status.as_deref(),
            Some("Regular file, 3 B")
        );
        assert!(live_info.resolved_path.is_some());
        let dangling_info = inspect_path(&dangling).unwrap();
        assert_eq!(
            dangling_info.target_status.as_deref(),
            Some("missing target")
        );
        assert!(dangling_info.resolved_path.is_none());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn hidden_directories_and_missing_paths_are_reported_safely() {
        let directory = test_directory("hidden").join(".secret");
        fs::create_dir_all(&directory).unwrap();
        let inspection = inspect_path(&directory).unwrap();
        assert_eq!(inspection.kind, "Directory");
        assert!(inspection.hidden);
        assert!(inspection.report.contains("Hidden name: Yes"));
        fs::remove_dir_all(directory.parent().unwrap()).unwrap();

        let missing = test_directory("missing");
        let error = inspect_path(&missing).unwrap_err();
        assert!(error.contains("cannot inspect"));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_modes_cover_special_execute_bits() {
        let file = fs::metadata("/dev/null").unwrap().file_type();
        assert_eq!(symbolic_mode(&file, 0o6754), "crwsr-sr--");
        assert_eq!(execute_character(0o1000, 0o001, 0o1000, 't', 'T'), 'T');
        assert_eq!(execute_character(0o1001, 0o001, 0o1000, 't', 'T'), 't');
    }

    #[test]
    fn formatting_and_visible_text_are_bounded() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(
            safe_text("safe\nspoof\u{202E}"),
            "safe\\u{000A}spoof\\u{202E}"
        );
        assert_eq!(format_time(UNIX_EPOCH + Duration::from_secs(0)).len(), 26);
        assert_eq!(compact("éclair", 2), "éc…");
    }

    #[test]
    fn nonmatching_and_zero_limit_searches_clear_stale_state() {
        let mut plugin = FileInfoPlugin {
            active_path: Some(PathBuf::from("/tmp/stale")),
            ..Default::default()
        };
        plugin
            .action_values
            .insert("copy_path".into(), "stale".into());
        assert!(plugin.search("ordinary search", 10).unwrap().is_empty());
        assert!(plugin.active_path.is_none());
        assert!(plugin.action_values.is_empty());
        plugin.active_path = Some(PathBuf::from("/tmp/stale"));
        assert!(plugin.search("fileinfo /tmp", 0).unwrap().is_empty());
        assert!(plugin.active_path.is_none());
    }
}
