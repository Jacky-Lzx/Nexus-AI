use chrono::{DateTime, SecondsFormat, Utc};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.id-inspector";
const RESULT_ID: &str = "id-inspector:result";
const MAX_INPUT_BYTES: usize = 512;
const UUID_GREGORIAN_OFFSET_TICKS: i128 = 122_192_928_000_000_000;

fn main() -> io::Result<()> {
    run_plugin(IdInspectorPlugin::default())
}

#[derive(Default)]
struct IdInspectorPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for IdInspectorPlugin {
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
        let inspection = match inspect(&request) {
            Ok(inspection) => inspection,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values
            .insert("paste_normalized".into(), inspection.normalized.clone());
        self.action_values
            .insert("copy_normalized".into(), inspection.normalized.clone());
        self.action_values
            .insert("copy_bytes".into(), inspection.bytes_hex.clone());
        if let Some(timestamp) = &inspection.timestamp {
            self.action_values
                .insert("copy_timestamp".into(), timestamp.iso.clone());
            self.action_values.insert(
                "copy_unix_milliseconds".into(),
                timestamp.unix_milliseconds.to_string(),
            );
        }
        Ok(vec![inspection_result(&inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("identifier inspection is no longer active".into());
        }
        let value = self.action_values.get(action_id).ok_or_else(|| {
            format!("unknown or inactive Identifier Inspector action: {action_id}")
        })?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_normalized" | "copy_normalized" => "Normalized identifier copied",
                "copy_bytes" => "Identifier bytes copied",
                "copy_timestamp" => "Embedded timestamp copied",
                "copy_unix_milliseconds" => "Unix milliseconds copied",
                _ => "Identifier value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedKind {
    Auto,
    Uuid,
    Ulid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InspectionRequest {
    expected: ExpectedKind,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EmbeddedTime {
    iso: String,
    unix_milliseconds: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentifierInspection {
    kind: String,
    normalized: String,
    bytes_hex: String,
    subtitle: String,
    report: String,
    timestamp: Option<EmbeddedTime>,
}

fn parse_request(query: &str) -> Result<Option<InspectionRequest>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let expected = match command.to_ascii_lowercase().as_str() {
        "idinfo" | "identifierinfo" => ExpectedKind::Auto,
        "uuidinfo" => ExpectedKind::Uuid,
        "ulidinfo" => ExpectedKind::Ulid,
        _ if command == "标识符信息" => ExpectedKind::Auto,
        _ => return Ok(None),
    };
    let remainder = &query[command_end..];
    let value = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if value.is_empty() {
        return Err("expected a UUID or ULID after the inspection command".into());
    }
    if value.len() > MAX_INPUT_BYTES {
        return Err("identifier input exceeds 512 bytes".into());
    }
    if value.chars().any(char::is_whitespace) {
        return Err("identifier must be one value without whitespace".into());
    }
    Ok(Some(InspectionRequest {
        expected,
        value: value.to_owned(),
    }))
}

fn inspect(request: &InspectionRequest) -> Result<IdentifierInspection, String> {
    match request.expected {
        ExpectedKind::Uuid => inspect_uuid(&request.value),
        ExpectedKind::Ulid => inspect_ulid(&request.value),
        ExpectedKind::Auto => {
            if request.value.chars().count() == 26 {
                inspect_ulid(&request.value)
            } else {
                inspect_uuid(&request.value)
            }
        }
    }
}

fn inspect_uuid(input: &str) -> Result<IdentifierInspection, String> {
    let bytes = parse_uuid(input)?;
    let normalized = format_uuid(bytes);
    let bytes_hex = format_bytes_hex(&bytes);
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(special_uuid_inspection(
            "Nil UUID",
            "All 128 bits are zero",
            normalized,
            bytes_hex,
        ));
    }
    if bytes.iter().all(|byte| *byte == u8::MAX) {
        return Ok(special_uuid_inspection(
            "Max UUID",
            "All 128 bits are one",
            normalized,
            bytes_hex,
        ));
    }
    let variant = uuid_variant(bytes[8]);
    if bytes[8] & 0xc0 != 0x80 {
        return Ok(IdentifierInspection {
            kind: "UUID".into(),
            normalized: normalized.clone(),
            bytes_hex: bytes_hex.clone(),
            subtitle: format!("{variant} · no RFC version field"),
            report: format!(
                "Identifier information\nType: UUID\nVariant: {variant}\nVersion: not defined for this variant\nNormalized: {normalized}\nBytes: {bytes_hex}\n128-bit integer: {}\nEmbedded time: not applicable",
                u128::from_be_bytes(bytes)
            ),
            timestamp: None,
        });
    }
    let version = bytes[6] >> 4;
    let version_description = uuid_version_description(version);
    let timestamp = uuid_timestamp(bytes, version)?;
    let mut lines = vec![
        "Identifier information".to_owned(),
        format!("Type: UUID v{version}"),
        format!("Version: {version} ({version_description})"),
        format!("Variant: {variant}"),
        format!("Normalized: {normalized}"),
        format!("Bytes: {bytes_hex}"),
        format!("128-bit integer: {}", u128::from_be_bytes(bytes)),
    ];
    if matches!(version, 1 | 6) {
        let clock_sequence = (u16::from(bytes[8] & 0x3f) << 8) | u16::from(bytes[9]);
        let node = bytes[10..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        let node_kind = if bytes[10] & 0x01 == 0 {
            "unicast/MAC-compatible"
        } else {
            "multicast/random-style"
        };
        lines.push(format!("Clock sequence: {clock_sequence}"));
        lines.push(format!("Node: {node} ({node_kind})"));
    }
    if let Some(timestamp) = &timestamp {
        lines.push(format!("Embedded time: {}", timestamp.iso));
        lines.push(format!(
            "Unix milliseconds: {}",
            timestamp.unix_milliseconds
        ));
    } else {
        lines.push("Embedded time: not defined for this UUID version".into());
    }
    Ok(IdentifierInspection {
        kind: format!("UUID v{version}"),
        normalized,
        bytes_hex,
        subtitle: format!("{version_description} · {variant}"),
        report: lines.join("\n"),
        timestamp,
    })
}

fn special_uuid_inspection(
    kind: &str,
    description: &str,
    normalized: String,
    bytes_hex: String,
) -> IdentifierInspection {
    IdentifierInspection {
        kind: kind.into(),
        subtitle: description.into(),
        report: format!(
            "Identifier information\nType: {kind}\nNormalized: {normalized}\nBytes: {bytes_hex}\n128-bit integer: {}\nEmbedded time: not applicable",
            if kind == "Nil UUID" {
                "0"
            } else {
                "340282366920938463463374607431768211455"
            }
        ),
        normalized,
        bytes_hex,
        timestamp: None,
    }
}

fn parse_uuid(input: &str) -> Result<[u8; 16], String> {
    let mut value = input;
    if value
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("urn:uuid:"))
    {
        value = &value[9..];
    }
    if value.starts_with('{') || value.ends_with('}') {
        if !(value.starts_with('{') && value.ends_with('}') && value.len() >= 2) {
            return Err("UUID braces must be paired".into());
        }
        value = &value[1..value.len() - 1];
    }
    let compact = match value.len() {
        32 if value.bytes().all(|byte| byte.is_ascii_hexdigit()) => value.to_owned(),
        36 => {
            if value.as_bytes().iter().enumerate().any(|(index, byte)| {
                matches!(index, 8 | 13 | 18 | 23) != (*byte == b'-')
                    || (!matches!(index, 8 | 13 | 18 | 23) && !byte.is_ascii_hexdigit())
            }) {
                return Err("UUID hyphens must use the 8-4-4-4-12 layout".into());
            }
            value
                .chars()
                .filter(|character| *character != '-')
                .collect()
        }
        _ => return Err("UUID must contain 32 hexadecimal digits".into()),
    };
    let mut bytes = [0_u8; 16];
    for (index, output) in bytes.iter_mut().enumerate() {
        *output = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
            .map_err(|_| "UUID contains an invalid hexadecimal digit")?;
    }
    Ok(bytes)
}

fn inspect_ulid(input: &str) -> Result<IdentifierInspection, String> {
    let (normalized, bytes) = parse_ulid(input)?;
    let timestamp_milliseconds = u64::from_be_bytes([
        0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
    ]);
    let timestamp = unix_millisecond_time(timestamp_milliseconds as i128)?;
    let bytes_hex = format_bytes_hex(&bytes);
    let randomness = bytes[6..]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let report = format!(
        "Identifier information\nType: ULID\nEncoding: Crockford Base32\nNormalized: {normalized}\nBytes: {bytes_hex}\n128-bit integer: {}\nEmbedded time: {}\nUnix milliseconds: {}\nRandomness (80-bit hex): {randomness}",
        u128::from_be_bytes(bytes),
        timestamp.iso,
        timestamp.unix_milliseconds
    );
    Ok(IdentifierInspection {
        kind: "ULID".into(),
        normalized,
        bytes_hex,
        subtitle: format!("Crockford Base32 · {}", timestamp.iso),
        report,
        timestamp: Some(timestamp),
    })
}

fn parse_ulid(input: &str) -> Result<(String, [u8; 16]), String> {
    if input.chars().count() != 26 || !input.is_ascii() {
        return Err("ULID must contain exactly 26 ASCII characters".into());
    }
    let normalized = input.to_ascii_uppercase();
    if normalized
        .as_bytes()
        .first()
        .is_none_or(|byte| !matches!(byte, b'0'..=b'7'))
    {
        return Err("ULID timestamp exceeds 48 bits".into());
    }
    let mut value = 0_u128;
    for character in normalized.bytes() {
        let digit = crockford_value(character).ok_or_else(|| {
            format!(
                "ULID contains invalid Crockford character '{}'",
                char::from(character)
            )
        })?;
        value = value
            .checked_mul(32)
            .and_then(|value| value.checked_add(u128::from(digit)))
            .ok_or_else(|| "ULID value exceeds 128 bits".to_owned())?;
    }
    Ok((normalized, value.to_be_bytes()))
}

fn crockford_value(character: u8) -> Option<u8> {
    match character {
        b'0'..=b'9' => Some(character - b'0'),
        b'A'..=b'H' => Some(character - b'A' + 10),
        b'J'..=b'K' => Some(character - b'J' + 18),
        b'M'..=b'N' => Some(character - b'M' + 20),
        b'P'..=b'T' => Some(character - b'P' + 22),
        b'V'..=b'Z' => Some(character - b'V' + 27),
        _ => None,
    }
}

fn uuid_timestamp(bytes: [u8; 16], version: u8) -> Result<Option<EmbeddedTime>, String> {
    match version {
        1 => {
            let low = u64::from(u32::from_be_bytes(bytes[0..4].try_into().unwrap()));
            let middle = u64::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
            let high = u64::from(u16::from_be_bytes(bytes[6..8].try_into().unwrap()) & 0x0fff);
            gregorian_uuid_time((high << 48) | (middle << 32) | low).map(Some)
        }
        6 => {
            let high = bytes[..6]
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            let low = (u64::from(bytes[6] & 0x0f) << 8) | u64::from(bytes[7]);
            gregorian_uuid_time((high << 12) | low).map(Some)
        }
        7 => {
            let milliseconds = bytes[..6]
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            unix_millisecond_time(milliseconds as i128).map(Some)
        }
        _ => Ok(None),
    }
}

fn gregorian_uuid_time(ticks: u64) -> Result<EmbeddedTime, String> {
    let unix_ticks = i128::from(ticks) - UUID_GREGORIAN_OFFSET_TICKS;
    let seconds = unix_ticks.div_euclid(10_000_000);
    let nanoseconds = unix_ticks.rem_euclid(10_000_000) * 100;
    let seconds: i64 = seconds
        .try_into()
        .map_err(|_| "UUID timestamp is outside the supported range")?;
    let datetime = DateTime::<Utc>::from_timestamp(seconds, nanoseconds as u32)
        .ok_or_else(|| "UUID timestamp is outside the supported date range".to_owned())?;
    Ok(EmbeddedTime {
        iso: datetime.to_rfc3339_opts(SecondsFormat::Nanos, true),
        unix_milliseconds: unix_ticks.div_euclid(10_000),
    })
}

fn unix_millisecond_time(milliseconds: i128) -> Result<EmbeddedTime, String> {
    let value: i64 = milliseconds
        .try_into()
        .map_err(|_| "identifier timestamp is outside the supported range")?;
    let datetime = DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| "identifier timestamp is outside the supported date range".to_owned())?;
    Ok(EmbeddedTime {
        iso: datetime.to_rfc3339_opts(SecondsFormat::Millis, true),
        unix_milliseconds: milliseconds,
    })
}

fn uuid_variant(byte: u8) -> &'static str {
    if byte & 0x80 == 0 {
        "NCS legacy"
    } else if byte & 0xc0 == 0x80 {
        "RFC 4122 / RFC 9562"
    } else if byte & 0xe0 == 0xc0 {
        "Microsoft legacy"
    } else {
        "Reserved for future use"
    }
}

fn uuid_version_description(version: u8) -> &'static str {
    match version {
        1 => "Gregorian time and node",
        2 => "DCE security",
        3 => "MD5 name-based",
        4 => "Random",
        5 => "SHA-1 name-based",
        6 => "Reordered Gregorian time and node",
        7 => "Unix Epoch time-ordered",
        8 => "Custom",
        _ => "Unassigned version",
    }
}

fn format_uuid(bytes: [u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn format_bytes_hex(bytes: &[u8; 16]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn inspection_result(inspection: &IdentifierInspection) -> SearchResult {
    let mut actions = vec![
        action(
            "paste_normalized",
            "Paste Normalized Identifier",
            true,
            true,
            true,
        ),
        action(
            "copy_normalized",
            "Copy Normalized Identifier",
            false,
            false,
            false,
        ),
        action("copy_bytes", "Copy Bytes", false, false, false),
    ];
    if inspection.timestamp.is_some() {
        actions.push(action(
            "copy_timestamp",
            "Copy Embedded Timestamp",
            false,
            false,
            false,
        ));
        actions.push(action(
            "copy_unix_milliseconds",
            "Copy Unix Milliseconds",
            false,
            false,
            false,
        ));
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: inspection.kind.clone(),
        subtitle: inspection.subtitle.clone(),
        preview: inspection.report.clone(),
        kind: ResultKind::Command,
        score: 3_430.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "number.square.fill".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "id-inspector:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Identifier inspection unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Identifier inspection unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
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

    #[test]
    fn parses_explicit_commands_and_ignores_other_queries() {
        assert_eq!(
            parse_request("idinfo 00000000-0000-0000-0000-000000000000").unwrap(),
            Some(InspectionRequest {
                expected: ExpectedKind::Auto,
                value: "00000000-0000-0000-0000-000000000000".into(),
            })
        );
        assert_eq!(
            parse_request("uuidinfo: 550e8400-e29b-41d4-a716-446655440000")
                .unwrap()
                .unwrap()
                .expected,
            ExpectedKind::Uuid
        );
        assert_eq!(
            parse_request("ulidinfo 01ARZ3NDEKTSV4RRFFQ69G5FAV")
                .unwrap()
                .unwrap()
                .expected,
            ExpectedKind::Ulid
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn rejects_missing_whitespace_and_oversized_inputs() {
        assert!(parse_request("idinfo").is_err());
        assert!(parse_request("idinfo one two").is_err());
        assert!(parse_request(&format!("idinfo {}", "a".repeat(MAX_INPUT_BYTES + 1))).is_err());
    }

    #[test]
    fn normalizes_supported_uuid_text_forms() {
        let expected = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            inspect_uuid("550E8400E29B41D4A716446655440000")
                .unwrap()
                .normalized,
            expected
        );
        assert_eq!(
            inspect_uuid("{550e8400-e29b-41d4-a716-446655440000}")
                .unwrap()
                .normalized,
            expected
        );
        assert_eq!(
            inspect_uuid("URN:UUID:550e8400-e29b-41d4-a716-446655440000")
                .unwrap()
                .normalized,
            expected
        );
    }

    #[test]
    fn rejects_invalid_uuid_shapes() {
        assert!(inspect_uuid("550e8400-e29b-41d4-a716-44665544000z").is_err());
        assert!(inspect_uuid("550e8400e29b-41d4-a716-446655440000").is_err());
        assert!(inspect_uuid("{550e8400-e29b-41d4-a716-446655440000").is_err());
    }

    #[test]
    fn reports_uuid_versions_variants_and_special_values() {
        let random = inspect_uuid("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(random.kind, "UUID v4");
        assert!(random.subtitle.contains("Random"));
        assert!(random.subtitle.contains("RFC 4122 / RFC 9562"));
        assert!(random.timestamp.is_none());
        assert_eq!(
            inspect_uuid("00000000-0000-0000-0000-000000000000")
                .unwrap()
                .kind,
            "Nil UUID"
        );
        assert_eq!(
            inspect_uuid("ffffffff-ffff-ffff-ffff-ffffffffffff")
                .unwrap()
                .kind,
            "Max UUID"
        );
        let legacy = inspect_uuid("550e8400-e29b-41d4-2716-446655440000").unwrap();
        assert_eq!(legacy.kind, "UUID");
        assert!(legacy.subtitle.contains("NCS legacy"));
        assert!(legacy.timestamp.is_none());
        assert!(legacy.report.contains("Version: not defined"));
    }

    #[test]
    fn extracts_uuid_v1_v6_and_v7_epoch_times() {
        let v1 = inspect_uuid("13814000-1dd2-11b2-8000-000000000000").unwrap();
        assert_eq!(v1.timestamp.unwrap().unix_milliseconds, 0);
        assert!(v1.report.contains("Clock sequence: 0"));
        let v6 = inspect_uuid("1b21dd21-3814-6000-8000-000000000000").unwrap();
        assert_eq!(v6.timestamp.unwrap().unix_milliseconds, 0);
        let v7 = inspect_uuid("00000000-0000-7000-8000-000000000000").unwrap();
        assert_eq!(v7.timestamp.unwrap().iso, "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn decodes_and_normalizes_ulids() {
        let inspection = inspect_ulid("01arz3ndektsv4rrffq69g5fav").unwrap();
        assert_eq!(inspection.normalized, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(
            inspection.timestamp.unwrap().unix_milliseconds,
            1_469_922_850_259
        );
        let epoch = inspect_ulid("00000000000000000000000000").unwrap();
        assert!(epoch.report.contains("1970-01-01T00:00:00.000Z"));
    }

    #[test]
    fn rejects_invalid_ambiguous_and_overflowing_ulids() {
        assert!(inspect_ulid("01ARZ3NDEKTSV4RRFFQ69G5FA").is_err());
        assert!(inspect_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAI").is_err());
        assert!(inspect_ulid("81ARZ3NDEKTSV4RRFFQ69G5FAV").is_err());
    }

    #[test]
    fn result_actions_include_time_only_when_available() {
        let timed = inspect_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let timed_result = inspection_result(&timed);
        assert!(timed_result.actions[0].default);
        assert!(timed_result.actions[0].paste_to_previous);
        assert!(
            timed_result
                .actions
                .iter()
                .any(|action| action.id == "copy_timestamp")
        );
        let random = inspect_uuid("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(inspection_result(&random).actions.len(), 3);
    }

    #[test]
    fn searches_clear_stale_actions_and_honor_limits() {
        let mut plugin = IdInspectorPlugin::default();
        plugin
            .action_values
            .insert("copy_normalized".into(), "stale".into());
        assert!(plugin.search("ordinary search", 10).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        plugin
            .action_values
            .insert("copy_normalized".into(), "stale".into());
        assert!(
            plugin
                .search("idinfo 550e8400-e29b-41d4-a716-446655440000", 0)
                .unwrap()
                .is_empty()
        );
        assert!(plugin.action_values.is_empty());
    }

    #[test]
    fn invalid_result_escapes_control_and_directional_characters() {
        let result = invalid_result("idinfo bad\nvalue\u{202e}", "bad\nvalue");
        assert_eq!(result.title, "idinfo bad\\u{000A}value\\u{202E}");
        assert!(result.preview.contains("bad\\u{000A}value"));
        assert!(!result.title.contains('\u{202e}'));
    }
}
