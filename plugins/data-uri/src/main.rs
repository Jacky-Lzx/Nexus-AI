use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const PLUGIN_ID: &str = "io.clipkeep.data-uri";
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_URI_BYTES: usize = 256 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 256;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(DataUriPlugin::default())
}

#[derive(Default)]
struct DataUriPlugin {
    result_values: BTreeMap<String, String>,
}

impl Plugin for DataUriPlugin {
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
        let transformation = match transform(request) {
            Ok(transformation) => transformation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!(
            "data-uri:{}-{}",
            transformation.direction.id(),
            transformation.encoding.id()
        );
        self.result_values
            .insert(result_id.clone(), transformation.value.clone());
        Ok(vec![data_uri_result(result_id, transformation)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Data URI Tools action: {action_id}"));
        }
        let value = self
            .result_values
            .get(result_id)
            .ok_or_else(|| "Data URI result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Data URI result copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Percent,
    Base64,
}

impl Encoding {
    fn id(self) -> &'static str {
        match self {
            Self::Percent => "percent",
            Self::Base64 => "base64",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Percent => "percent-encoded",
            Self::Base64 => "Base64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Encode,
    Decode,
}

impl Direction {
    fn id(self) -> &'static str {
        match self {
            Self::Encode => "encode",
            Self::Decode => "decode",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Encode => "Create Data URI",
            Self::Decode => "Decode Data URI",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MediaDescriptor {
    display: String,
    charset: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Request {
    Encode {
        encoding: Encoding,
        descriptor: MediaDescriptor,
        text: String,
    },
    Decode {
        uri: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transformation {
    direction: Direction,
    encoding: Encoding,
    descriptor: MediaDescriptor,
    value: String,
    input_bytes: usize,
    output_bytes: usize,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();
    let encode = match command_lower.as_str() {
        "datauri" | "dataurl" | "data-url" => Some(true),
        "datauridecode" | "dataurl-decode" | "undatauri" => Some(false),
        _ if command == "数据URI" => Some(true),
        _ if command == "数据URI解码" => Some(false),
        _ => None,
    };
    let Some(encode) = encode else {
        return Ok(None);
    };
    if query.len() > MAX_URI_BYTES + 1024 {
        return Err("Data URI query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let Some((options, input)) = remainder.split_once("::") else {
        return Err("expected '::' before the text or Data URI".into());
    };
    let input = input.strip_prefix(' ').unwrap_or(input);
    if input.is_empty() {
        return Err("text or Data URI cannot be empty".into());
    }

    if encode {
        if input.len() > MAX_TEXT_BYTES {
            return Err("text to encode exceeds 64 KiB".into());
        }
        let mut option_parts = options.split_whitespace().peekable();
        let encoding = match option_parts.peek().copied() {
            Some(value) if value.eq_ignore_ascii_case("base64") => {
                option_parts.next();
                Encoding::Base64
            }
            Some(value)
                if value.eq_ignore_ascii_case("percent") || value.eq_ignore_ascii_case("url") =>
            {
                option_parts.next();
                Encoding::Percent
            }
            _ => Encoding::Percent,
        };
        let descriptor = option_parts.next().unwrap_or("text/plain;charset=UTF-8");
        if option_parts.next().is_some() {
            return Err("expected at most one media type after the encoding mode".into());
        }
        let descriptor = parse_explicit_descriptor(descriptor)?;
        validate_charset(&descriptor, input.as_bytes())?;
        Ok(Some(Request::Encode {
            encoding,
            descriptor,
            text: input.into(),
        }))
    } else {
        if !options.trim().is_empty() {
            return Err("Data URI decode does not accept options before '::'".into());
        }
        let uri = input.trim();
        if uri.is_empty() {
            return Err("Data URI cannot be empty".into());
        }
        if uri.len() > MAX_URI_BYTES {
            return Err("Data URI exceeds 256 KiB".into());
        }
        Ok(Some(Request::Decode { uri: uri.into() }))
    }
}

fn transform(request: Request) -> Result<Transformation, String> {
    match request {
        Request::Encode {
            encoding,
            descriptor,
            text,
        } => {
            let payload = match encoding {
                Encoding::Percent => percent_encode(text.as_bytes()),
                Encoding::Base64 => BASE64.encode(text.as_bytes()),
            };
            let marker = if encoding == Encoding::Base64 {
                ";base64"
            } else {
                ""
            };
            let value = format!("data:{}{marker},{payload}", descriptor.display);
            if value.len() > MAX_URI_BYTES {
                return Err("generated Data URI exceeds 256 KiB".into());
            }
            Ok(Transformation {
                direction: Direction::Encode,
                encoding,
                descriptor,
                input_bytes: text.len(),
                output_bytes: value.len(),
                value,
            })
        }
        Request::Decode { uri } => {
            let parsed = decode_data_uri(&uri)?;
            Ok(Transformation {
                direction: Direction::Decode,
                encoding: parsed.encoding,
                descriptor: parsed.descriptor,
                input_bytes: uri.len(),
                output_bytes: parsed.text.len(),
                value: parsed.text,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecodedDataUri {
    encoding: Encoding,
    descriptor: MediaDescriptor,
    text: String,
}

fn decode_data_uri(uri: &str) -> Result<DecodedDataUri, String> {
    if uri.len() < 5 || !uri.as_bytes()[..5].eq_ignore_ascii_case(b"data:") {
        return Err("value must begin with the data: scheme".into());
    }
    let body = &uri[5..];
    let Some(comma) = body.find(',') else {
        return Err("Data URI is missing its metadata/payload comma".into());
    };
    let metadata = &body[..comma];
    let payload = &body[comma + 1..];
    let (descriptor, encoding) = parse_data_uri_metadata(metadata)?;
    let payload_bytes = percent_decode(payload, MAX_URI_BYTES)?;
    let decoded = match encoding {
        Encoding::Percent => payload_bytes,
        Encoding::Base64 => BASE64
            .decode(payload_bytes)
            .map_err(|error| format!("invalid Base64 Data URI payload: {error}"))?,
    };
    if decoded.len() > MAX_TEXT_BYTES {
        return Err("decoded Data URI text exceeds 64 KiB".into());
    }
    validate_charset(&descriptor, &decoded)?;
    let text = String::from_utf8(decoded)
        .map_err(|_| "decoded Data URI payload is not UTF-8 text".to_owned())?;
    Ok(DecodedDataUri {
        encoding,
        descriptor,
        text,
    })
}

fn parse_explicit_descriptor(value: &str) -> Result<MediaDescriptor, String> {
    if value.len() > MAX_DESCRIPTOR_BYTES {
        return Err("media descriptor exceeds 256 bytes".into());
    }
    let mut parts = value.split(';');
    let media_type = parts.next().unwrap_or_default();
    if media_type.is_empty() {
        return Err("media type cannot be empty when creating a Data URI".into());
    }
    normalize_descriptor(media_type, parts, None)
}

fn parse_data_uri_metadata(metadata: &str) -> Result<(MediaDescriptor, Encoding), String> {
    if metadata.len() > MAX_DESCRIPTOR_BYTES {
        return Err("Data URI media descriptor exceeds 256 bytes".into());
    }
    if !metadata.is_ascii() {
        return Err("Data URI media descriptor must be ASCII".into());
    }
    let mut parts = metadata.split(';').collect::<Vec<_>>();
    let encoding = if parts
        .last()
        .is_some_and(|part| part.eq_ignore_ascii_case("base64"))
    {
        parts.pop();
        Encoding::Base64
    } else {
        Encoding::Percent
    };
    let media_type = parts.first().copied().unwrap_or_default();
    let parameters = parts.into_iter().skip(1);
    let default_charset = media_type.is_empty().then_some("US-ASCII");
    let media_type = if media_type.is_empty() {
        "text/plain"
    } else {
        media_type
    };
    Ok((
        normalize_descriptor(media_type, parameters, default_charset)?,
        encoding,
    ))
}

fn normalize_descriptor<'a>(
    media_type: &str,
    parameters: impl Iterator<Item = &'a str>,
    default_charset: Option<&str>,
) -> Result<MediaDescriptor, String> {
    let Some((top_level, subtype)) = media_type.split_once('/') else {
        return Err("media type must have a type/subtype form".into());
    };
    if top_level.is_empty()
        || subtype.is_empty()
        || subtype.contains('/')
        || !top_level.bytes().all(is_token_byte)
        || !subtype.bytes().all(is_token_byte)
    {
        return Err("media type contains invalid token characters".into());
    }
    let mut display = format!(
        "{}/{}",
        top_level.to_ascii_lowercase(),
        subtype.to_ascii_lowercase()
    );
    let mut names = BTreeSet::new();
    let mut charset = None;
    for parameter in parameters {
        let Some((name, value)) = parameter.split_once('=') else {
            return Err("media parameters must use name=value syntax".into());
        };
        if name.is_empty()
            || value.is_empty()
            || !name.bytes().all(is_token_byte)
            || !value.bytes().all(is_token_byte)
        {
            return Err("media parameter contains invalid token characters".into());
        }
        let normalized_name = name.to_ascii_lowercase();
        if !names.insert(normalized_name.clone()) {
            return Err(format!("duplicate media parameter '{normalized_name}'"));
        }
        if normalized_name == "charset" {
            charset = Some(value.to_owned());
        }
        display.push(';');
        display.push_str(&normalized_name);
        display.push('=');
        display.push_str(value);
    }
    if charset.is_none()
        && let Some(default_charset) = default_charset
    {
        display.push_str(";charset=");
        display.push_str(default_charset);
        charset = Some(default_charset.into());
    }
    Ok(MediaDescriptor { display, charset })
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn validate_charset(descriptor: &MediaDescriptor, bytes: &[u8]) -> Result<(), String> {
    let Some(charset) = descriptor.charset.as_deref() else {
        return Ok(());
    };
    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
        return Ok(());
    }
    if charset.eq_ignore_ascii_case("us-ascii") || charset.eq_ignore_ascii_case("ascii") {
        if bytes.is_ascii() {
            return Ok(());
        }
        return Err("US-ASCII Data URI contains non-ASCII payload bytes".into());
    }
    Err(format!(
        "unsupported text charset '{charset}'; use UTF-8 or US-ASCII"
    ))
}

fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(*byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0x0F) as usize]));
        }
    }
    output
}

fn percent_decode(value: &str, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len().min(maximum_bytes));
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(format!("incomplete percent escape at payload byte {index}"));
            }
            let high = hex_value(bytes[index + 1])
                .ok_or_else(|| format!("invalid percent escape at payload byte {index}"))?;
            let low = hex_value(bytes[index + 2])
                .ok_or_else(|| format!("invalid percent escape at payload byte {index}"))?;
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
        if output.len() > maximum_bytes {
            return Err(format!("decoded payload exceeds {maximum_bytes} bytes"));
        }
    }
    Ok(output)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn data_uri_result(result_id: String, transformation: Transformation) -> SearchResult {
    let visible = safe_text(&compact(&transformation.value, MAX_VISIBLE_CHARACTERS));
    let title = compact(&safe_text(&transformation.value), 180);
    let direction = transformation.direction.label();
    let encoding = transformation.encoding.label();
    let descriptor = &transformation.descriptor.display;
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{direction} · {encoding} · {descriptor}"),
        preview: format!(
            "{direction}\nEncoding: {encoding}\nMedia type: {descriptor}\nInput: {} bytes\nOutput: {} bytes\n\nResult\n{visible}",
            transformation.input_bytes, transformation.output_bytes
        ),
        kind: ResultKind::Command,
        score: 3_420.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "doc.badge.gearshape".into()),
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
        id: "data-uri:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Data URI transformation unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!(
            "Data URI transformation unavailable\n{}",
            safe_text(message)
        ),
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
    fn parses_encode_modes_media_types_decode_and_nonmatching_queries() {
        assert!(parse_request("data science").unwrap().is_none());
        let default = parse_request("datauri :: hello").unwrap().unwrap();
        assert!(matches!(
            default,
            Request::Encode {
                encoding: Encoding::Percent,
                ..
            }
        ));
        let custom = parse_request("datauri base64 application/json :: {}")
            .unwrap()
            .unwrap();
        assert!(matches!(
            custom,
            Request::Encode {
                encoding: Encoding::Base64,
                ..
            }
        ));
        let decode = parse_request("数据URI解码: :: data:,hello")
            .unwrap()
            .unwrap();
        assert!(matches!(decode, Request::Decode { .. }));
    }

    #[test]
    fn normalizes_and_validates_media_descriptors() {
        let descriptor =
            parse_explicit_descriptor("Text/Plain;Charset=UTF-8;format=flowed").unwrap();
        assert_eq!(descriptor.display, "text/plain;charset=UTF-8;format=flowed");
        assert_eq!(descriptor.charset.as_deref(), Some("UTF-8"));
        assert!(parse_explicit_descriptor("text").is_err());
        assert!(parse_explicit_descriptor("text/plain;charset=UTF-8;charset=UTF-8").is_err());
        assert!(parse_explicit_descriptor("text/plain;name=hello world").is_err());
    }

    #[test]
    fn percent_encoding_round_trips_unicode_and_reserved_bytes() {
        let request = parse_request("datauri :: A + B, 東京 😀").unwrap().unwrap();
        let encoded = transform(request).unwrap();
        assert_eq!(
            encoded.value,
            "data:text/plain;charset=UTF-8,A%20%2B%20B%2C%20%E6%9D%B1%E4%BA%AC%20%F0%9F%98%80"
        );
        let decoded = decode_data_uri(&encoded.value).unwrap();
        assert_eq!(decoded.text, "A + B, 東京 😀");
        assert_eq!(decoded.encoding, Encoding::Percent);
    }

    #[test]
    fn base64_encoding_round_trips_text() {
        let request = parse_request("datauri base64 text/html;charset=UTF-8 :: <b>é</b>")
            .unwrap()
            .unwrap();
        let encoded = transform(request).unwrap();
        assert!(
            encoded
                .value
                .starts_with("data:text/html;charset=UTF-8;base64,")
        );
        let decoded = decode_data_uri(&encoded.value).unwrap();
        assert_eq!(decoded.text, "<b>é</b>");
        assert_eq!(decoded.encoding, Encoding::Base64);
    }

    #[test]
    fn decodes_default_descriptor_and_keeps_plus_literal() {
        let decoded = decode_data_uri("DATA:,hello+world%21").unwrap();
        assert_eq!(decoded.text, "hello+world!");
        assert_eq!(decoded.descriptor.display, "text/plain;charset=US-ASCII");
    }

    #[test]
    fn decodes_percent_escaped_base64_alphabet() {
        let decoded =
            decode_data_uri("data:text/plain;charset=UTF-8;base64,8J%2BYgA%3D%3D").unwrap();
        assert_eq!(decoded.text, "😀");
    }

    #[test]
    fn enforces_declared_charsets() {
        assert!(decode_data_uri("data:;charset=US-ASCII,%C3%A9").is_err());
        assert!(decode_data_uri("data:text/plain;charset=ISO-8859-1,%E9").is_err());
        assert!(parse_request("datauri text/plain;charset=US-ASCII :: é").is_err());
        assert!(parse_request("datauri text/plain;charset=UTF-8 :: é").is_ok());
    }

    #[test]
    fn rejects_malformed_binary_and_oversized_values() {
        assert!(decode_data_uri("https://example.com").is_err());
        assert!(decode_data_uri("data:text/plain,no-percent-%GG").is_err());
        assert!(decode_data_uri("data:text/plain;base64,%%%%").is_err());
        assert!(decode_data_uri("data:application/octet-stream;base64,/w==").is_err());
        assert!(parse_request(&format!("datauri :: {}", "x".repeat(MAX_TEXT_BYTES + 1))).is_err());
    }

    #[test]
    fn rejects_bad_command_shapes_and_metadata() {
        assert!(parse_request("datauri hello").is_err());
        assert!(parse_request("datauri percent text/plain extra :: hello").is_err());
        assert!(parse_request("datauridecode base64 :: data:,hello").is_err());
        assert!(parse_request("datauridecode ::").is_err());
        assert!(decode_data_uri("data:text/plain;base64;charset=UTF-8,SGk=").is_err());
    }

    #[test]
    fn results_are_actionable_and_clear_stale_values() {
        let mut plugin = DataUriPlugin::default();
        let result = plugin.search("datauri :: hello", 1).unwrap().remove(0);
        assert_eq!(result.id, "data-uri:encode-percent");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert_eq!(
            plugin.result_values[&result.id],
            "data:text/plain;charset=UTF-8,hello"
        );
        let invalid = plugin
            .search("datauridecode :: data:text/plain,%GG", 1)
            .unwrap()
            .remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.result_values.is_empty());
        assert!(plugin.search("datauri :: hello", 0).unwrap().is_empty());
    }

    #[test]
    fn decoded_controls_are_escaped_only_in_visible_fields() {
        let mut plugin = DataUriPlugin::default();
        let result = plugin
            .search(
                "datauridecode :: data:text/plain;charset=UTF-8,a%0Ab%E2%80%AE",
                1,
            )
            .unwrap()
            .remove(0);
        assert_eq!(plugin.result_values[&result.id], "a\nb\u{202e}");
        assert!(result.title.contains("\\u{000A}"));
        assert!(result.title.contains("\\u{202E}"));
        assert!(!result.title.contains('\u{202e}'));
    }
}
