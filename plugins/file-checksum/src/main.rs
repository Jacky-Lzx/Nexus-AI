use clipkeep_plugin_sdk::{Plugin, SearchContext, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use ring::digest::{Algorithm, Context, SHA1_FOR_LEGACY_USE_ONLY, SHA256, SHA512};
use std::{
    collections::BTreeMap,
    env, fs,
    fs::File,
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
};
use url::Url;

const PLUGIN_ID: &str = "io.clipkeep.file-checksum";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const READ_BUFFER_BYTES: usize = 128 * 1024;
const MAX_TITLE_CHARACTERS: usize = 180;

fn main() -> io::Result<()> {
    run_plugin(FileChecksumPlugin::default())
}

#[derive(Default)]
struct FileChecksumPlugin {
    action_values: BTreeMap<(String, String), String>,
}

impl Plugin for FileChecksumPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.search_with_context(query, limit, &SearchContext::new(0, None))
    }

    fn search_with_context(
        &mut self,
        query: &str,
        limit: usize,
        context: &SearchContext,
    ) -> Result<Vec<SearchResult>, String> {
        context.check()?;
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let evaluation = match evaluate_with_context(&request, context) {
            Ok(evaluation) => evaluation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values = evaluation.action_values;
        let mut results = evaluation.results;
        results.truncate(limit);
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let value = self
            .action_values
            .get(&(result_id.to_owned(), action_id.to_owned()))
            .ok_or_else(|| format!("unknown or inactive File Checksum action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "copy_path" => "File path copied",
                "copy_expected" => "Expected checksum copied",
                _ => "File checksum copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashAlgorithm {
    Sha256,
    Sha512,
    Sha1,
}

impl HashAlgorithm {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().replace('-', "").as_str() {
            "sha256" | "256" => Some(Self::Sha256),
            "sha512" | "512" => Some(Self::Sha512),
            "sha1" | "1" => Some(Self::Sha1),
            _ => None,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
            Self::Sha1 => "sha1",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
            Self::Sha1 => "SHA-1 (legacy)",
        }
    }

    fn digest_length(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha512 => 128,
            Self::Sha1 => 40,
        }
    }

    fn ring_algorithm(self) -> &'static Algorithm {
        match self {
            Self::Sha256 => &SHA256,
            Self::Sha512 => &SHA512,
            Self::Sha1 => &SHA1_FOR_LEGACY_USE_ONLY,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChecksumRequest {
    Calculate {
        algorithms: Vec<HashAlgorithm>,
        path: PathBuf,
    },
    Verify {
        algorithm: HashAlgorithm,
        expected: String,
        path: PathBuf,
    },
}

struct HashValue {
    algorithm: HashAlgorithm,
    value: String,
}

struct HashedFile {
    path: PathBuf,
    size: u64,
    hashes: Vec<HashValue>,
}

struct Evaluation {
    results: Vec<SearchResult>,
    action_values: BTreeMap<(String, String), String>,
}

fn parse_request(query: &str) -> Result<Option<ChecksumRequest>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "checksum" | "filehash" | "hashfile"
    ) && command != "文件哈希"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let arguments = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if arguments.is_empty() {
        return Err("expected a file path after the checksum command".into());
    }
    let (first, rest) = take_token(arguments);
    if first.eq_ignore_ascii_case("verify") || first.eq_ignore_ascii_case("check") {
        return parse_verification(rest).map(Some);
    }
    let (algorithms, path_text) = if let Some(algorithm) = HashAlgorithm::parse(first) {
        (vec![algorithm], rest)
    } else {
        (
            vec![HashAlgorithm::Sha256, HashAlgorithm::Sha512],
            arguments,
        )
    };
    if path_text.is_empty() {
        return Err("expected a file path after the checksum algorithm".into());
    }
    Ok(Some(ChecksumRequest::Calculate {
        algorithms,
        path: parse_path(path_text)?,
    }))
}

fn parse_verification(arguments: &str) -> Result<ChecksumRequest, String> {
    let (algorithm_text, rest) = take_token(arguments);
    let algorithm = HashAlgorithm::parse(algorithm_text)
        .ok_or_else(|| "verification requires sha256, sha512, or sha1".to_owned())?;
    let (expected, path_text) = take_token(rest);
    if expected.is_empty() || path_text.is_empty() {
        return Err("expected 'verify <algorithm> <digest> <absolute path>'".into());
    }
    Ok(ChecksumRequest::Verify {
        algorithm,
        expected: normalize_expected(algorithm, expected)?,
        path: parse_path(path_text)?,
    })
}

fn take_token(value: &str) -> (&str, &str) {
    let value = value.trim_start();
    match value.find(char::is_whitespace) {
        Some(index) => (&value[..index], value[index..].trim_start()),
        None => (value, ""),
    }
}

fn normalize_expected(algorithm: HashAlgorithm, value: &str) -> Result<String, String> {
    let value = value
        .split_once(':')
        .filter(|(prefix, _)| HashAlgorithm::parse(prefix) == Some(algorithm))
        .map_or(value, |(_, digest)| digest);
    if value.len() != algorithm.digest_length()
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "{} digest must contain exactly {} hexadecimal characters",
            algorithm.label(),
            algorithm.digest_length()
        ));
    }
    Ok(value.to_ascii_lowercase())
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

#[cfg(test)]
fn evaluate(request: &ChecksumRequest) -> Result<Evaluation, String> {
    evaluate_with_context(request, &SearchContext::new(0, None))
}
fn evaluate_with_context(
    request: &ChecksumRequest,
    context: &SearchContext,
) -> Result<Evaluation, String> {
    match request {
        ChecksumRequest::Calculate { algorithms, path } => {
            calculate_results(hash_file_with_context(path, algorithms, context)?)
        }
        ChecksumRequest::Verify {
            algorithm,
            expected,
            path,
        } => verify_result(
            hash_file_with_context(path, &[*algorithm], context)?,
            *algorithm,
            expected,
        ),
    }
}

#[cfg(test)]
fn hash_file(path: &Path, algorithms: &[HashAlgorithm]) -> Result<HashedFile, String> {
    hash_file_with_context(path, algorithms, &SearchContext::new(0, None))
}
fn hash_file_with_context(
    path: &Path,
    algorithms: &[HashAlgorithm],
    context: &SearchContext,
) -> Result<HashedFile, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {}: {error}", safe_path(path)))?;
    let file = File::open(&canonical)
        .map_err(|error| format!("cannot open {}: {error}", safe_path(&canonical)))?;
    let initial = file
        .metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", safe_path(&canonical)))?;
    if !initial.is_file() {
        return Err("checksum input must be a regular file".into());
    }
    if initial.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file is {}; the checksum limit is {}",
            format_bytes(initial.len()),
            format_bytes(MAX_FILE_BYTES)
        ));
    }
    let initial_modified = initial.modified().ok();
    let mut contexts: Vec<(HashAlgorithm, Context)> = algorithms
        .iter()
        .copied()
        .map(|algorithm| (algorithm, Context::new(algorithm.ring_algorithm())))
        .collect();
    let mut reader = BufReader::with_capacity(READ_BUFFER_BYTES, file);
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    loop {
        context.check()?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", safe_path(&canonical)))?;
        if read == 0 {
            break;
        }
        for (_, context) in &mut contexts {
            context.update(&buffer[..read]);
        }
    }
    let final_metadata = reader
        .get_ref()
        .metadata()
        .map_err(|error| format!("cannot recheck {}: {error}", safe_path(&canonical)))?;
    if initial.len() != final_metadata.len() || initial_modified != final_metadata.modified().ok() {
        return Err("file changed while its checksum was being calculated; try again".into());
    }
    let hashes = contexts
        .into_iter()
        .map(|(algorithm, context)| HashValue {
            algorithm,
            value: hex(context.finish().as_ref()),
        })
        .collect();
    Ok(HashedFile {
        path: canonical,
        size: initial.len(),
        hashes,
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn calculate_results(file: HashedFile) -> Result<Evaluation, String> {
    let path = file.path.to_string_lossy().into_owned();
    let filename = file
        .path
        .file_name()
        .map(|name| safe_text(&name.to_string_lossy()))
        .unwrap_or_else(|| safe_path(&file.path));
    let mut results = Vec::new();
    let mut action_values = BTreeMap::new();
    for hash in file.hashes {
        let result_id = format!("file-checksum:{}", hash.algorithm.slug());
        action_values.insert((result_id.clone(), "paste_hash".into()), hash.value.clone());
        action_values.insert((result_id.clone(), "copy_hash".into()), hash.value.clone());
        action_values.insert((result_id.clone(), "copy_path".into()), path.clone());
        let legacy_note = if hash.algorithm == HashAlgorithm::Sha1 {
            " · legacy algorithm"
        } else {
            ""
        };
        results.push(SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: result_id,
            title: format!("{} · {}", hash.algorithm.label(), hash.value),
            subtitle: format!(
                "{} · {} · local file checksum{}",
                format_bytes(file.size),
                filename,
                legacy_note
            ),
            preview: format!(
                "{}\n{}\n\nFile: {}\nSize: {} ({})",
                hash.algorithm.label(),
                hash.value,
                safe_path(&file.path),
                format_bytes(file.size),
                file.size
            ),
            kind: ResultKind::Command,
            score: 3_420.0,
            metadata: BTreeMap::from([
                ("symbol_name".into(), "checkmark.seal".into()),
                ("preview_style".into(), "text".into()),
            ]),
            actions: vec![
                action("paste_hash", "Paste Checksum", true, true, true),
                action("copy_hash", "Copy Checksum", false, false, false),
                action("copy_path", "Copy File Path", false, false, false),
            ],
        });
    }
    Ok(Evaluation {
        results,
        action_values,
    })
}

fn verify_result(
    file: HashedFile,
    algorithm: HashAlgorithm,
    expected: &str,
) -> Result<Evaluation, String> {
    let actual = file
        .hashes
        .first()
        .ok_or_else(|| "checksum calculation returned no digest".to_owned())?
        .value
        .clone();
    let matches = actual == expected;
    let result_id = "file-checksum:verify".to_owned();
    let path = file.path.to_string_lossy().into_owned();
    let mut action_values = BTreeMap::new();
    action_values.insert((result_id.clone(), "copy_actual".into()), actual.clone());
    action_values.insert(
        (result_id.clone(), "copy_expected".into()),
        expected.to_owned(),
    );
    action_values.insert((result_id.clone(), "copy_path".into()), path);
    let (title, symbol, color) = if matches {
        ("Checksum matches", "checkmark.seal.fill", "#30D158")
    } else {
        ("Checksum does not match", "xmark.seal.fill", "#FF453A")
    };
    let legacy_note = if algorithm == HashAlgorithm::Sha1 {
        "\n\nSHA-1 is retained only for compatibility and should not be used as a security guarantee."
    } else {
        ""
    };
    let result = SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: title.into(),
        subtitle: format!(
            "{} · {} · {}",
            algorithm.label(),
            format_bytes(file.size),
            safe_text(&file.path.file_name().unwrap_or_default().to_string_lossy())
        ),
        preview: format!(
            "{}\nExpected: {}\nActual:   {}\n\nFile: {}\nSize: {} ({}){}",
            title,
            expected,
            actual,
            safe_path(&file.path),
            format_bytes(file.size),
            file.size,
            legacy_note
        ),
        kind: ResultKind::Command,
        score: 3_425.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), symbol.into()),
            ("symbol_color".into(), color.into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("copy_actual", "Copy Actual Checksum", true, false, false),
            action(
                "copy_expected",
                "Copy Expected Checksum",
                false,
                false,
                false,
            ),
            action("copy_path", "Copy File Path", false, false, false),
        ],
    };
    Ok(Evaluation {
        results: vec![result],
        action_values,
    })
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

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "file-checksum:invalid".into(),
        title: compact(query.trim(), MAX_TITLE_CHARACTERS),
        subtitle: format!("File checksum unavailable · {}", compact(message, 120)),
        preview: format!("File checksum unavailable\n{message}"),
        kind: ResultKind::Command,
        score: 3_420.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn safe_path(path: &Path) -> String {
    safe_text(&path.to_string_lossy())
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
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

    fn temporary_file(contents: &[u8]) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "clipkeep-file-checksum-{}-{nonce}-{sequence}.bin",
            std::process::id()
        ));
        let mut file = File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
        path
    }

    #[test]
    fn parses_calculation_commands_algorithms_and_aliases() {
        let request = parse_request("checksum /tmp/file.bin").unwrap().unwrap();
        assert_eq!(
            request,
            ChecksumRequest::Calculate {
                algorithms: vec![HashAlgorithm::Sha256, HashAlgorithm::Sha512],
                path: PathBuf::from("/tmp/file.bin"),
            }
        );
        let request = parse_request("HASHFILE SHA-512 /tmp/file.bin")
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            ChecksumRequest::Calculate {
                algorithms: vec![HashAlgorithm::Sha512],
                path: PathBuf::from("/tmp/file.bin"),
            }
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn parses_verification_and_normalizes_expected_digest() {
        let digest = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        let request = parse_request(&format!("checksum verify sha256 {digest} /tmp/file.bin"))
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            ChecksumRequest::Verify {
                algorithm: HashAlgorithm::Sha256,
                expected: digest.to_ascii_lowercase(),
                path: PathBuf::from("/tmp/file.bin"),
            }
        );
        assert!(parse_request("checksum verify md5 deadbeef /tmp/file").is_err());
        assert!(parse_request("checksum verify sha256 bad /tmp/file").is_err());
        assert_eq!(
            normalize_expected(HashAlgorithm::Sha256, &format!("SHA256:{digest}")).unwrap(),
            digest.to_ascii_lowercase()
        );
    }

    #[test]
    fn paths_support_quotes_tilde_and_local_file_urls() {
        assert_eq!(
            parse_path("\"/tmp/file with spaces.bin\"").unwrap(),
            PathBuf::from("/tmp/file with spaces.bin")
        );
        assert!(parse_path("relative/file.bin").is_err());
        assert!(parse_path("file://remote.example/tmp/file.bin").is_err());
        let file_url = Url::from_file_path("/tmp/file with spaces.bin").unwrap();
        assert_eq!(
            parse_path(file_url.as_str()).unwrap(),
            PathBuf::from("/tmp/file with spaces.bin")
        );
        if let Some(home) = env::var_os("HOME") {
            assert_eq!(
                parse_path("~/file.bin").unwrap(),
                PathBuf::from(home).join("file.bin")
            );
        }
    }

    #[test]
    fn hashes_a_file_with_known_sha_values_in_one_stream() {
        let path = temporary_file(b"abc");
        let file = hash_file(
            &path,
            &[
                HashAlgorithm::Sha256,
                HashAlgorithm::Sha512,
                HashAlgorithm::Sha1,
            ],
        )
        .unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(file.size, 3);
        assert_eq!(
            file.hashes[0].value,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            file.hashes[1].value,
            concat!(
                "ddaf35a193617abacc417349ae204131",
                "12e6fa4e89a97ea20a9eeee64b55d39a",
                "2192992a274fc1a836ba3c23a3feebbd",
                "454d4423643ce80e2a9ac94fa54ca49f"
            )
        );
        assert_eq!(
            file.hashes[2].value,
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn verification_distinguishes_matches_and_mismatches() {
        let path = temporary_file(b"abc");
        let matching = ChecksumRequest::Verify {
            algorithm: HashAlgorithm::Sha256,
            expected: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            path: path.clone(),
        };
        let result = evaluate(&matching).unwrap().results.remove(0);
        assert_eq!(result.title, "Checksum matches");
        assert_eq!(result.metadata["symbol_color"], "#30D158");

        let mismatch = ChecksumRequest::Verify {
            algorithm: HashAlgorithm::Sha256,
            expected: "0".repeat(64),
            path: path.clone(),
        };
        let result = evaluate(&mismatch).unwrap().results.remove(0);
        fs::remove_file(path).unwrap();
        assert_eq!(result.title, "Checksum does not match");
        assert_eq!(result.metadata["symbol_color"], "#FF453A");
    }

    #[test]
    fn rejects_directories_and_missing_files_without_actions() {
        let request = ChecksumRequest::Calculate {
            algorithms: vec![HashAlgorithm::Sha256],
            path: env::temp_dir(),
        };
        match evaluate(&request) {
            Err(message) => assert!(message.contains("regular file")),
            Ok(_) => panic!("directory should not be hashed"),
        }

        let mut plugin = FileChecksumPlugin::default();
        let result = plugin
            .search("checksum /definitely/not/a/file", 20)
            .unwrap()
            .remove(0);
        assert!(result.subtitle.starts_with("File checksum unavailable"));
        assert!(result.actions.is_empty());
    }

    #[test]
    fn calculation_results_are_rankable_bounded_and_actionable() {
        let path = temporary_file(b"abc");
        let mut plugin = FileChecksumPlugin::default();
        let results = plugin
            .search(&format!("checksum {}", path.display()), 1)
            .unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].title.starts_with("SHA-256 · ba7816"));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(results[0].metadata["preview_style"], "text");
        assert_eq!(
            plugin.action_values[&(results[0].id.clone(), "copy_hash".into())],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha1_results_are_explicitly_marked_as_legacy() {
        let path = temporary_file(b"abc");
        let mut plugin = FileChecksumPlugin::default();
        let result = plugin
            .search(&format!("checksum sha1 {}", path.display()), 20)
            .unwrap()
            .remove(0);
        fs::remove_file(path).unwrap();
        assert!(result.title.starts_with("SHA-1 (legacy)"));
        assert!(result.subtitle.contains("legacy algorithm"));
    }

    #[test]
    fn safe_text_escapes_controls_and_directional_overrides() {
        assert_eq!(
            safe_text("safe\u{202e}name\n"),
            "safe\\u{202E}name\\u{000A}"
        );
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(compact("甲乙丙", 2), "甲乙…");
    }

    #[test]
    fn searches_invalidate_old_actions_and_honor_zero_limit() {
        let path = temporary_file(b"abc");
        let mut plugin = FileChecksumPlugin::default();
        plugin
            .search(&format!("checksum sha256 {}", path.display()), 20)
            .unwrap();
        assert!(!plugin.action_values.is_empty());
        assert!(plugin.search("ordinary search", 20).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(
            plugin
                .search(&format!("checksum {}", path.display()), 0)
                .unwrap()
                .is_empty()
        );
        fs::remove_file(path).unwrap();
    }
}
