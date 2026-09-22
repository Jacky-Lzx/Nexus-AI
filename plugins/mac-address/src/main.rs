use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.mac-address";
const RESULT_ID: &str = "mac-address:result";
const MAX_QUERY_BYTES: usize = 1024;

fn main() -> io::Result<()> {
    run_plugin(MacAddressPlugin::default())
}

#[derive(Default)]
struct MacAddressPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for MacAddressPlugin {
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
        let (address, generated) = match request {
            Request::Inspect(address) => (address, false),
            Request::Random => match random_local_unicast() {
                Ok(address) => (address, true),
                Err(message) => return Ok(vec![invalid_result(query, &message)]),
            },
        };
        let inspection = inspect(address, generated);
        self.action_values
            .insert("paste_address".into(), inspection.normalized.clone());
        self.action_values
            .insert("copy_address".into(), inspection.normalized.clone());
        self.action_values
            .insert("copy_compact".into(), inspection.compact.clone());
        if let Some(eui64) = &inspection.modified_eui64 {
            self.action_values
                .insert("copy_eui64".into(), eui64.clone());
        }
        self.action_values
            .insert("copy_report".into(), inspection.report.clone());
        Ok(vec![inspection_result(&inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("MAC address result is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive MAC Address action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "copy_report" => "MAC address report copied",
                "copy_eui64" => "Modified EUI-64 copied",
                _ => "MAC address copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MacAddress([u8; 6]);

impl MacAddress {
    fn parse(input: &str) -> Result<Self, String> {
        let compact = if input.contains(':') || input.contains('-') {
            if input.contains(':') && input.contains('-') {
                return Err("MAC address cannot mix ':' and '-' separators".into());
            }
            let separator = if input.contains(':') { ':' } else { '-' };
            let parts = input.split(separator).collect::<Vec<_>>();
            if parts.len() != 6 || parts.iter().any(|part| part.len() != 2) {
                return Err("separated MAC address must contain six two-digit octets".into());
            }
            parts.concat()
        } else if input.contains('.') {
            let parts = input.split('.').collect::<Vec<_>>();
            if parts.len() != 3 || parts.iter().any(|part| part.len() != 4) {
                return Err("Cisco-style MAC address must contain three four-digit groups".into());
            }
            parts.concat()
        } else {
            input.into()
        };
        if compact.len() != 12 {
            return Err("MAC-48 address must contain exactly 12 hexadecimal digits".into());
        }
        if !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("MAC address contains a non-hexadecimal digit".into());
        }
        let mut bytes = [0_u8; 6];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
                .map_err(|_| "MAC address contains an invalid octet".to_owned())?;
        }
        Ok(Self(bytes))
    }

    fn normalized(self) -> String {
        self.0
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    fn hyphenated(self) -> String {
        self.normalized().replace(':', "-")
    }

    fn compact(self) -> String {
        self.0
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join("")
    }

    fn cisco(self) -> String {
        let compact = self.compact().to_ascii_lowercase();
        format!("{}.{}.{}", &compact[..4], &compact[4..8], &compact[8..])
    }

    fn is_broadcast(self) -> bool {
        self.0 == [0xff; 6]
    }

    fn is_unspecified(self) -> bool {
        self.0 == [0; 6]
    }

    fn is_multicast(self) -> bool {
        self.0[0] & 0x01 != 0
    }

    fn is_locally_administered(self) -> bool {
        self.0[0] & 0x02 != 0
    }

    fn address_type(self) -> &'static str {
        if self.is_broadcast() {
            "Broadcast"
        } else if self.is_unspecified() {
            "Unspecified"
        } else if self.is_multicast() {
            "Multicast"
        } else {
            "Unicast"
        }
    }

    fn administration(self) -> &'static str {
        if self.is_broadcast() || self.is_unspecified() {
            "Not applicable"
        } else if self.is_locally_administered() {
            "Locally administered"
        } else {
            "Universally administered"
        }
    }

    fn first_24_bits(self) -> String {
        format!("{:02X}:{:02X}:{:02X}", self.0[0], self.0[1], self.0[2])
    }

    fn modified_eui64_bytes(self) -> Option<[u8; 8]> {
        if self.is_multicast() || self.is_broadcast() || self.is_unspecified() {
            return None;
        }
        Some([
            self.0[0] ^ 0x02,
            self.0[1],
            self.0[2],
            0xff,
            0xfe,
            self.0[3],
            self.0[4],
            self.0[5],
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Request {
    Inspect(MacAddress),
    Random,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "macaddr" | "macaddress"
    ) && command != "mac地址"
    {
        return Ok(None);
    }
    if query.len() > MAX_QUERY_BYTES {
        return Err("MAC Address query exceeds 1 KiB".into());
    }
    let remainder = &query[command_end..];
    let input = remainder.strip_prefix(':').unwrap_or(remainder).trim();
    if input.is_empty() {
        return Err("expected a MAC-48 address or 'random'".into());
    }
    if input.eq_ignore_ascii_case("random") || input == "随机" {
        return Ok(Some(Request::Random));
    }
    if input.split_whitespace().count() != 1 {
        return Err("MAC address cannot contain whitespace".into());
    }
    Ok(Some(Request::Inspect(MacAddress::parse(input)?)))
}

fn random_local_unicast() -> Result<MacAddress, String> {
    let mut bytes = [0_u8; 6];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("secure random generation failed: {error}"))?;
    Ok(local_unicast_from_bytes(bytes))
}

fn local_unicast_from_bytes(mut bytes: [u8; 6]) -> MacAddress {
    bytes[0] = (bytes[0] & 0xfc) | 0x02;
    MacAddress(bytes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inspection {
    normalized: String,
    compact: String,
    address_type: String,
    administration: String,
    modified_eui64: Option<String>,
    generated: bool,
    report: String,
}

fn inspect(address: MacAddress, generated: bool) -> Inspection {
    let normalized = address.normalized();
    let compact = address.compact();
    let modified_bytes = address.modified_eui64_bytes();
    let modified_eui64 = modified_bytes.map(format_eui64);
    let link_local = modified_bytes.map(format_link_local);
    let individual_group_bit = if address.is_multicast() {
        "1 (group)"
    } else {
        "0 (individual)"
    };
    let universal_local_bit = if address.is_broadcast() {
        "1 (set; not an administration claim for broadcast)"
    } else if address.is_unspecified() {
        "0 (clear; not an administration claim for unspecified)"
    } else if address.is_locally_administered() {
        "1 (local)"
    } else {
        "0 (universal)"
    };
    let mut lines = vec![
        "MAC-48 address inspection".into(),
        format!("Normalized: {normalized}"),
        format!("Compact: {compact}"),
        format!("Hyphenated: {}", address.hyphenated()),
        format!("Cisco dotted: {}", address.cisco()),
        format!("Address type: {}", address.address_type()),
        format!("Administration: {}", address.administration()),
        format!("I/G bit: {individual_group_bit}"),
        format!("U/L bit: {universal_local_bit}"),
        format!("First 24 bits: {}", address.first_24_bits()),
        "Vendor: not inferred without an OUI database".into(),
    ];
    if let Some(value) = &modified_eui64 {
        lines.push(format!("Modified EUI-64: {value}"));
    } else {
        lines.push(
            "Modified EUI-64: not derived for multicast, broadcast, or unspecified addresses"
                .into(),
        );
    }
    if let Some(value) = link_local {
        lines.push(format!("IPv6 link-local form: {value}"));
    }
    if generated {
        lines.push("Generation: system-secure random local unicast address".into());
    }
    Inspection {
        normalized,
        compact,
        address_type: address.address_type().into(),
        administration: address.administration().into(),
        modified_eui64,
        generated,
        report: lines.join("\n"),
    }
}

fn format_eui64(bytes: [u8; 8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn format_link_local(bytes: [u8; 8]) -> String {
    let groups = [
        u16::from_be_bytes([bytes[0], bytes[1]]),
        u16::from_be_bytes([bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
    ];
    format!(
        "fe80::{:x}:{:x}:{:x}:{:x}",
        groups[0], groups[1], groups[2], groups[3]
    )
}

fn inspection_result(inspection: &Inspection) -> SearchResult {
    let mut actions = vec![
        action("paste_address", "Paste MAC Address", true, true, true),
        action("copy_address", "Copy MAC Address", false, false, false),
        action("copy_compact", "Copy Compact Address", false, false, false),
    ];
    if inspection.modified_eui64.is_some() {
        actions.push(action(
            "copy_eui64",
            "Copy Modified EUI-64",
            false,
            false,
            false,
        ));
    }
    actions.push(action(
        "copy_report",
        "Copy MAC Report",
        false,
        false,
        false,
    ));
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: inspection.normalized.clone(),
        subtitle: format!(
            "{} · {}{}",
            inspection.address_type,
            inspection.administration,
            if inspection.generated {
                " · secure random"
            } else {
                ""
            }
        ),
        preview: inspection.report.clone(),
        kind: ResultKind::Command,
        score: 3_440.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "network".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "mac-address:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "MAC Address unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("MAC Address unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_440.0,
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
    fn parses_and_normalizes_common_mac48_notations() {
        let expected = MacAddress([0, 0x11, 0x22, 0x33, 0x44, 0x55]);
        for input in [
            "00:11:22:33:44:55",
            "00-11-22-33-44-55",
            "0011.2233.4455",
            "001122334455",
        ] {
            assert_eq!(MacAddress::parse(input).unwrap(), expected);
        }
        assert_eq!(expected.normalized(), "00:11:22:33:44:55");
        assert_eq!(expected.hyphenated(), "00-11-22-33-44-55");
        assert_eq!(expected.cisco(), "0011.2233.4455");
    }

    #[test]
    fn rejects_malformed_mixed_and_non_mac48_values() {
        for input in [
            "",
            "00:11:22:33:44",
            "00:11-22:33:44:55",
            "0:11:22:33:44:55",
            "0011.2233.445",
            "00112233445Z",
            "00112233445566",
        ] {
            assert!(MacAddress::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn classifies_individual_group_and_universal_local_bits() {
        let universal = MacAddress::parse("00:11:22:33:44:55").unwrap();
        assert_eq!(universal.address_type(), "Unicast");
        assert_eq!(universal.administration(), "Universally administered");
        let local = MacAddress::parse("02:11:22:33:44:55").unwrap();
        assert_eq!(local.address_type(), "Unicast");
        assert_eq!(local.administration(), "Locally administered");
        let multicast = MacAddress::parse("01:00:5E:00:00:01").unwrap();
        assert_eq!(multicast.address_type(), "Multicast");
    }

    #[test]
    fn identifies_broadcast_and_unspecified_special_addresses() {
        let broadcast = MacAddress::parse("FF:FF:FF:FF:FF:FF").unwrap();
        assert_eq!(broadcast.address_type(), "Broadcast");
        assert_eq!(broadcast.administration(), "Not applicable");
        assert_eq!(broadcast.modified_eui64_bytes(), None);
        let zero = MacAddress::parse("00:00:00:00:00:00").unwrap();
        assert_eq!(zero.address_type(), "Unspecified");
        assert_eq!(zero.modified_eui64_bytes(), None);
    }

    #[test]
    fn derives_modified_eui64_and_link_local_form() {
        let address = MacAddress::parse("00:11:22:33:44:55").unwrap();
        let bytes = address.modified_eui64_bytes().unwrap();
        assert_eq!(format_eui64(bytes), "02:11:22:FF:FE:33:44:55");
        assert_eq!(format_link_local(bytes), "fe80::211:22ff:fe33:4455");
        let local = MacAddress::parse("02:11:22:33:44:55").unwrap();
        assert_eq!(local.modified_eui64_bytes().unwrap()[0], 0x00);
    }

    #[test]
    fn deterministic_random_mapping_always_sets_local_unicast_bits() {
        for first in 0_u8..=u8::MAX {
            let address = local_unicast_from_bytes([first, 1, 2, 3, 4, 5]);
            assert!(!address.is_multicast());
            assert!(address.is_locally_administered());
            assert_eq!(address.0[1..], [1, 2, 3, 4, 5]);
        }
    }

    #[test]
    fn secure_random_generation_produces_a_local_unicast_address() {
        let address = random_local_unicast().unwrap();
        assert_eq!(address.address_type(), "Unicast");
        assert_eq!(address.administration(), "Locally administered");
        assert_ne!(address, MacAddress([0; 6]));
    }

    #[test]
    fn parses_only_explicit_bounded_commands() {
        assert!(parse_request("mac 00:11:22:33:44:55").unwrap().is_none());
        assert!(matches!(
            parse_request("macaddr: 00:11:22:33:44:55").unwrap(),
            Some(Request::Inspect(_))
        ));
        assert_eq!(
            parse_request("mac地址 随机").unwrap(),
            Some(Request::Random)
        );
        assert!(parse_request("macaddr").is_err());
        assert!(parse_request(&format!("macaddr {}", "0".repeat(MAX_QUERY_BYTES))).is_err());
    }

    #[test]
    fn reports_fields_without_claiming_a_vendor() {
        let inspection = inspect(MacAddress::parse("00:11:22:33:44:55").unwrap(), false);
        assert!(inspection.report.contains("First 24 bits: 00:11:22"));
        assert!(inspection.report.contains("Vendor: not inferred"));
        assert!(inspection.report.contains("I/G bit: 0 (individual)"));
        assert!(inspection.report.contains("U/L bit: 0 (universal)"));
        let broadcast = inspect(MacAddress::parse("FF:FF:FF:FF:FF:FF").unwrap(), false);
        assert!(
            broadcast
                .report
                .contains("not an administration claim for broadcast")
        );
    }

    #[test]
    fn results_are_actionable_safe_and_invalidate_stale_values() {
        let mut plugin = MacAddressPlugin::default();
        let result = plugin
            .search("macaddr 00:11:22:33:44:55", 1)
            .unwrap()
            .remove(0);
        assert_eq!(result.id, RESULT_ID);
        assert_eq!(result.actions.len(), 5);
        assert!(result.actions[0].default);
        assert_eq!(plugin.action_values["copy_address"], "00:11:22:33:44:55");
        let invalid = plugin.search("macaddr nope", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        let unsafe_result = plugin
            .search("macaddr 00112233445\u{202E}", 1)
            .unwrap()
            .remove(0);
        assert!(!unsafe_result.title.contains('\u{202E}'));
        assert!(unsafe_result.title.contains("\\u{202E}"));
        assert!(plugin.search("macaddr random", 0).unwrap().is_empty());
    }
}
