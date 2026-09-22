use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::collections::BTreeMap;
use std::io;
use std::process::Command;

const PLUGIN_ID: &str = "io.clipkeep.port-inspector";
const LSOF_PATH: &str = "/usr/sbin/lsof";
const MAX_BINDINGS: usize = 200;
const MAX_REPORT_BINDINGS: usize = 50;

fn main() -> io::Result<()> {
    run_plugin(PortInspectorPlugin::default())
}

#[derive(Default)]
struct PortInspectorPlugin {
    active_values: BTreeMap<(String, String), String>,
}

impl Plugin for PortInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let scan = match scan_bindings(request) {
            Ok(scan) => scan,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        if scan.bindings.is_empty() {
            return Ok(vec![empty_result(request)]);
        }
        let report = build_report(request, &scan);
        Ok(scan
            .bindings
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, binding)| {
                let result_id = format!("port-inspector:{index}:{}", binding.pid);
                for (action_id, value) in [
                    ("paste_endpoint", binding.endpoint.clone()),
                    ("copy_endpoint", binding.endpoint.clone()),
                    ("copy_pid", binding.pid.to_string()),
                    ("copy_process", binding.command.clone()),
                ] {
                    self.active_values
                        .insert((result_id.clone(), action_id.into()), value);
                }
                binding_result(result_id, binding, &report, index)
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let value = self
            .active_values
            .get(&(result_id.to_owned(), action_id.to_owned()))
            .ok_or_else(|| "port inspection result or action is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Port value copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Port {
        port: u16,
        protocol: Option<Protocol>,
    },
    AllTcp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    pid: u32,
    command: String,
    user: String,
    protocol: Protocol,
    endpoint: String,
    state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scan {
    bindings: Vec<Binding>,
    truncated: bool,
}

#[derive(Default)]
struct ProcessRecord {
    pid: Option<u32>,
    command: String,
    user: String,
}

#[derive(Default)]
struct FileRecord {
    protocol: Option<Protocol>,
    endpoint: String,
    state: Option<String>,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let remainder = query[command_end..]
        .strip_prefix(':')
        .unwrap_or(&query[command_end..])
        .trim();
    if command.eq_ignore_ascii_case("ports") {
        if !remainder.is_empty() {
            return Err("'ports' takes no arguments and lists visible TCP listeners".into());
        }
        return Ok(Some(Request::AllTcp));
    }
    if !command.eq_ignore_ascii_case("port") && command != "端口" {
        return Ok(None);
    }
    if remainder.is_empty() {
        return Err("expected a port number, such as 'port 3000' or 'port udp 5353'".into());
    }
    let tokens: Vec<&str> = remainder.split_whitespace().collect();
    let (port_text, protocol) = match tokens.as_slice() {
        [port] => (port.trim_start_matches(':'), None),
        [protocol, port] if parse_protocol(protocol).is_some() => {
            (port.trim_start_matches(':'), parse_protocol(protocol))
        }
        [port, protocol] if parse_protocol(protocol).is_some() => {
            (port.trim_start_matches(':'), parse_protocol(protocol))
        }
        _ => {
            return Err(
                "expected 'port <number>', 'port tcp <number>', or 'port udp <number>'".into(),
            );
        }
    };
    let port = port_text
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "port must be an integer from 1 through 65535".to_owned())?;
    Ok(Some(Request::Port { port, protocol }))
}

fn parse_protocol(value: &str) -> Option<Protocol> {
    if value.eq_ignore_ascii_case("tcp") {
        Some(Protocol::Tcp)
    } else if value.eq_ignore_ascii_case("udp") {
        Some(Protocol::Udp)
    } else {
        None
    }
}

fn scan_bindings(request: Request) -> Result<Scan, String> {
    let mut bindings = match request {
        Request::AllTcp => run_lsof(Protocol::Tcp, None)?,
        Request::Port {
            port,
            protocol: Some(protocol),
        } => run_lsof(protocol, Some(port))?,
        Request::Port {
            port,
            protocol: None,
        } => {
            let mut values = run_lsof(Protocol::Tcp, Some(port))?;
            values.extend(run_lsof(Protocol::Udp, Some(port))?);
            values
        }
    };
    sort_and_deduplicate(&mut bindings);
    let truncated = bindings.len() > MAX_BINDINGS;
    bindings.truncate(MAX_BINDINGS);
    Ok(Scan {
        bindings,
        truncated,
    })
}

fn run_lsof(protocol: Protocol, port: Option<u16>) -> Result<Vec<Binding>, String> {
    let selector = match (protocol, port) {
        (Protocol::Tcp, Some(port)) => format!("-iTCP:{port}"),
        (Protocol::Tcp, None) => "-iTCP".into(),
        (Protocol::Udp, Some(port)) => format!("-iUDP:{port}"),
        (Protocol::Udp, None) => "-iUDP".into(),
    };
    let mut command = Command::new(LSOF_PATH);
    command.args(["-nP", &selector]);
    if protocol == Protocol::Tcp {
        command.arg("-sTCP:LISTEN");
    }
    let output = command
        .arg("-FpcLunPT")
        .output()
        .map_err(|error| format!("could not run {LSOF_PATH}: {error}"))?;
    if !output.status.success() && output.stdout.is_empty() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if output.status.code() == Some(1) && message.is_empty() {
            return Ok(Vec::new());
        }
        return Err(if message.is_empty() {
            format!("lsof exited with status {}", output.status)
        } else {
            format!("lsof failed: {message}")
        });
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "lsof returned non-UTF-8 output".to_owned())?;
    Ok(parse_lsof_output(&text))
}

fn parse_lsof_output(output: &str) -> Vec<Binding> {
    let mut bindings = Vec::new();
    let mut process = ProcessRecord::default();
    let mut file = FileRecord::default();
    for line in output.lines() {
        let Some((field, value)) = line.split_at_checked(1) else {
            continue;
        };
        match field {
            "p" => {
                finish_file(&process, &mut file, &mut bindings);
                process = ProcessRecord {
                    pid: value.parse().ok(),
                    ..ProcessRecord::default()
                };
            }
            "c" => process.command = safe_text(value, 96),
            "L" => process.user = safe_text(value, 64),
            "u" if process.user.is_empty() => process.user = safe_text(value, 64),
            "f" => finish_file(&process, &mut file, &mut bindings),
            "P" => file.protocol = parse_protocol(value),
            "n" => file.endpoint = safe_text(value, 160),
            "T" if value.starts_with("ST=") => file.state = Some(safe_text(&value[3..], 32)),
            _ => {}
        }
    }
    finish_file(&process, &mut file, &mut bindings);
    bindings
}

fn finish_file(process: &ProcessRecord, file: &mut FileRecord, bindings: &mut Vec<Binding>) {
    if let (Some(pid), Some(protocol)) = (process.pid, file.protocol)
        && !file.endpoint.is_empty()
    {
        bindings.push(Binding {
            pid,
            command: if process.command.is_empty() {
                "Unknown process".into()
            } else {
                process.command.clone()
            },
            user: if process.user.is_empty() {
                "unknown user".into()
            } else {
                process.user.clone()
            },
            protocol,
            endpoint: file.endpoint.clone(),
            state: file.state.clone(),
        });
    }
    *file = FileRecord::default();
}

fn sort_and_deduplicate(bindings: &mut Vec<Binding>) {
    bindings.sort_by(|left, right| {
        endpoint_port(&left.endpoint)
            .cmp(&endpoint_port(&right.endpoint))
            .then_with(|| left.protocol.label().cmp(right.protocol.label()))
            .then_with(|| left.endpoint.cmp(&right.endpoint))
            .then_with(|| left.command.cmp(&right.command))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    bindings.dedup_by(|left, right| {
        left.pid == right.pid && left.protocol == right.protocol && left.endpoint == right.endpoint
    });
}

fn endpoint_port(endpoint: &str) -> u16 {
    endpoint
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(u16::MAX)
}

fn safe_text(value: &str, max_characters: usize) -> String {
    value
        .chars()
        .take(max_characters)
        .map(|character| {
            if character.is_control() || is_directional_or_invisible(character) {
                '�'
            } else {
                character
            }
        })
        .collect()
}

fn is_directional_or_invisible(character: char) -> bool {
    matches!(
        character as u32,
        0x00AD
            | 0x061C
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0xFEFF
    )
}

fn build_report(request: Request, scan: &Scan) -> String {
    let scope = request_label(request);
    let rows = scan
        .bindings
        .iter()
        .take(MAX_REPORT_BINDINGS)
        .map(|binding| {
            format!(
                "{} {} · {} · PID {} · {}{}",
                binding.protocol.label(),
                binding.endpoint,
                binding.command,
                binding.pid,
                binding.user,
                binding
                    .state
                    .as_deref()
                    .map(|state| format!(" · {state}"))
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut notes = Vec::new();
    if scan.bindings.len() > MAX_REPORT_BINDINGS {
        notes.push("Preview shows the first 50 bindings; use the result list for the remainder.");
    }
    if scan.truncated {
        notes.push("The scan is capped at 200 unique bindings.");
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", notes.join("\n"))
    };
    format!(
        "Port inspection\nScope: {scope}\nVisible unique bindings: {}\nResults reflect what lsof exposes to the current user.\n\n{rows}{notes}",
        scan.bindings.len()
    )
}

fn request_label(request: Request) -> String {
    match request {
        Request::AllTcp => "All TCP listeners".into(),
        Request::Port {
            port,
            protocol: Some(protocol),
        } => format!("{} port {port}", protocol.label()),
        Request::Port {
            port,
            protocol: None,
        } => format!("TCP listeners and UDP bindings on port {port}"),
    }
}

fn binding_result(result_id: String, binding: Binding, report: &str, index: usize) -> SearchResult {
    let status = binding.state.as_deref().unwrap_or("bound socket");
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: format!(
            "{} {} · {}",
            binding.protocol.label(),
            binding.endpoint,
            binding.command
        ),
        subtitle: format!("PID {} · {} · {status}", binding.pid, binding.user),
        preview: report.into(),
        kind: ResultKind::Command,
        score: 3_392.0 - index as f32,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "network".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste_endpoint", "Paste Endpoint", true, true),
            action("copy_endpoint", "Copy Endpoint", false, false),
            action("copy_pid", "Copy PID", false, false),
            action("copy_process", "Copy Process Name", false, false),
        ],
    }
}

fn empty_result(request: Request) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "port-inspector:empty".into(),
        title: match request {
            Request::AllTcp => "No visible TCP listeners".into(),
            Request::Port { port, .. } => format!("No visible bindings on port {port}"),
        },
        subtitle: format!("{} · read-only check", request_label(request)),
        preview: format!(
            "Port inspection\nScope: {}\nNo matching sockets were visible to the current user.",
            request_label(request)
        ),
        kind: ResultKind::Command,
        score: 3_392.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "network.slash".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "port-inspector:invalid".into(),
        title: "Port Inspector".into(),
        subtitle: format!("Port inspection failed · {message}"),
        preview: format!("Port inspection failed\n{message}"),
        kind: ResultKind::Command,
        score: 3_392.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn action(id: &str, title: &str, default: bool, paste_to_previous: bool) -> ResultAction {
    ResultAction {
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window: paste_to_previous,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(pid: u32, protocol: Protocol, endpoint: &str, command: &str) -> Binding {
        Binding {
            pid,
            command: command.into(),
            user: "alice".into(),
            protocol,
            endpoint: endpoint.into(),
            state: (protocol == Protocol::Tcp).then(|| "LISTEN".into()),
        }
    }

    #[test]
    fn parses_supported_queries_without_stealing_other_searches() {
        assert_eq!(
            parse_request("port 3000").unwrap(),
            Some(Request::Port {
                port: 3000,
                protocol: None
            })
        );
        assert_eq!(
            parse_request("port udp 5353").unwrap(),
            Some(Request::Port {
                port: 5353,
                protocol: Some(Protocol::Udp)
            })
        );
        assert_eq!(
            parse_request("端口:443 tcp").unwrap(),
            Some(Request::Port {
                port: 443,
                protocol: Some(Protocol::Tcp)
            })
        );
        assert_eq!(parse_request("ports").unwrap(), Some(Request::AllTcp));
        assert_eq!(parse_request("url https://example.com:80").unwrap(), None);
    }

    #[test]
    fn rejects_invalid_ports_and_query_shapes() {
        assert!(parse_request("port").is_err());
        assert!(parse_request("port 0").is_err());
        assert!(parse_request("port 65536").is_err());
        assert!(parse_request("port sctp 80").is_err());
        assert!(parse_request("ports 80").is_err());
    }

    #[test]
    fn parses_process_and_file_records_and_deduplicates_fds() {
        let output = "p686\ncControlCenter\nu501\nLalice\nf13\nPTCP\nn*:5000\nTST=LISTEN\nf14\nPTCP\nn*:5000\nTST=LISTEN\np842\ncpython\nLalice\nf4\nPUDP\nn127.0.0.1:5353\n";
        let mut values = parse_lsof_output(output);
        assert_eq!(values.len(), 3);
        sort_and_deduplicate(&mut values);
        assert_eq!(values.len(), 2);
        assert_eq!(
            values[0],
            binding(686, Protocol::Tcp, "*:5000", "ControlCenter")
        );
        assert_eq!(values[1].protocol, Protocol::Udp);
        assert_eq!(values[1].endpoint, "127.0.0.1:5353");
    }

    #[test]
    fn incomplete_file_records_are_ignored() {
        let values = parse_lsof_output("p42\nctest\nf1\nPTCP\nf2\nn*:9000\n");
        assert!(values.is_empty());
    }

    #[test]
    fn sorting_uses_numeric_ports_then_protocol_and_process() {
        let mut values = vec![
            binding(2, Protocol::Tcp, "*:10000", "z"),
            binding(3, Protocol::Udp, "*:80", "u"),
            binding(1, Protocol::Tcp, "127.0.0.1:80", "a"),
        ];
        sort_and_deduplicate(&mut values);
        assert_eq!(values[0].endpoint, "127.0.0.1:80");
        assert_eq!(values[1].endpoint, "*:80");
        assert_eq!(values[2].endpoint, "*:10000");
    }

    #[test]
    fn extracts_ipv4_ipv6_and_wildcard_ports() {
        assert_eq!(endpoint_port("127.0.0.1:3000"), 3000);
        assert_eq!(endpoint_port("[::1]:443"), 443);
        assert_eq!(endpoint_port("*:5353"), 5353);
        assert_eq!(endpoint_port("malformed"), u16::MAX);
    }

    #[test]
    fn sanitizes_control_directional_and_overlong_text() {
        assert_eq!(safe_text("ab\n\u{202E}cd", 10), "ab��cd");
        assert_eq!(safe_text("abcdef", 3), "abc");
    }

    #[test]
    fn result_actions_are_safe_and_copyable() {
        let value = binding(42, Protocol::Tcp, "127.0.0.1:8080", "server");
        let result = binding_result("port-inspector:0:42".into(), value, "report", 0);
        assert_eq!(result.actions.len(), 4);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert!(result.actions.iter().all(|action| !action.destructive));
    }

    #[test]
    fn empty_result_is_visible_without_actions() {
        let result = empty_result(Request::Port {
            port: 65432,
            protocol: None,
        });
        assert!(result.title.contains("65432"));
        assert!(result.actions.is_empty());
    }

    #[test]
    fn report_preview_is_bounded_independently_from_result_count() {
        let bindings = (1..=51)
            .map(|port| binding(port, Protocol::Tcp, &format!("*:{port}"), "server"))
            .collect();
        let report = build_report(
            Request::AllTcp,
            &Scan {
                bindings,
                truncated: false,
            },
        );
        assert!(report.contains("Visible unique bindings: 51"));
        assert!(report.contains("Preview shows the first 50"));
        assert!(!report.contains("TCP *:51 ·"));
    }
}
