use clipkeep_plugin_sdk::{Plugin, SearchContext, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::IpAddr,
    process::Command,
    thread,
};

const PLUGIN_ID: &str = "io.clipkeep.dns-lookup";
const DIG_PATH: &str = "/usr/bin/dig";
const MAX_QUERY_BYTES: usize = 512;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_RECORDS: usize = 100;
const MAX_VISIBLE_FIELD_CHARACTERS: usize = 4_096;
const MAX_TITLE_CHARACTERS: usize = 200;

fn main() -> io::Result<()> {
    run_plugin(DnsLookupPlugin::default())
}

#[derive(Default)]
struct DnsLookupPlugin {
    action_values: BTreeMap<(String, String), String>,
}

impl Plugin for DnsLookupPlugin {
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
        let lookup = match run_lookup(&request, context) {
            Ok(lookup) => lookup,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let mut evaluation = build_results(&request, lookup);
        evaluation.results.truncate(limit);
        let active_ids: BTreeSet<String> = evaluation
            .results
            .iter()
            .map(|result| result.id.clone())
            .collect();
        evaluation
            .action_values
            .retain(|(result_id, _), _| active_ids.contains(result_id));
        self.action_values = evaluation.action_values;
        Ok(evaluation.results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let value = self
            .action_values
            .get(&(result_id.to_owned(), action_id.to_owned()))
            .ok_or_else(|| format!("unknown or inactive DNS Lookup action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "copy_name" => "DNS record name copied",
                _ => "DNS record value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum RecordType {
    A,
    Aaaa,
    Cname,
    Mx,
    Txt,
    Ns,
    Srv,
    Caa,
    Soa,
    Ptr,
}

impl RecordType {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_uppercase().as_str() {
            "A" => Some(Self::A),
            "AAAA" => Some(Self::Aaaa),
            "CNAME" => Some(Self::Cname),
            "MX" => Some(Self::Mx),
            "TXT" => Some(Self::Txt),
            "NS" => Some(Self::Ns),
            "SRV" => Some(Self::Srv),
            "CAA" => Some(Self::Caa),
            "SOA" => Some(Self::Soa),
            "PTR" => Some(Self::Ptr),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Cname => "CNAME",
            Self::Mx => "MX",
            Self::Txt => "TXT",
            Self::Ns => "NS",
            Self::Srv => "SRV",
            Self::Caa => "CAA",
            Self::Soa => "SOA",
            Self::Ptr => "PTR",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DnsRequest {
    target: String,
    record_types: Vec<RecordType>,
    reverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DnsRecord {
    name: String,
    ttl: u32,
    class: String,
    record_type: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DigResponse {
    requested_type: RecordType,
    status: String,
    server: Option<String>,
    query_time_ms: Option<u64>,
    records: Vec<DnsRecord>,
}

struct Evaluation {
    results: Vec<SearchResult>,
    action_values: BTreeMap<(String, String), String>,
}

fn parse_request(query: &str) -> Result<Option<DnsRequest>, String> {
    let query = query.trim();
    if query.len() > MAX_QUERY_BYTES {
        return Err("DNS query exceeds 512 bytes".into());
    }
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "dns" | "dig") && command != "域名" {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let arguments = remainder
        .strip_prefix(':')
        .unwrap_or(remainder)
        .trim_start();
    if arguments.is_empty() {
        return Err("expected a domain name, IP address, or DNS record type".into());
    }
    let tokens: Vec<&str> = arguments.split_whitespace().collect();
    if tokens.len() > 2 {
        return Err("expected 'dns <name>', 'dns <type> <name>', or 'dns reverse <ip>'".into());
    }
    if matches!(tokens[0].to_ascii_lowercase().as_str(), "reverse" | "rev") {
        if tokens.len() != 2 {
            return Err("reverse DNS requires exactly one IP address".into());
        }
        return reverse_request(tokens[1]).map(Some);
    }
    if let Some(record_type) = RecordType::parse(tokens[0]) {
        if tokens.len() != 2 {
            return Err(format!("{} lookup requires a target", record_type.label()));
        }
        if record_type == RecordType::Ptr {
            return reverse_request(tokens[1]).map(Some);
        }
        return forward_request(tokens[1], vec![record_type]).map(Some);
    }
    if tokens.len() == 2 {
        let record_type = RecordType::parse(tokens[1]).ok_or_else(|| {
            let unsupported = if tokens[0].contains('.') || tokens[0].parse::<IpAddr>().is_ok() {
                tokens[1]
            } else {
                tokens[0]
            };
            format!("unsupported DNS record type '{unsupported}'")
        })?;
        if record_type == RecordType::Ptr {
            return reverse_request(tokens[0]).map(Some);
        }
        return forward_request(tokens[0], vec![record_type]).map(Some);
    }
    if tokens[0].parse::<IpAddr>().is_ok() {
        reverse_request(tokens[0]).map(Some)
    } else {
        forward_request(tokens[0], vec![RecordType::A, RecordType::Aaaa]).map(Some)
    }
}

fn reverse_request(value: &str) -> Result<DnsRequest, String> {
    let address: IpAddr = value
        .parse()
        .map_err(|_| "reverse DNS target must be a valid IPv4 or IPv6 address".to_owned())?;
    Ok(DnsRequest {
        target: address.to_string(),
        record_types: vec![RecordType::Ptr],
        reverse: true,
    })
}

fn forward_request(value: &str, record_types: Vec<RecordType>) -> Result<DnsRequest, String> {
    Ok(DnsRequest {
        target: validate_domain(value)?,
        record_types,
        reverse: false,
    })
}

fn validate_domain(value: &str) -> Result<String, String> {
    let value = value.strip_suffix('.').unwrap_or(value);
    if value.is_empty() || value.len() > 253 {
        return Err("DNS name must contain 1–253 ASCII characters".into());
    }
    if !value.is_ascii() {
        return Err("internationalized DNS names must be entered in punycode form".into());
    }
    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err("each DNS label must contain 1–63 characters".into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("DNS labels cannot begin or end with '-'".into());
        }
        if !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("DNS names may contain only letters, digits, '-', '_', and '.'".into());
        }
    }
    Ok(value.to_ascii_lowercase())
}

fn run_lookup(request: &DnsRequest, context: &SearchContext) -> Result<Vec<DigResponse>, String> {
    if request.reverse {
        return Ok(vec![run_dig(
            &request.target,
            RecordType::Ptr,
            true,
            context,
        )?]);
    }
    thread::scope(|scope| {
        let handles: Vec<_> = request
            .record_types
            .iter()
            .copied()
            .map(|record_type| {
                let target = request.target.as_str();
                scope.spawn(move || run_dig(target, record_type, false, context))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "DNS lookup worker stopped unexpectedly".to_owned())?
            })
            .collect()
    })
}

fn dig_arguments(target: &str, record_type: RecordType, reverse: bool) -> Vec<String> {
    let mut arguments = if reverse {
        vec!["-x".into(), target.into()]
    } else {
        vec![target.into(), record_type.label().into()]
    };
    arguments.extend(
        [
            "+time=2",
            "+tries=1",
            "+noall",
            "+answer",
            "+comments",
            "+stats",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    arguments
}

fn run_dig(
    target: &str,
    record_type: RecordType,
    reverse: bool,
    context: &SearchContext,
) -> Result<DigResponse, String> {
    let arguments = dig_arguments(target, record_type, reverse);
    let output = clipkeep_plugin_sdk::command_output(
        Command::new(DIG_PATH).args(&arguments).env("LC_ALL", "C"),
        context,
        MAX_OUTPUT_BYTES,
    )
    .map_err(|error| format!("could not start the macOS DNS tool: {error}"))?;
    if output.stdout.len() > MAX_OUTPUT_BYTES || output.stderr.len() > MAX_OUTPUT_BYTES {
        return Err("DNS tool output exceeded 256 KiB".into());
    }
    if !output.status.success() {
        let detail = if output.stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            String::from_utf8_lossy(&output.stderr)
        };
        return Err(format!(
            "DNS lookup failed: {}",
            compact(&safe_text(detail.trim()), 240)
        ));
    }
    parse_dig_output(record_type, &String::from_utf8_lossy(&output.stdout))
}

fn parse_dig_output(record_type: RecordType, output: &str) -> Result<DigResponse, String> {
    let mut status = None;
    let mut server = None;
    let mut query_time_ms = None;
    let mut records = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(index) = line.find("status: ") {
            let tail = &line[index + "status: ".len()..];
            let value = tail
                .split(|character: char| character == ',' || character.is_whitespace())
                .next()
                .unwrap_or_default();
            if !value.is_empty() {
                status = Some(safe_text(value));
            }
            continue;
        }
        if let Some(value) = line.strip_prefix(";; SERVER: ") {
            server = Some(compact(&safe_text(value), 240));
            continue;
        }
        if let Some(value) = line
            .strip_prefix(";; Query time: ")
            .and_then(|value| value.strip_suffix(" msec"))
        {
            query_time_ms = value.parse().ok();
            continue;
        }
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let Ok(ttl) = fields[1].parse::<u32>() else {
            continue;
        };
        records.push(DnsRecord {
            name: compact(&safe_text(fields[0]), MAX_VISIBLE_FIELD_CHARACTERS),
            ttl,
            class: compact(&safe_text(fields[2]), 32),
            record_type: compact(&safe_text(fields[3]), 32),
            value: compact(
                &safe_text(&fields[4..].join(" ")),
                MAX_VISIBLE_FIELD_CHARACTERS,
            ),
        });
        if records.len() == MAX_RECORDS {
            break;
        }
    }
    records.sort_by(|left, right| {
        (&left.record_type, &left.value, &left.name, left.ttl).cmp(&(
            &right.record_type,
            &right.value,
            &right.name,
            right.ttl,
        ))
    });
    records.dedup_by(|left, right| {
        left.name == right.name
            && left.record_type == right.record_type
            && left.value == right.value
    });
    Ok(DigResponse {
        requested_type: record_type,
        status: status.unwrap_or_else(|| "UNKNOWN".into()),
        server,
        query_time_ms,
        records,
    })
}

fn build_results(request: &DnsRequest, responses: Vec<DigResponse>) -> Evaluation {
    let mut records = Vec::new();
    for (response_index, response) in responses.iter().enumerate() {
        for record in &response.records {
            records.push((response_index, record.clone()));
        }
    }
    records.sort_by(|(_, left), (_, right)| {
        (&left.record_type, &left.value, &left.name).cmp(&(
            &right.record_type,
            &right.value,
            &right.name,
        ))
    });
    records.dedup_by(|(_, left), (_, right)| {
        left.name == right.name
            && left.record_type == right.record_type
            && left.value == right.value
    });
    if records.is_empty() {
        return Evaluation {
            results: vec![empty_result(request, &responses)],
            action_values: BTreeMap::new(),
        };
    }
    let mut results = Vec::new();
    let mut action_values = BTreeMap::new();
    for (index, (response_index, record)) in records.into_iter().take(MAX_RECORDS).enumerate() {
        let response = &responses[response_index];
        let result_id = format!("dns:{}:{index}", record.record_type.to_ascii_lowercase());
        action_values.insert(
            (result_id.clone(), "paste_value".into()),
            record.value.clone(),
        );
        action_values.insert(
            (result_id.clone(), "copy_value".into()),
            record.value.clone(),
        );
        action_values.insert((result_id.clone(), "copy_name".into()), record.name.clone());
        let server = response.server.as_deref().unwrap_or("system resolver");
        let query_time = response
            .query_time_ms
            .map(|milliseconds| format!("{milliseconds} ms"))
            .unwrap_or_else(|| "not reported".into());
        results.push(SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: result_id,
            title: compact(
                &format!("{} · {}", record.record_type, record.value),
                MAX_TITLE_CHARACTERS,
            ),
            subtitle: format!(
                "{} · TTL {} · DNS via {}",
                record.name,
                format_ttl(record.ttl),
                compact(server, 100)
            ),
            preview: format!(
                "DNS {} record\nName: {}\nValue: {}\nClass: {}\nTTL: {} seconds\nStatus: {}\nResolver: {}\nQuery time: {}",
                record.record_type,
                record.name,
                record.value,
                record.class,
                record.ttl,
                response.status,
                server,
                query_time
            ),
            kind: ResultKind::Command,
            score: 3_430.0,
            metadata: BTreeMap::from([
                ("symbol_name".into(), "network.badge.shield.half.filled".into()),
                ("preview_style".into(), "text".into()),
            ]),
            actions: vec![
                action("paste_value", "Paste Record Value", true, true, true),
                action("copy_value", "Copy Record Value", false, false, false),
                action("copy_name", "Copy Record Name", false, false, false),
            ],
        });
    }
    Evaluation {
        results,
        action_values,
    }
}

fn empty_result(request: &DnsRequest, responses: &[DigResponse]) -> SearchResult {
    let statuses: Vec<String> = responses
        .iter()
        .map(|response| format!("{}: {}", response.requested_type.label(), response.status))
        .collect();
    let nonexistent = responses
        .iter()
        .any(|response| response.status == "NXDOMAIN");
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "dns:empty".into(),
        title: if nonexistent {
            "DNS name does not exist".into()
        } else {
            "No DNS records found".into()
        },
        subtitle: format!("{} · {}", request.target, statuses.join(" · ")),
        preview: format!(
            "No DNS records were returned.\nTarget: {}\n{}",
            request.target,
            statuses.join("\n")
        ),
        kind: ResultKind::Command,
        score: 3_430.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "questionmark.diamond".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "dns:invalid".into(),
        title: compact(query.trim(), MAX_TITLE_CHARACTERS),
        subtitle: format!("DNS lookup unavailable · {}", compact(message, 120)),
        preview: format!("DNS lookup unavailable\n{message}"),
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

fn format_ttl(seconds: u32) -> String {
    if seconds >= 86_400 && seconds.is_multiple_of(86_400) {
        format!("{}d", seconds / 86_400)
    } else if seconds >= 3_600 && seconds.is_multiple_of(3_600) {
        format!("{}h", seconds / 3_600)
    } else if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
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

    fn response(record_type: RecordType, status: &str, records: Vec<DnsRecord>) -> DigResponse {
        DigResponse {
            requested_type: record_type,
            status: status.into(),
            server: Some("192.0.2.53#53".into()),
            query_time_ms: Some(12),
            records,
        }
    }

    fn record(record_type: &str, value: &str) -> DnsRecord {
        DnsRecord {
            name: "example.com.".into(),
            ttl: 300,
            class: "IN".into(),
            record_type: record_type.into(),
            value: value.into(),
        }
    }

    #[test]
    fn parses_default_forward_and_automatic_reverse_queries() {
        assert_eq!(
            parse_request("dns Example.COM.").unwrap(),
            Some(DnsRequest {
                target: "example.com".into(),
                record_types: vec![RecordType::A, RecordType::Aaaa],
                reverse: false,
            })
        );
        assert_eq!(
            parse_request("dns 2001:4860:4860::8888").unwrap(),
            Some(DnsRequest {
                target: "2001:4860:4860::8888".into(),
                record_types: vec![RecordType::Ptr],
                reverse: true,
            })
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn parses_explicit_record_types_in_both_orders() {
        let leading = parse_request("DNS MX example.com").unwrap().unwrap();
        let trailing = parse_request("dig example.com txt").unwrap().unwrap();
        let reverse = parse_request("域名 reverse 1.1.1.1").unwrap().unwrap();
        assert_eq!(leading.record_types, vec![RecordType::Mx]);
        assert_eq!(trailing.record_types, vec![RecordType::Txt]);
        assert_eq!(reverse.record_types, vec![RecordType::Ptr]);
        assert!(reverse.reverse);
    }

    #[test]
    fn rejects_invalid_names_types_shapes_and_oversized_queries() {
        assert!(parse_request("dns bad name extra").is_err());
        assert!(parse_request("dns example.com HTTPS").is_err());
        assert_eq!(
            parse_request("dns HTTPS example.com").unwrap_err(),
            "unsupported DNS record type 'HTTPS'"
        );
        assert!(parse_request("dns -bad.example").is_err());
        assert!(parse_request("dns café.example").is_err());
        assert!(parse_request("dns reverse not-an-ip").is_err());
        assert!(parse_request(&format!("dns {}", "a".repeat(MAX_QUERY_BYTES))).is_err());
    }

    #[test]
    fn dig_arguments_keep_untrusted_targets_in_one_process_argument() {
        assert_eq!(
            dig_arguments("example.com", RecordType::Mx, false),
            vec![
                "example.com",
                "MX",
                "+time=2",
                "+tries=1",
                "+noall",
                "+answer",
                "+comments",
                "+stats"
            ]
        );
        assert_eq!(
            dig_arguments("1.1.1.1", RecordType::Ptr, true)[..2],
            ["-x", "1.1.1.1"]
        );
    }

    #[test]
    fn parses_answer_status_ttl_server_and_values_with_spaces() {
        let output = concat!(
            ";; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: 7\n",
            "example.com. 300 IN MX 10 mail.example.com.\n",
            "example.com. 60 IN TXT \"v=spf1 -all\"\n",
            ";; Query time: 12 msec\n",
            ";; SERVER: 192.0.2.53#53(192.0.2.53)\n"
        );
        let response = parse_dig_output(RecordType::Mx, output).unwrap();
        assert_eq!(response.status, "NOERROR");
        assert_eq!(response.query_time_ms, Some(12));
        assert_eq!(response.records.len(), 2);
        assert_eq!(response.records[0].value, "10 mail.example.com.");
        assert_eq!(response.records[1].value, "\"v=spf1 -all\"");
    }

    #[test]
    fn parses_nxdomain_without_fabricating_records() {
        let output = ";; ->>HEADER<<- opcode: QUERY, status: NXDOMAIN, id: 8\n";
        let response = parse_dig_output(RecordType::A, output).unwrap();
        assert_eq!(response.status, "NXDOMAIN");
        assert!(response.records.is_empty());
    }

    #[test]
    fn result_building_deduplicates_and_exposes_safe_actions() {
        let request =
            forward_request("example.com", vec![RecordType::A, RecordType::Aaaa]).unwrap();
        let duplicated = record("A", "192.0.2.1");
        let evaluation = build_results(
            &request,
            vec![
                response(RecordType::A, "NOERROR", vec![duplicated.clone()]),
                response(RecordType::Aaaa, "NOERROR", vec![duplicated]),
            ],
        );
        assert_eq!(evaluation.results.len(), 1);
        assert!(evaluation.results[0].actions[0].default);
        assert!(evaluation.results[0].actions[0].paste_to_previous);
        assert_eq!(evaluation.results[0].metadata["preview_style"], "text");
        assert_eq!(
            evaluation.action_values[&(evaluation.results[0].id.clone(), "copy_value".into())],
            "192.0.2.1"
        );
    }

    #[test]
    fn empty_and_nxdomain_results_are_visible_without_actions() {
        let request = forward_request("missing.example", vec![RecordType::A]).unwrap();
        let evaluation = build_results(
            &request,
            vec![response(RecordType::A, "NXDOMAIN", Vec::new())],
        );
        assert_eq!(evaluation.results[0].title, "DNS name does not exist");
        assert!(evaluation.results[0].actions.is_empty());
        assert!(evaluation.action_values.is_empty());
    }

    #[test]
    fn searches_honor_limits_and_invalidate_old_actions() {
        let mut plugin = DnsLookupPlugin::default();
        plugin
            .action_values
            .insert(("old".into(), "copy_value".into()), "stale".into());
        assert!(plugin.search("ordinary search", 20).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(plugin.search("dns example.com", 0).unwrap().is_empty());
    }

    #[test]
    fn display_helpers_are_bounded_and_escape_directional_controls() {
        assert_eq!(format_ttl(3_600), "1h");
        assert_eq!(format_ttl(61), "61s");
        assert_eq!(safe_text("safe\u{202e}value"), "safe\\u{202E}value");
        assert_eq!(compact("甲乙丙", 2), "甲乙…");
    }
}
