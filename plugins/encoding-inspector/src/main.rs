use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::BTreeMap,
    env, fs,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
use url::Url;

const PLUGIN_ID: &str = "io.clipkeep.encoding-inspector";
const RESULT_ID: &str = "encoding-inspector:result";
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_VISIBLE_PATH_CHARACTERS: usize = 1_024;
const SAMPLE_HEX_BYTES: usize = 32;

fn main() -> io::Result<()> {
    run_plugin(EncodingInspectorPlugin::default())
}

#[derive(Default)]
struct EncodingInspectorPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for EncodingInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let path = match parse_request(query) {
            Ok(Some(path)) => path,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let inspection = match inspect_file(&path) {
            Ok(inspection) => inspection,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values
            .insert("paste_encoding".into(), inspection.encoding.clone());
        self.action_values
            .insert("copy_encoding".into(), inspection.encoding.clone());
        self.action_values
            .insert("copy_report".into(), inspection.report.clone());
        self.action_values
            .insert("copy_path".into(), inspection.path.clone());
        Ok(vec![inspection_result(&inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("encoding inspection is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive Encoding Inspector action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_encoding" | "copy_encoding" => "Encoding copied",
                "copy_report" => "Encoding report copied",
                "copy_path" => "File path copied",
                _ => "Encoding value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextStats {
    unicode_scalars: usize,
    logical_lines: usize,
    crlf: usize,
    lf: usize,
    cr: usize,
    final_line_ending: bool,
    nul_scalars: usize,
    control_scalars: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContentAnalysis {
    encoding: String,
    bom: &'static str,
    confidence: &'static str,
    content_type: String,
    text: Option<String>,
    binary_like: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EncodingInspection {
    path: String,
    filename: String,
    size: u64,
    encoding: String,
    content_type: String,
    bom: String,
    confidence: String,
    binary_like: bool,
    text_stats: Option<TextStats>,
    nul_bytes: usize,
    control_bytes: usize,
    first_bytes: String,
    report: String,
}

fn parse_request(query: &str) -> Result<Option<PathBuf>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "encoding" | "fileencoding" | "encinfo"
    ) && command != "编码信息"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let value = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if value.is_empty() {
        return Err("expected a local file path after the encoding command".into());
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

fn inspect_file(path: &Path) -> Result<EncodingInspection, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {}: {error}", safe_path(path)))?;
    let mut file = File::open(&canonical)
        .map_err(|error| format!("cannot open {}: {error}", safe_path(&canonical)))?;
    let initial = file
        .metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", safe_path(&canonical)))?;
    if !initial.is_file() {
        return Err("encoding input must be a regular file".into());
    }
    if initial.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file is {}; the encoding inspection limit is {}",
            format_bytes(initial.len()),
            format_bytes(MAX_FILE_BYTES)
        ));
    }
    let initial_modified = initial.modified().ok();
    let mut bytes = Vec::with_capacity(initial.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", safe_path(&canonical)))?;
    let final_metadata = file
        .metadata()
        .map_err(|error| format!("cannot recheck {}: {error}", safe_path(&canonical)))?;
    if initial.len() != final_metadata.len()
        || initial_modified != final_metadata.modified().ok()
        || bytes.len() as u64 != initial.len()
    {
        return Err("file changed while its encoding was being inspected; try again".into());
    }
    let analysis = analyze_content(&bytes)?;
    let text_stats = analysis.text.as_deref().map(analyze_text);
    let nul_bytes = bytes.iter().filter(|byte| **byte == 0).count();
    let control_bytes = bytes
        .iter()
        .filter(|byte| matches!(**byte, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f))
        .count();
    let first_bytes = bytes
        .iter()
        .take(SAMPLE_HEX_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let path_text = canonical.to_string_lossy().into_owned();
    let filename = canonical
        .file_name()
        .map(|name| safe_text(&name.to_string_lossy()))
        .unwrap_or_else(|| safe_path(&canonical));
    let mut inspection = EncodingInspection {
        path: path_text,
        filename,
        size: initial.len(),
        encoding: analysis.encoding,
        content_type: analysis.content_type,
        bom: analysis.bom.into(),
        confidence: analysis.confidence.into(),
        binary_like: analysis.binary_like,
        text_stats,
        nul_bytes,
        control_bytes,
        first_bytes,
        report: String::new(),
    };
    inspection.report = build_report(&inspection);
    Ok(inspection)
}

fn analyze_content(bytes: &[u8]) -> Result<ContentAnalysis, String> {
    if bytes.is_empty() {
        return Ok(ContentAnalysis {
            encoding: "Empty file".into(),
            bom: "None",
            confidence: "Exact",
            content_type: "Empty".into(),
            text: Some(String::new()),
            binary_like: false,
        });
    }
    if bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) {
        return decode_utf32(&bytes[4..], false, "UTF-32 BE", "UTF-32 BE BOM", "Exact");
    }
    if bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00]) {
        return decode_utf32(&bytes[4..], true, "UTF-32 LE", "UTF-32 LE BOM", "Exact");
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = std::str::from_utf8(&bytes[3..])
            .map_err(|error| format!("UTF-8 BOM is present but text is invalid: {error}"))?;
        return Ok(text_analysis(
            "UTF-8",
            "UTF-8 BOM",
            "Exact",
            text.to_owned(),
        ));
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&bytes[2..], true, "UTF-16 LE", "UTF-16 LE BOM", "Exact");
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&bytes[2..], false, "UTF-16 BE", "UTF-16 BE BOM", "Exact");
    }
    if let Some(format) = binary_signature(bytes) {
        return Ok(binary_analysis(format));
    }
    if let Some(analysis) = probable_utf32(bytes)? {
        return Ok(analysis);
    }
    if let Some(analysis) = probable_utf16(bytes)? {
        return Ok(analysis);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let encoding = if text.is_ascii() { "ASCII" } else { "UTF-8" };
        return Ok(text_analysis(encoding, "None", "Exact", text.to_owned()));
    }
    Ok(binary_analysis(
        binary_signature(bytes).unwrap_or("Unknown binary data"),
    ))
}

fn text_analysis(
    encoding: &str,
    bom: &'static str,
    confidence: &'static str,
    text: String,
) -> ContentAnalysis {
    let stats = analyze_text(&text);
    let scalar_count = stats.unicode_scalars.max(1);
    let binary_like =
        stats.nul_scalars > 0 || stats.control_scalars.saturating_mul(100) > scalar_count;
    ContentAnalysis {
        encoding: encoding.into(),
        bom,
        confidence,
        content_type: "Text".into(),
        text: Some(text),
        binary_like,
    }
}

fn binary_analysis(format: &str) -> ContentAnalysis {
    ContentAnalysis {
        encoding: "Binary".into(),
        bom: "None",
        confidence: "Signature/byte analysis",
        content_type: format.into(),
        text: None,
        binary_like: true,
    }
}

fn decode_utf16(
    bytes: &[u8],
    little_endian: bool,
    encoding: &str,
    bom: &'static str,
    confidence: &'static str,
) -> Result<ContentAnalysis, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(format!("{encoding} byte length is not divisible by 2"));
    }
    let (chunks, remainder) = bytes.as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    let units = chunks
        .iter()
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    let text = String::from_utf16(&units)
        .map_err(|error| format!("{encoding} contains an invalid surrogate: {error}"))?;
    Ok(text_analysis(encoding, bom, confidence, text))
}

fn decode_utf32(
    bytes: &[u8],
    little_endian: bool,
    encoding: &str,
    bom: &'static str,
    confidence: &'static str,
) -> Result<ContentAnalysis, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("{encoding} byte length is not divisible by 4"));
    }
    let mut text = String::new();
    let (chunks, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        let value = if little_endian {
            u32::from_le_bytes(*chunk)
        } else {
            u32::from_be_bytes(*chunk)
        };
        let character = char::from_u32(value)
            .ok_or_else(|| format!("{encoding} contains invalid scalar U+{value:04X}"))?;
        text.push(character);
    }
    Ok(text_analysis(encoding, bom, confidence, text))
}

fn probable_utf32(bytes: &[u8]) -> Result<Option<ContentAnalysis>, String> {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(4) {
        return Ok(None);
    }
    let groups = bytes.len() / 4;
    let (chunks, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    let le_zero_groups = chunks
        .iter()
        .filter(|chunk| chunk[1] == 0 && chunk[2] == 0 && chunk[3] == 0)
        .count();
    let be_zero_groups = chunks
        .iter()
        .filter(|chunk| chunk[0] == 0 && chunk[1] == 0 && chunk[2] == 0)
        .count();
    if le_zero_groups * 100 >= groups * 60 {
        return Ok(decode_utf32(bytes, true, "Probable UTF-32 LE", "None", "Heuristic").ok());
    }
    if be_zero_groups * 100 >= groups * 60 {
        return Ok(decode_utf32(bytes, false, "Probable UTF-32 BE", "None", "Heuristic").ok());
    }
    Ok(None)
}

fn probable_utf16(bytes: &[u8]) -> Result<Option<ContentAnalysis>, String> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return Ok(None);
    }
    let pairs = bytes.len() / 2;
    let odd_zeros = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    let even_zeros = bytes.iter().step_by(2).filter(|byte| **byte == 0).count();
    if odd_zeros * 100 >= pairs * 60 && even_zeros * 100 <= pairs * 10 {
        return Ok(decode_utf16(bytes, true, "Probable UTF-16 LE", "None", "Heuristic").ok());
    }
    if even_zeros * 100 >= pairs * 60 && odd_zeros * 100 <= pairs * 10 {
        return Ok(decode_utf16(bytes, false, "Probable UTF-16 BE", "None", "Heuristic").ok());
    }
    Ok(None)
}

fn binary_signature(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("PNG image")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("JPEG image")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("GIF image")
    } else if bytes.starts_with(b"%PDF-") {
        Some("PDF document")
    } else if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        Some("ZIP archive")
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Some("Gzip stream")
    } else if bytes.starts_with(b"\x7fELF") {
        Some("ELF executable")
    } else if bytes.starts_with(b"\0asm") {
        Some("WebAssembly module")
    } else if bytes.starts_with(b"SQLite format 3\0") {
        Some("SQLite database")
    } else if matches!(
        bytes.get(..4),
        Some([0xfe, 0xed, 0xfa, 0xce])
            | Some([0xfe, 0xed, 0xfa, 0xcf])
            | Some([0xce, 0xfa, 0xed, 0xfe])
            | Some([0xcf, 0xfa, 0xed, 0xfe])
            | Some([0xca, 0xfe, 0xba, 0xbe])
    ) {
        Some("Mach-O binary")
    } else {
        None
    }
}

fn analyze_text(text: &str) -> TextStats {
    let mut crlf = 0;
    let mut lf = 0;
    let mut cr = 0;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' if characters.peek() == Some(&'\n') => {
                characters.next();
                crlf += 1;
            }
            '\r' => cr += 1,
            '\n' => lf += 1,
            _ => {}
        }
    }
    let final_line_ending = text.ends_with('\n') || text.ends_with('\r');
    let separators = crlf + lf + cr;
    let unicode_scalars = text.chars().count();
    TextStats {
        unicode_scalars,
        logical_lines: if text.is_empty() {
            0
        } else {
            separators + usize::from(!final_line_ending)
        },
        crlf,
        lf,
        cr,
        final_line_ending,
        nul_scalars: text.chars().filter(|character| *character == '\0').count(),
        control_scalars: text
            .chars()
            .filter(|character| character.is_control() && !matches!(character, '\t' | '\r' | '\n'))
            .count(),
    }
}

fn build_report(inspection: &EncodingInspection) -> String {
    let mut lines = vec![
        "Encoding inspection".into(),
        format!("Path: {}", visible(&inspection.path)),
        format!(
            "Size: {} ({} bytes)",
            format_bytes(inspection.size),
            inspection.size
        ),
        format!("Content: {}", inspection.content_type),
        format!("Encoding: {}", inspection.encoding),
        format!("Detection: {}", inspection.confidence),
        format!("Byte order mark: {}", inspection.bom),
        format!(
            "Binary-like text controls: {}",
            yes_no(inspection.binary_like)
        ),
        format!("NUL bytes: {}", inspection.nul_bytes),
        format!("Other control bytes: {}", inspection.control_bytes),
        format!(
            "First {} bytes: {}",
            inspection.size.min(SAMPLE_HEX_BYTES as u64),
            if inspection.first_bytes.is_empty() {
                "(none)"
            } else {
                &inspection.first_bytes
            }
        ),
    ];
    if let Some(stats) = &inspection.text_stats {
        lines.extend([
            format!("Unicode scalars: {}", stats.unicode_scalars),
            format!("Logical lines: {}", stats.logical_lines),
            format!("Line endings: {}", line_ending_summary(stats)),
            format!("Final line ending: {}", yes_no(stats.final_line_ending)),
            format!("NUL scalars: {}", stats.nul_scalars),
            format!("Other control scalars: {}", stats.control_scalars),
        ]);
    }
    lines.join("\n")
}

fn line_ending_summary(stats: &TextStats) -> String {
    let mut endings = Vec::new();
    if stats.crlf > 0 {
        endings.push(format!("CRLF {}", stats.crlf));
    }
    if stats.lf > 0 {
        endings.push(format!("LF {}", stats.lf));
    }
    if stats.cr > 0 {
        endings.push(format!("CR {}", stats.cr));
    }
    if endings.is_empty() {
        "none".into()
    } else {
        endings.join(", ")
    }
}

fn inspection_result(inspection: &EncodingInspection) -> SearchResult {
    let detail = inspection
        .text_stats
        .as_ref()
        .map(|stats| {
            format!(
                " · {} line{}",
                stats.logical_lines,
                if stats.logical_lines == 1 { "" } else { "s" }
            )
        })
        .unwrap_or_default();
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!("{} · {}", inspection.encoding, inspection.filename),
        subtitle: format!(
            "{} · {} · BOM {}{}",
            inspection.content_type,
            format_bytes(inspection.size),
            inspection.bom,
            detail
        ),
        preview: inspection.report.clone(),
        kind: ResultKind::File,
        score: 3_430.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "text.magnifyingglass".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste_encoding", "Paste Encoding", true, true, true),
            action("copy_encoding", "Copy Encoding", false, false, false),
            action("copy_report", "Copy Encoding Report", false, false, false),
            action("copy_path", "Copy File Path", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "encoding-inspector:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Encoding inspection unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Encoding inspection unavailable\n{}", safe_text(message)),
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "nexus-encoding-inspector-{}-{name}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_explicit_paths_quotes_tilde_and_file_urls() {
        assert_eq!(
            parse_request("encoding \"/tmp/a b.txt\"").unwrap(),
            Some(PathBuf::from("/tmp/a b.txt"))
        );
        assert_eq!(
            parse_request("编码信息: file:///tmp/a%20b.txt").unwrap(),
            Some(PathBuf::from("/tmp/a b.txt"))
        );
        let home = PathBuf::from(env::var_os("HOME").unwrap());
        assert_eq!(
            parse_request("encinfo ~/file.txt").unwrap(),
            Some(home.join("file.txt"))
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn rejects_missing_relative_remote_and_oversized_paths() {
        assert!(parse_request("encoding").is_err());
        assert!(parse_request("encoding relative.txt").is_err());
        assert!(parse_request("encoding file://remote.example/tmp/a").is_err());
        assert!(parse_request(&format!("encoding /{}", "x".repeat(MAX_PATH_BYTES))).is_err());
    }

    #[test]
    fn detects_empty_ascii_utf8_and_utf8_bom() {
        assert_eq!(analyze_content(b"").unwrap().encoding, "Empty file");
        let ascii = analyze_content(b"hello\nworld\n").unwrap();
        assert_eq!(ascii.encoding, "ASCII");
        assert_eq!(
            analyze_text(ascii.text.as_deref().unwrap()).logical_lines,
            2
        );
        let utf8 = analyze_content("你好\n".as_bytes()).unwrap();
        assert_eq!(utf8.encoding, "UTF-8");
        assert_eq!(utf8.bom, "None");
        let bom = analyze_content(b"\xef\xbb\xbfhello").unwrap();
        assert_eq!(bom.encoding, "UTF-8");
        assert_eq!(bom.bom, "UTF-8 BOM");
    }

    #[test]
    fn decodes_utf16_and_utf32_both_byte_orders() {
        let utf16_le = analyze_content(&[0xff, 0xfe, b'A', 0, b'\n', 0]).unwrap();
        assert_eq!(utf16_le.encoding, "UTF-16 LE");
        assert_eq!(utf16_le.text.as_deref(), Some("A\n"));
        let utf16_be = analyze_content(&[0xfe, 0xff, 0, b'A']).unwrap();
        assert_eq!(utf16_be.encoding, "UTF-16 BE");
        let utf32_le = analyze_content(&[0xff, 0xfe, 0, 0, b'A', 0, 0, 0]).unwrap();
        assert_eq!(utf32_le.encoding, "UTF-32 LE");
        let utf32_be = analyze_content(&[0, 0, 0xfe, 0xff, 0, 0, 0, b'A']).unwrap();
        assert_eq!(utf32_be.encoding, "UTF-32 BE");
    }

    #[test]
    fn detects_probable_utf16_and_utf32_without_boms() {
        let utf16_le = analyze_content(&[b'A', 0, b'B', 0, b'C', 0]).unwrap();
        assert_eq!(utf16_le.encoding, "Probable UTF-16 LE");
        assert_eq!(utf16_le.confidence, "Heuristic");
        assert_eq!(
            analyze_content(&[0, b'A', 0, b'B', 0, b'C'])
                .unwrap()
                .encoding,
            "Probable UTF-16 BE"
        );
        let utf32_le = analyze_content(&[b'A', 0, 0, 0, b'B', 0, 0, 0]).unwrap();
        assert_eq!(utf32_le.encoding, "Probable UTF-32 LE");
        assert_eq!(utf32_le.confidence, "Heuristic");
    }

    #[test]
    fn reports_mixed_line_endings_controls_and_final_newline() {
        let stats = analyze_text("a\r\nb\nc\rd\0");
        assert_eq!(stats.crlf, 1);
        assert_eq!(stats.lf, 1);
        assert_eq!(stats.cr, 1);
        assert_eq!(stats.logical_lines, 4);
        assert!(!stats.final_line_ending);
        assert_eq!(stats.nul_scalars, 1);
        assert_eq!(stats.control_scalars, 1);
        assert_eq!(line_ending_summary(&stats), "CRLF 1, LF 1, CR 1");
    }

    #[test]
    fn identifies_common_binary_signatures_and_unknown_binary() {
        let png = analyze_content(b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert_eq!(png.encoding, "Binary");
        assert_eq!(png.content_type, "PNG image");
        let sqlite = analyze_content(b"SQLite format 3\0more").unwrap();
        assert_eq!(sqlite.content_type, "SQLite database");
        let unknown = analyze_content(&[0xff, 0x00, 0x81]).unwrap();
        assert_eq!(unknown.content_type, "Unknown binary data");
    }

    #[test]
    fn rejects_malformed_declared_unicode_encodings() {
        assert!(analyze_content(&[0xff, 0xfe, b'A']).is_err());
        assert!(analyze_content(&[0x00, 0x00, 0xfe, 0xff, 0, 0, 0]).is_err());
        assert!(analyze_content(&[0xef, 0xbb, 0xbf, 0xff]).is_err());
    }

    #[test]
    fn inspects_real_files_and_rejects_directories_and_large_files() {
        let directory = test_directory("files");
        fs::create_dir(&directory).unwrap();
        let text_path = directory.join("sample.txt");
        fs::write(&text_path, "hello\n世界\n").unwrap();
        let inspection = inspect_file(&text_path).unwrap();
        assert_eq!(inspection.encoding, "UTF-8");
        assert_eq!(inspection.text_stats.as_ref().unwrap().logical_lines, 2);
        assert!(inspection.report.contains("Final line ending: Yes"));
        assert!(inspect_file(&directory).is_err());
        let large = directory.join("large.bin");
        let file = File::create(&large).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(inspect_file(&large).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn results_are_actionable_safe_and_invalidate_stale_values() {
        let directory = test_directory("result");
        fs::create_dir(&directory).unwrap();
        let path = directory.join("sample.txt");
        fs::write(&path, "hello\n").unwrap();
        let inspection = inspect_file(&path).unwrap();
        let result = inspection_result(&inspection);
        assert_eq!(result.kind, ResultKind::File);
        assert_eq!(result.actions.len(), 4);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        fs::remove_dir_all(directory).unwrap();

        let mut plugin = EncodingInspectorPlugin::default();
        plugin
            .action_values
            .insert("copy_report".into(), "stale".into());
        assert!(plugin.search("ordinary search", 10).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        plugin
            .action_values
            .insert("copy_report".into(), "stale".into());
        assert!(plugin.search("encoding /tmp/a", 0).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        let invalid = invalid_result("encoding bad\nname\u{202e}", "bad\npath");
        assert!(!invalid.title.contains('\u{202e}'));
        assert!(invalid.preview.contains("bad\\u{000A}path"));
    }
}
