use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    time::{SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.jwt-inspector";
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_DECODED_SEGMENT_BYTES: usize = 64 * 1024;

fn main() -> io::Result<()> {
    run_plugin(JwtInspectorPlugin::default())
}

#[derive(Default)]
struct JwtInspectorPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for JwtInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(token) = parse_command(query) else {
            return Ok(Vec::new());
        };
        if token.len() > MAX_TOKEN_BYTES {
            return Ok(vec![invalid_result(query, "token exceeds 64 KiB")]);
        }
        let decoded = match inspect_jwt(token, now_unix_seconds()) {
            Ok(decoded) => decoded,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let parts = decoded.parts();
        Ok(parts
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, part)| {
                let result_id = format!("jwt-inspector:{}", part.id);
                self.recent_values
                    .insert(result_id.clone(), part.value.clone());
                jwt_result(query, part, result_id, index == 0, 3_425.0 - index as f32)
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown JWT inspector action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "decoded JWT result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Decoded JWT JSON copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DecodedJwt {
    header: Value,
    payload: Value,
    header_pretty: String,
    payload_pretty: String,
    algorithm: String,
    time_status: String,
    time_detail: String,
    signature_detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct JwtPart {
    id: &'static str,
    label: &'static str,
    symbol: &'static str,
    value: String,
    detail: String,
}

impl DecodedJwt {
    fn parts(&self) -> Vec<JwtPart> {
        vec![
            JwtPart {
                id: "payload",
                label: "JWT Payload",
                symbol: "doc.text.magnifyingglass",
                value: self.payload_pretty.clone(),
                detail: format!(
                    "{} · {} · {}",
                    self.time_status, self.time_detail, self.signature_detail
                ),
            },
            JwtPart {
                id: "header",
                label: "JWT Header",
                symbol: "curlybraces",
                value: self.header_pretty.clone(),
                detail: format!(
                    "Declared algorithm: {} · {}",
                    self.algorithm, self.signature_detail
                ),
            },
        ]
    }
}

fn parse_command(query: &str) -> Option<&str> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    if !query[..command_end].eq_ignore_ascii_case("jwt") {
        return None;
    }
    let mut token = query[command_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace())
        .trim();
    if token
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("decode"))
        && token
            .get(6..7)
            .is_some_and(|separator| separator.chars().all(char::is_whitespace))
    {
        token = token[7..].trim();
    }
    (!token.is_empty()).then_some(token)
}

fn inspect_jwt(token: &str, now: i64) -> Result<DecodedJwt, String> {
    let segments: Vec<&str> = token.split('.').collect();
    if segments.len() != 3 {
        return Err("expected three dot-separated JWT segments".into());
    }
    let header = decode_json_segment(segments[0], "header")?;
    let payload = decode_json_segment(segments[1], "payload")?;
    let header_object = header
        .as_object()
        .ok_or_else(|| "JWT header must be a JSON object".to_owned())?;
    let payload_object = payload
        .as_object()
        .ok_or_else(|| "JWT payload must be a JSON object".to_owned())?;
    let signature_detail = inspect_signature(segments[2])?;
    let algorithm = header_object
        .get("alg")
        .and_then(Value::as_str)
        .unwrap_or("not declared")
        .to_owned();
    let (time_status, time_detail) = inspect_time_claims(payload_object, now);
    let header_pretty = serde_json::to_string_pretty(&header).map_err(|error| error.to_string())?;
    let payload_pretty =
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    Ok(DecodedJwt {
        header,
        payload,
        header_pretty,
        payload_pretty,
        algorithm,
        time_status,
        time_detail,
        signature_detail,
    })
}

fn decode_json_segment(segment: &str, label: &str) -> Result<Value, String> {
    if segment.is_empty() {
        return Err(format!("JWT {label} segment is empty"));
    }
    let decoded = decode_base64url(segment)
        .map_err(|error| format!("invalid JWT {label} Base64URL: {error}"))?;
    if decoded.len() > MAX_DECODED_SEGMENT_BYTES {
        return Err(format!("decoded JWT {label} exceeds 64 KiB"));
    }
    serde_json::from_slice(&decoded).map_err(|error| format!("invalid JWT {label} JSON: {error}"))
}

fn decode_base64url(segment: &str) -> Result<Vec<u8>, base64::DecodeError> {
    URL_SAFE_NO_PAD
        .decode(segment)
        .or_else(|_| URL_SAFE.decode(segment))
}

fn inspect_signature(segment: &str) -> Result<String, String> {
    if segment.is_empty() {
        return Ok("signature absent and not verified".into());
    }
    let decoded = decode_base64url(segment)
        .map_err(|error| format!("invalid JWT signature Base64URL: {error}"))?;
    Ok(format!(
        "signature present ({} bytes) but not verified",
        decoded.len()
    ))
}

fn inspect_time_claims(payload: &Map<String, Value>, now: i64) -> (String, String) {
    let expiration = numeric_date(payload.get("exp"));
    let not_before = numeric_date(payload.get("nbf"));
    let issued_at = numeric_date(payload.get("iat"));

    let status = if expiration.is_some_and(|value| value <= now) {
        "Expired claim (unverified)"
    } else if not_before.is_some_and(|value| value > now) {
        "Not-active-yet claim (unverified)"
    } else if expiration.is_some() || not_before.is_some() {
        "Time claims currently active (unverified)"
    } else {
        "No expiration or not-before claim"
    };

    let mut details = Vec::new();
    if let Some(value) = expiration {
        details.push(format!("exp {value} ({})", relative_time(value, now)));
    }
    if let Some(value) = not_before {
        details.push(format!("nbf {value} ({})", relative_time(value, now)));
    }
    if let Some(value) = issued_at {
        details.push(format!("iat {value} ({})", relative_time(value, now)));
    }
    if details.is_empty() {
        details.push("no numeric exp, nbf, or iat claims".into());
    }
    (status.into(), details.join(" · "))
}

fn numeric_date(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| {
            value.as_f64().and_then(|number| {
                let truncated = number.trunc();
                (number.is_finite()
                    && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
                        .contains(&truncated))
                .then_some(truncated as i64)
            })
        })
}

fn relative_time(value: i64, now: i64) -> String {
    let delta = i128::from(value) - i128::from(now);
    if delta == 0 {
        return "now".into();
    }
    let absolute = delta.unsigned_abs();
    let (amount, unit) = if absolute >= 86_400 {
        (absolute / 86_400, "d")
    } else if absolute >= 3_600 {
        (absolute / 3_600, "h")
    } else if absolute >= 60 {
        (absolute / 60, "m")
    } else {
        (absolute, "s")
    };
    if delta > 0 {
        format!("in {amount}{unit}")
    } else {
        format!("{amount}{unit} ago")
    }
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn jwt_result(
    query: &str,
    part: JwtPart,
    result_id: String,
    primary: bool,
    score: f32,
) -> SearchResult {
    let title = if primary {
        query.into()
    } else {
        format!("{query} — {}", part.label)
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{} · {}", part.label, compact(&part.detail, 120)),
        preview: format!("{}\n\n{}", part.value, part.detail),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([
            ("symbol_name".into(), part.symbol.into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            ResultAction {
                id: "paste".into(),
                title: format!("Paste {}", part.label),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                id: "copy".into(),
                title: format!("Copy {}", part.label),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "jwt-inspector:invalid".into(),
        title: query.into(),
        subtitle: format!("Invalid JWT · {}", compact(message, 120)),
        preview: format!("Invalid JWT\n{message}\n\nNo signature verification was attempted."),
        kind: ResultKind::Command,
        score: 3_425.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
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

    const EXAMPLE_TOKEN: &str = concat!(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
        "eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.",
        "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
    );

    fn token_with_payload(payload: Value) -> String {
        let header = serde_json::json!({"alg": "none", "typ": "JWT"});
        format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap()),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    #[test]
    fn parses_space_colon_and_decode_commands() {
        assert_eq!(
            parse_command(&format!("jwt {EXAMPLE_TOKEN}")),
            Some(EXAMPLE_TOKEN)
        );
        assert_eq!(
            parse_command(&format!("JWT:{EXAMPLE_TOKEN}")),
            Some(EXAMPLE_TOKEN)
        );
        assert_eq!(
            parse_command(&format!("jwt decode {EXAMPLE_TOKEN}")),
            Some(EXAMPLE_TOKEN)
        );
        assert_eq!(parse_command("jwt"), None);
        assert_eq!(parse_command("ordinary text"), None);
    }

    #[test]
    fn decodes_and_pretty_prints_standard_jwt() {
        let decoded = inspect_jwt(EXAMPLE_TOKEN, 1_700_000_000).unwrap();
        assert_eq!(decoded.algorithm, "HS256");
        assert_eq!(decoded.payload["sub"], "1234567890");
        assert!(decoded.payload_pretty.contains("\"John Doe\""));
        assert_eq!(decoded.time_status, "No expiration or not-before claim");
        assert!(decoded.signature_detail.contains("not verified"));
    }

    #[test]
    fn reports_expired_and_not_active_time_claims_as_unverified() {
        let expired = token_with_payload(serde_json::json!({"exp": 999, "iat": 100}));
        let decoded = inspect_jwt(&expired, 1_000).unwrap();
        assert_eq!(decoded.time_status, "Expired claim (unverified)");
        assert!(decoded.time_detail.contains("exp 999 (1s ago)"));

        let future = token_with_payload(serde_json::json!({"nbf": 1_120}));
        let decoded = inspect_jwt(&future, 1_000).unwrap();
        assert_eq!(decoded.time_status, "Not-active-yet claim (unverified)");
        assert!(decoded.time_detail.contains("in 2m"));
    }

    #[test]
    fn rejects_malformed_segments_and_non_object_json() {
        assert!(inspect_jwt("only.two", 0).is_err());
        assert!(inspect_jwt("not-base64.e30.", 0).is_err());
        let array_payload = token_with_payload(serde_json::json!([1, 2, 3]));
        assert!(inspect_jwt(&array_payload, 0).is_err());
    }

    #[test]
    fn accepts_padded_base64url_and_unsecured_signature_shape() {
        let header = URL_SAFE.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE.encode(br#"{"sub":"demo"}"#);
        let decoded = inspect_jwt(&format!("{header}.{payload}."), 0).unwrap();
        assert_eq!(decoded.payload["sub"], "demo");
        assert_eq!(
            decoded.signature_detail,
            "signature absent and not verified"
        );
    }

    #[test]
    fn search_returns_rankable_payload_header_and_visible_errors() {
        let mut plugin = JwtInspectorPlugin::default();
        let query = format!("jwt {EXAMPLE_TOKEN}");
        let results = plugin.search(&query, 20).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, query);
        assert!(results[0].subtitle.contains("signature present"));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(results[1].actions[1].title, "Copy JWT Header");
        assert_eq!(plugin.recent_values.len(), 2);

        let invalid = plugin.search("jwt not-a-token", 20).unwrap();
        assert_eq!(invalid.len(), 1);
        assert!(invalid[0].subtitle.starts_with("Invalid JWT · "));
        assert_eq!(invalid[0].metadata["symbol_color"], "#FF9F0A");
        assert!(invalid[0].actions.is_empty());
        assert!(plugin.recent_values.is_empty());
    }

    #[test]
    fn compact_is_unicode_safe() {
        assert_eq!(compact("甲乙丙丁", 3), "甲乙丙…");
    }
}
