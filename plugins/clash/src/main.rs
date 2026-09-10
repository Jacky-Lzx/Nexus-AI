use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{
    PluginPresentationItem, PresentationStyle, ResultAction, ResultKind, SearchResult,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

const PLUGIN_ID: &str = "io.clipkeep.clash";
const DEFAULT_CONTROLLER: &str = "http://127.0.0.1:9090";

fn main() -> io::Result<()> {
    run_plugin(ClashPlugin::default())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClashMode {
    Rule,
    Global,
    Direct,
}

impl ClashMode {
    const ALL: [Self; 3] = [Self::Rule, Self::Global, Self::Direct];

    fn id(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Rule => "Rule Mode",
            Self::Global => "Global Mode",
            Self::Direct => "Direct Mode",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Rule => "Route connections according to Clash rules",
            Self::Global => "Route all connections through the selected global proxy",
            Self::Direct => "Bypass proxies and connect directly",
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Self::Rule => "rule rules 规则 模式",
            Self::Global => "global proxy 全局 代理 模式",
            Self::Direct => "direct bypass 直连 绕过 模式",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Rule => "list.bullet.rectangle",
            Self::Global => "globe",
            Self::Direct => "arrow.left.arrow.right",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "rule" => Some(Self::Rule),
            "global" => Some(Self::Global),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }
}

struct ClashPlugin {
    controller: String,
    secret: String,
    current_mode: Option<ClashMode>,
    last_error: Option<String>,
}

impl Default for ClashPlugin {
    fn default() -> Self {
        Self {
            controller: DEFAULT_CONTROLLER.into(),
            secret: String::new(),
            current_mode: None,
            last_error: None,
        }
    }
}

impl ClashPlugin {
    fn refresh_mode(&mut self) {
        match ControllerClient::new(&self.controller, &self.secret).and_then(|client| client.mode())
        {
            Ok(mode) => {
                self.current_mode = Some(mode);
                self.last_error = None;
            }
            Err(error) => {
                self.current_mode = None;
                self.last_error = Some(error);
            }
        }
    }

    fn set_mode(&mut self, mode: ClashMode) -> Result<Option<String>, String> {
        ControllerClient::new(&self.controller, &self.secret)?.set_mode(mode)?;
        self.current_mode = Some(mode);
        self.last_error = None;
        Ok(Some(format!("Clash mode changed to {}", mode.title())))
    }
}

impl Plugin for ClashPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.refresh_mode();
        let query = query.trim().to_lowercase();
        Ok(ClashMode::ALL
            .into_iter()
            .filter(|mode| matches_query(*mode, &query))
            .take(limit)
            .map(|mode| mode_result(mode, self.current_mode))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id != "set" {
            return Err(format!("unknown Clash action: {action_id}"));
        }
        let mode = result_id
            .strip_prefix("clash:mode:")
            .and_then(ClashMode::parse)
            .ok_or_else(|| "invalid Clash mode result".to_owned())?;
        self.set_mode(mode)
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let controller = settings
            .get("controller_url")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CONTROLLER)
            .trim_end_matches('/')
            .to_owned();
        let secret = settings.get("secret").cloned().unwrap_or_default();
        let changed = controller != self.controller || secret != self.secret;
        self.controller = controller;
        self.secret = secret;
        if changed {
            self.current_mode = None;
            self.last_error = None;
        }
        Ok(changed)
    }

    fn presentation_items(&self) -> Vec<PluginPresentationItem> {
        let (value, style, symbol) = if let Some(mode) = self.current_mode {
            (
                mode.title().to_owned(),
                PresentationStyle::Success,
                mode.symbol(),
            )
        } else {
            (
                self.last_error
                    .as_deref()
                    .unwrap_or("Clash controller unavailable")
                    .to_owned(),
                PresentationStyle::Warning,
                "exclamationmark.triangle",
            )
        };
        vec![PluginPresentationItem {
            id: "clash-status".into(),
            title: "Current Mode".into(),
            value,
            style,
            metadata: BTreeMap::from([("symbol_name".into(), symbol.into())]),
        }]
    }
}

fn matches_query(mode: ClashMode, query: &str) -> bool {
    query.is_empty()
        || query.split_whitespace().all(|word| {
            mode.title().to_lowercase().contains(word)
                || mode.keywords().to_lowercase().contains(word)
        })
}

fn mode_result(mode: ClashMode, current: Option<ClashMode>) -> SearchResult {
    let selected = current == Some(mode);
    let subtitle = if selected {
        format!("Current · {}", mode.subtitle())
    } else {
        mode.subtitle().into()
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: format!("clash:mode:{}", mode.id()),
        title: mode.title().into(),
        subtitle: subtitle.clone(),
        preview: subtitle,
        kind: ResultKind::Command,
        score: if selected { 3_100.0 } else { 3_000.0 },
        metadata: BTreeMap::from([
            ("symbol_name".into(), mode.symbol().into()),
            ("mode".into(), mode.id().into()),
            ("current".into(), selected.to_string()),
        ]),
        actions: vec![ResultAction {
            id: "set".into(),
            title: format!("Switch to {}", mode.title()),
            destructive: false,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

struct ControllerClient {
    address: String,
    host_header: String,
    base_path: String,
    secret: String,
}

impl ControllerClient {
    fn new(controller: &str, secret: &str) -> Result<Self, String> {
        if controller.contains(['\r', '\n']) || secret.contains(['\r', '\n']) {
            return Err("Clash controller settings contain invalid characters".into());
        }
        let rest = controller
            .strip_prefix("http://")
            .ok_or("Controller URL must use http://")?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            return Err("Controller URL is missing a host".into());
        }
        let address = if authority.starts_with('[') {
            if authority.contains("]:") {
                authority.to_owned()
            } else {
                format!("{authority}:80")
            }
        } else if authority.rsplit_once(':').is_some() {
            authority.to_owned()
        } else {
            format!("{authority}:80")
        };
        Ok(Self {
            address,
            host_header: authority.into(),
            base_path: path.trim_matches('/').into(),
            secret: secret.into(),
        })
    }

    fn mode(&self) -> Result<ClashMode, String> {
        let response = self.request("GET", None)?;
        let json: Value = serde_json::from_slice(&response)
            .map_err(|error| format!("Clash returned invalid configuration JSON: {error}"))?;
        json.get("mode")
            .and_then(Value::as_str)
            .and_then(ClashMode::parse)
            .ok_or_else(|| "Clash did not return a supported mode".into())
    }

    fn set_mode(&self, mode: ClashMode) -> Result<(), String> {
        let body = format!(r#"{{"mode":"{}"}}"#, mode.id());
        self.request("PATCH", Some(body.as_bytes())).map(|_| ())
    }

    fn request(&self, method: &str, body: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut addresses = self
            .address
            .to_socket_addrs()
            .map_err(|error| format!("Could not resolve Clash controller: {error}"))?;
        let address = addresses
            .next()
            .ok_or("Could not resolve Clash controller")?;
        let timeout = Duration::from_millis(800);
        let mut stream = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("Could not connect to Clash: {error}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("Could not configure Clash connection: {error}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| format!("Could not configure Clash connection: {error}"))?;

        let path = if self.base_path.is_empty() {
            "/configs".to_owned()
        } else {
            format!("/{}/configs", self.base_path)
        };
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.host_header
        );
        if !self.secret.is_empty() {
            request.push_str(&format!("Authorization: Bearer {}\r\n", self.secret));
        }
        if let Some(body) = body {
            request.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        request.push_str("\r\n");
        stream
            .write_all(request.as_bytes())
            .and_then(|_| body.map_or(Ok(()), |body| stream.write_all(body)))
            .map_err(|error| format!("Could not send request to Clash: {error}"))?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|error| format!("Could not read response from Clash: {error}"))?;
        parse_http_response(&response)
    }
}

fn parse_http_response(response: &[u8]) -> Result<Vec<u8>, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("Clash returned an invalid HTTP response")?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "Clash returned invalid HTTP headers")?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("Clash returned an invalid HTTP status")?;
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => "Clash rejected the configured Secret".into(),
            _ => format!("Clash controller returned HTTP {status}"),
        });
    }
    let body = &response[header_end + 4..];
    if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        decode_chunked(body)
    } else {
        Ok(body.to_vec())
    }
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("Clash returned an invalid chunked response")?;
        let size_text = std::str::from_utf8(&input[..line_end])
            .map_err(|_| "Clash returned an invalid chunk size")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or(""), 16)
            .map_err(|_| "Clash returned an invalid chunk size")?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if input.len() < size + 2 || &input[size..size + 2] != b"\r\n" {
            return Err("Clash returned a truncated chunked response".into());
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    fn mock_server(status: &str, body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let length = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..length]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn reads_current_mode_with_bearer_secret() {
        let (url, server) = mock_server("200 OK", r#"{"mode":"rule"}"#);
        let client = ControllerClient::new(&url, "hello").unwrap();
        assert_eq!(client.mode().unwrap(), ClashMode::Rule);
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /configs HTTP/1.1"));
        assert!(request.contains("Authorization: Bearer hello\r\n"));
    }

    #[test]
    fn patches_selected_mode() {
        let (url, server) = mock_server("204 No Content", "");
        let client = ControllerClient::new(&url, "").unwrap();
        client.set_mode(ClashMode::Direct).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("PATCH /configs HTTP/1.1"));
        assert!(request.ends_with(r#"{"mode":"direct"}"#));
    }

    #[test]
    fn rejects_non_http_controller_urls() {
        assert!(ControllerClient::new("https://127.0.0.1:9090", "").is_err());
    }
}
