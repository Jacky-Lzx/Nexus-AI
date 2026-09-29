use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    time::{SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.id-generator";
const DEFAULT_NANO_ID_LENGTH: usize = 21;
const MAX_NANO_ID_LENGTH: usize = 128;
const NANO_ID_ALPHABET: &[u8; 64] =
    b"_-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ULID_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn main() -> io::Result<()> {
    run_plugin(IdGeneratorPlugin::default())
}

#[derive(Default)]
struct IdGeneratorPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for IdGeneratorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let query = query.trim();
        let Some(request) = parse_request(query) else {
            return Ok(Vec::new());
        };
        let generated = generate(request)?;
        Ok(generated
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, identifier)| {
                let result_id = format!("id-generator:{}", identifier.id);
                self.recent_values
                    .insert(result_id.clone(), identifier.value.clone());
                identifier_result(
                    query,
                    identifier,
                    result_id,
                    index == 0,
                    3_400.0 - index as f32,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown identifier generator action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "generated identifier is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Identifier copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdRequest {
    All,
    Uuids,
    Uuid4,
    Uuid7,
    Ulid,
    NanoId(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GeneratedId {
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    symbol: &'static str,
    value: String,
}

fn parse_request(query: &str) -> Option<IdRequest> {
    let normalized = query.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "id" | "ids" | "identifier" | "identifiers" | "id generator" | "标识符" | "唯一标识符" => {
            Some(IdRequest::All)
        }
        "uuid" => Some(IdRequest::Uuids),
        "uuid4" | "uuid 4" | "uuid v4" | "uuid-v4" => Some(IdRequest::Uuid4),
        "uuid7" | "uuid 7" | "uuid v7" | "uuid-v7" => Some(IdRequest::Uuid7),
        "ulid" => Some(IdRequest::Ulid),
        "nanoid" | "nano id" => Some(IdRequest::NanoId(DEFAULT_NANO_ID_LENGTH)),
        _ => parse_nano_id_length(&normalized).map(IdRequest::NanoId),
    }
}

fn parse_nano_id_length(query: &str) -> Option<usize> {
    let length = query
        .strip_prefix("nanoid ")
        .or_else(|| query.strip_prefix("nano id "))?
        .trim()
        .parse::<usize>()
        .ok()?;
    (1..=MAX_NANO_ID_LENGTH).contains(&length).then_some(length)
}

fn generate(request: IdRequest) -> Result<Vec<GeneratedId>, String> {
    let mut generated = Vec::new();
    if matches!(
        request,
        IdRequest::All | IdRequest::Uuids | IdRequest::Uuid4
    ) {
        generated.push(GeneratedId {
            id: "uuid4",
            label: "UUID v4",
            detail: "Random RFC 9562 identifier",
            symbol: "4.square.fill",
            value: generate_uuid4()?,
        });
    }
    if matches!(
        request,
        IdRequest::All | IdRequest::Uuids | IdRequest::Uuid7
    ) {
        generated.push(GeneratedId {
            id: "uuid7",
            label: "UUID v7",
            detail: "Time-ordered RFC 9562 identifier",
            symbol: "7.square.fill",
            value: generate_uuid7()?,
        });
    }
    if matches!(request, IdRequest::All | IdRequest::Ulid) {
        generated.push(GeneratedId {
            id: "ulid",
            label: "ULID",
            detail: "Time-sortable Crockford Base32 identifier",
            symbol: "clock.badge.checkmark",
            value: generate_ulid()?,
        });
    }
    let nano_length = match request {
        IdRequest::All => Some(DEFAULT_NANO_ID_LENGTH),
        IdRequest::NanoId(length) => Some(length),
        _ => None,
    };
    if let Some(length) = nano_length {
        generated.push(GeneratedId {
            id: "nanoid",
            label: "Nano ID",
            detail: "URL-safe random identifier",
            symbol: "character.cursor.ibeam",
            value: generate_nano_id(length)?,
        });
    }
    Ok(generated)
}

fn generate_uuid4() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes)?;
    Ok(uuid4_from_bytes(bytes))
}

fn uuid4_from_bytes(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(bytes)
}

fn generate_uuid7() -> Result<String, String> {
    let mut random = [0_u8; 10];
    fill_random(&mut random)?;
    Ok(uuid7_from_parts(now_milliseconds(), random))
}

fn uuid7_from_parts(timestamp_milliseconds: u64, random: [u8; 10]) -> String {
    let timestamp = timestamp_milliseconds.to_be_bytes();
    let mut bytes = [0_u8; 16];
    bytes[..6].copy_from_slice(&timestamp[2..]);
    bytes[6] = 0x70 | (random[0] & 0x0f);
    bytes[7] = random[1];
    bytes[8] = 0x80 | (random[2] & 0x3f);
    bytes[9..].copy_from_slice(&random[3..]);
    format_uuid(bytes)
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

fn generate_ulid() -> Result<String, String> {
    let mut random = [0_u8; 10];
    fill_random(&mut random)?;
    Ok(ulid_from_parts(now_milliseconds(), random))
}

fn ulid_from_parts(timestamp_milliseconds: u64, random: [u8; 10]) -> String {
    let timestamp = timestamp_milliseconds.to_be_bytes();
    let mut bytes = [0_u8; 16];
    bytes[..6].copy_from_slice(&timestamp[2..]);
    bytes[6..].copy_from_slice(&random);
    let mut value = u128::from_be_bytes(bytes);
    let mut encoded = [b'0'; 26];
    for character in encoded.iter_mut().rev() {
        *character = ULID_ALPHABET[(value & 0x1f) as usize];
        value >>= 5;
    }
    String::from_utf8(encoded.to_vec()).expect("ULID alphabet is ASCII")
}

fn generate_nano_id(length: usize) -> Result<String, String> {
    let mut bytes = vec![0_u8; length];
    fill_random(&mut bytes)?;
    Ok(nano_id_from_bytes(&bytes))
}

fn nano_id_from_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| char::from(NANO_ID_ALPHABET[usize::from(*byte & 0x3f)]))
        .collect()
}

fn fill_random(bytes: &mut [u8]) -> Result<(), String> {
    getrandom::getrandom(bytes).map_err(|error| format!("secure random generation failed: {error}"))
}

fn now_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn identifier_result(
    query: &str,
    identifier: GeneratedId,
    result_id: String,
    primary: bool,
    score: f32,
) -> SearchResult {
    let title = if primary {
        query.into()
    } else {
        format!("{query} — {}", identifier.label)
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{} · {}", identifier.label, identifier.value),
        preview: format!("{}\n{}", identifier.value, identifier.detail),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([("symbol_name".into(), identifier.symbol.into())]),
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste Identifier".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy Identifier".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_commands_and_nano_id_lengths() {
        assert_eq!(parse_request("id"), Some(IdRequest::All));
        assert_eq!(parse_request("UUID v4"), Some(IdRequest::Uuid4));
        assert_eq!(parse_request("uuid7"), Some(IdRequest::Uuid7));
        assert_eq!(parse_request("ulid"), Some(IdRequest::Ulid));
        assert_eq!(parse_request("nanoid"), Some(IdRequest::NanoId(21)));
        assert_eq!(parse_request("nanoid 64"), Some(IdRequest::NanoId(64)));
        assert_eq!(parse_request("nanoid 0"), None);
        assert_eq!(parse_request("nanoid 129"), None);
        assert_eq!(parse_request("ordinary text"), None);
    }

    #[test]
    fn uuid4_sets_version_and_variant_bits() {
        assert_eq!(
            uuid4_from_bytes([0; 16]),
            "00000000-0000-4000-8000-000000000000"
        );
        assert_eq!(
            uuid4_from_bytes([0xff; 16]),
            "ffffffff-ffff-4fff-bfff-ffffffffffff"
        );
    }

    #[test]
    fn uuid7_embeds_the_timestamp_and_sets_required_bits() {
        assert_eq!(
            uuid7_from_parts(0x0123_4567_89ab, [0; 10]),
            "01234567-89ab-7000-8000-000000000000"
        );
        let value = uuid7_from_parts(0x0123_4567_89ab, [0xff; 10]);
        assert_eq!(&value[14..15], "7");
        assert!(matches!(&value[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn ulid_is_fixed_width_crockford_base32() {
        assert_eq!(ulid_from_parts(0, [0; 10]), "00000000000000000000000000");
        let value = ulid_from_parts(0x0123_4567_89ab, [0xff; 10]);
        assert_eq!(value.len(), 26);
        assert!(value.bytes().all(|byte| ULID_ALPHABET.contains(&byte)));
        assert!(value.as_bytes()[0] <= b'7');
    }

    #[test]
    fn nano_id_uses_a_uniform_power_of_two_alphabet() {
        assert_eq!(nano_id_from_bytes(&[0, 1, 2, 63, 64]), "_-0Z_");
        assert_eq!(nano_id_from_bytes(&[255; 128]).len(), 128);
    }

    #[test]
    fn all_request_generates_each_supported_identifier() {
        let generated = generate(IdRequest::All).unwrap();
        assert_eq!(
            generated.iter().map(|item| item.id).collect::<Vec<_>>(),
            ["uuid4", "uuid7", "ulid", "nanoid"]
        );
        assert_eq!(generated[0].value.len(), 36);
        assert_eq!(generated[2].value.len(), 26);
        assert_eq!(generated[3].value.len(), DEFAULT_NANO_ID_LENGTH);
    }

    #[test]
    fn search_results_are_rankable_and_invalidate_old_values() {
        let mut plugin = IdGeneratorPlugin::default();
        let results = plugin.search("uuid", 20).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "uuid");
        assert_eq!(results[1].title, "uuid — UUID v7");
        assert!(results[0].subtitle.starts_with("UUID v4 · "));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values.len(), 2);

        plugin.search("not a command", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }
}
