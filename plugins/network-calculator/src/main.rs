use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

const PLUGIN_ID: &str = "io.clipkeep.network-calculator";
const IPV6_ADDRESS_SPACE: &str = "340282366920938463463374607431768211456";

fn main() -> io::Result<()> {
    run_plugin(NetworkCalculatorPlugin::default())
}

#[derive(Default)]
struct NetworkCalculatorPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for NetworkCalculatorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(network) = parse_request(query) else {
            return Ok(Vec::new());
        };
        Ok(calculate(network)
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, field)| {
                let result_id = format!("network-calculator:{}", field.id);
                self.recent_values
                    .insert(result_id.clone(), field.value.clone());
                field_result(
                    query.trim(),
                    field,
                    result_id,
                    index == 0,
                    3_410.0 - index as f32,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown network calculator action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "network calculation is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Network value copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkInput {
    V4 { address: u32, prefix: u8 },
    V6 { address: u128, prefix: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetworkField {
    id: &'static str,
    label: String,
    value: String,
    detail: String,
    symbol: &'static str,
}

fn parse_request(query: &str) -> Option<NetworkInput> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "cidr" | "subnet" | "network"
    ) && !matches!(command, "网段" | "子网")
    {
        return None;
    }
    let input = query[command_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace())
        .trim();
    if input.is_empty() {
        return None;
    }
    let (address, prefix) = if let Some((address, prefix)) = input.rsplit_once('/') {
        if address.contains('/') {
            return None;
        }
        (address.trim(), Some(prefix.trim().parse::<u8>().ok()?))
    } else {
        (input, None)
    };
    match IpAddr::from_str(address).ok()? {
        IpAddr::V4(address) => {
            let prefix = prefix.unwrap_or(32);
            (prefix <= 32).then_some(NetworkInput::V4 {
                address: u32::from(address),
                prefix,
            })
        }
        IpAddr::V6(address) => {
            let prefix = prefix.unwrap_or(128);
            (prefix <= 128).then_some(NetworkInput::V6 {
                address: u128::from(address),
                prefix,
            })
        }
    }
}

fn calculate(input: NetworkInput) -> Vec<NetworkField> {
    match input {
        NetworkInput::V4 { address, prefix } => calculate_v4(address, prefix),
        NetworkInput::V6 { address, prefix } => calculate_v6(address, prefix),
    }
}

fn calculate_v4(address: u32, prefix: u8) -> Vec<NetworkField> {
    let host_bits = 32 - u32::from(prefix);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << host_bits
    };
    let network = address & mask;
    let broadcast = network | !mask;
    let total_addresses = 1_u64 << host_bits;
    let (first_usable, last_usable, usable_addresses, range_detail) = match prefix {
        0..=30 => (
            network + 1,
            broadcast - 1,
            total_addresses - 2,
            "network and broadcast addresses excluded",
        ),
        31 => (network, broadcast, 2, "RFC 3021 point-to-point addresses"),
        32 => (network, network, 1, "single-host route"),
        _ => unreachable!("validated IPv4 prefix"),
    };
    let network_address = Ipv4Addr::from(network);
    let broadcast_address = Ipv4Addr::from(broadcast);
    let input_address = Ipv4Addr::from(address);
    let common_detail = format!("Input {input_address}/{prefix} · IPv4 · {host_bits} host bits");
    vec![
        NetworkField {
            id: "network",
            label: "Network".into(),
            value: format!("{network_address}/{prefix}"),
            detail: common_detail.clone(),
            symbol: "network",
        },
        NetworkField {
            id: "netmask",
            label: "Netmask".into(),
            value: Ipv4Addr::from(mask).to_string(),
            detail: format!("Prefix length /{prefix}"),
            symbol: "rectangle.split.3x3",
        },
        NetworkField {
            id: "wildcard",
            label: "Wildcard Mask".into(),
            value: Ipv4Addr::from(!mask).to_string(),
            detail: "Bitwise inverse of the subnet mask".into(),
            symbol: "square.split.diagonal.2x2",
        },
        NetworkField {
            id: "last",
            label: if prefix <= 30 {
                "Broadcast".into()
            } else {
                "Last Address".into()
            },
            value: broadcast_address.to_string(),
            detail: if prefix <= 30 {
                "IPv4 directed broadcast address".into()
            } else {
                "No dedicated broadcast address for this prefix".into()
            },
            symbol: "dot.radiowaves.left.and.right",
        },
        NetworkField {
            id: "range",
            label: "Usable Range".into(),
            value: format!(
                "{} – {}",
                Ipv4Addr::from(first_usable),
                Ipv4Addr::from(last_usable)
            ),
            detail: format!(
                "{} · {range_detail}",
                address_label(usable_addresses, "usable")
            ),
            symbol: "arrow.left.and.right",
        },
        NetworkField {
            id: "count",
            label: "Address Count".into(),
            value: total_addresses.to_string(),
            detail: format!(
                "{} · {}",
                address_label(total_addresses, "total"),
                address_label(usable_addresses, "usable")
            ),
            symbol: "number",
        },
    ]
}

fn address_label(count: u64, qualifier: &str) -> String {
    let noun = if count == 1 { "address" } else { "addresses" };
    format!("{count} {qualifier} {noun}")
}

fn calculate_v6(address: u128, prefix: u8) -> Vec<NetworkField> {
    let host_bits = 128 - u32::from(prefix);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << host_bits
    };
    let network = address & mask;
    let last = network | !mask;
    let address_count = if host_bits == 128 {
        IPV6_ADDRESS_SPACE.into()
    } else {
        (1_u128 << host_bits).to_string()
    };
    let network_address = Ipv6Addr::from(network);
    let last_address = Ipv6Addr::from(last);
    let input_address = Ipv6Addr::from(address);
    vec![
        NetworkField {
            id: "network",
            label: "Network".into(),
            value: format!("{network_address}/{prefix}"),
            detail: format!("Input {input_address}/{prefix} · IPv6 · {host_bits} host bits"),
            symbol: "network",
        },
        NetworkField {
            id: "last",
            label: "Last Address".into(),
            value: last_address.to_string(),
            detail: "Highest address in the prefix; IPv6 has no broadcast address".into(),
            symbol: "arrow.right.to.line",
        },
        NetworkField {
            id: "range",
            label: "Address Range".into(),
            value: format!("{network_address} – {last_address}"),
            detail: "Inclusive IPv6 address range".into(),
            symbol: "arrow.left.and.right",
        },
        NetworkField {
            id: "count",
            label: "Address Count".into(),
            value: address_count,
            detail: format!("2^{host_bits} addresses"),
            symbol: "number",
        },
        NetworkField {
            id: "host-bits",
            label: "Host Bits".into(),
            value: host_bits.to_string(),
            detail: format!("128 − {prefix} = {host_bits}"),
            symbol: "number.square",
        },
    ]
}

fn field_result(
    query: &str,
    field: NetworkField,
    result_id: String,
    primary: bool,
    score: f32,
) -> SearchResult {
    let title = if primary {
        query.into()
    } else {
        format!("{query} — {}", field.label)
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{} · {}", field.label, field.value),
        preview: format!("{}\n{}", field.value, field.detail),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([("symbol_name".into(), field.symbol.into())]),
        actions: vec![
            ResultAction {
                id: "paste".into(),
                title: format!("Paste {}", field.label),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                id: "copy".into(),
                title: format!("Copy {}", field.label),
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
    fn parses_ipv4_ipv6_defaults_and_aliases() {
        assert_eq!(
            parse_request("cidr 192.168.1.42/24"),
            Some(NetworkInput::V4 {
                address: u32::from(Ipv4Addr::new(192, 168, 1, 42)),
                prefix: 24,
            })
        );
        assert_eq!(
            parse_request("subnet:10.0.0.1"),
            Some(NetworkInput::V4 {
                address: u32::from(Ipv4Addr::new(10, 0, 0, 1)),
                prefix: 32,
            })
        );
        assert_eq!(
            parse_request("网段 2001:db8::1234/64"),
            Some(NetworkInput::V6 {
                address: u128::from(Ipv6Addr::from_str("2001:db8::1234").unwrap()),
                prefix: 64,
            })
        );
        assert_eq!(parse_request("cidr 192.168.1.1/33"), None);
        assert_eq!(parse_request("cidr 2001:db8::1/129"), None);
        assert_eq!(parse_request("ordinary text"), None);
    }

    #[test]
    fn calculates_ipv4_24_network_masks_range_and_counts() {
        let fields = calculate_v4(u32::from(Ipv4Addr::new(192, 168, 1, 42)), 24);
        assert_eq!(fields[0].value, "192.168.1.0/24");
        assert_eq!(fields[1].value, "255.255.255.0");
        assert_eq!(fields[2].value, "0.0.0.255");
        assert_eq!(fields[3].value, "192.168.1.255");
        assert_eq!(fields[4].value, "192.168.1.1 – 192.168.1.254");
        assert!(fields[4].detail.starts_with("254 usable addresses"));
        assert_eq!(fields[5].value, "256");
    }

    #[test]
    fn treats_ipv4_31_and_32_as_usable_endpoint_ranges() {
        let point_to_point = calculate_v4(u32::from(Ipv4Addr::new(10, 0, 0, 3)), 31);
        assert_eq!(point_to_point[0].value, "10.0.0.2/31");
        assert_eq!(point_to_point[3].label, "Last Address");
        assert_eq!(point_to_point[4].value, "10.0.0.2 – 10.0.0.3");
        assert!(point_to_point[4].detail.starts_with("2 usable addresses"));

        let host = calculate_v4(u32::from(Ipv4Addr::new(10, 0, 0, 3)), 32);
        assert_eq!(host[4].value, "10.0.0.3 – 10.0.0.3");
        assert!(host[4].detail.starts_with("1 usable address ·"));
    }

    #[test]
    fn calculates_ipv4_zero_prefix_without_shift_overflow() {
        let fields = calculate_v4(u32::from(Ipv4Addr::new(203, 0, 113, 9)), 0);
        assert_eq!(fields[0].value, "0.0.0.0/0");
        assert_eq!(fields[1].value, "0.0.0.0");
        assert_eq!(fields[3].value, "255.255.255.255");
        assert_eq!(fields[5].value, "4294967296");
    }

    #[test]
    fn calculates_ipv6_64_network_range_and_count() {
        let address = u128::from(Ipv6Addr::from_str("2001:db8::1234").unwrap());
        let fields = calculate_v6(address, 64);
        assert_eq!(fields[0].value, "2001:db8::/64");
        assert_eq!(fields[1].value, "2001:db8::ffff:ffff:ffff:ffff");
        assert_eq!(fields[3].value, "18446744073709551616");
        assert_eq!(fields[4].value, "64");
    }

    #[test]
    fn calculates_full_ipv6_address_space() {
        let fields = calculate_v6(u128::from(Ipv6Addr::LOCALHOST), 0);
        assert_eq!(fields[0].value, "::/0");
        assert_eq!(fields[1].value, "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff");
        assert_eq!(fields[3].value, IPV6_ADDRESS_SPACE);
        assert_eq!(fields[4].value, "128");
    }

    #[test]
    fn search_results_rank_exact_network_first_and_invalidate_old_values() {
        let mut plugin = NetworkCalculatorPlugin::default();
        let results = plugin.search("cidr 192.168.1.42/24", 20).unwrap();
        assert_eq!(results.len(), 6);
        assert_eq!(results[0].title, "cidr 192.168.1.42/24");
        assert_eq!(results[0].subtitle, "Network · 192.168.1.0/24");
        assert_eq!(results[1].title, "cidr 192.168.1.42/24 — Netmask");
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values.len(), 6);

        plugin.search("not a command", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }
}
