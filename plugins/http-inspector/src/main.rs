use clipkeep_plugin_sdk::{Plugin, SearchContext, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io, process::Command};
use url::Url;

const PLUGIN_ID: &str = "io.clipkeep.http-inspector";
const RESULT_ID: &str = "http-inspector:result";
const CURL_PATH: &str = "/usr/bin/curl";
const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_EFFECTIVE_URL_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const WRITE_OUT: &str = concat!(
    "nexus_status=%{http_code}\n",
    "nexus_url=%{url_effective}\n",
    "nexus_redirects=%{num_redirects}\n",
    "nexus_remote_ip=%{remote_ip}\n",
    "nexus_remote_port=%{remote_port}\n",
    "nexus_http_version=%{http_version}\n",
    "nexus_content_type=%{content_type}\n",
    "nexus_header_bytes=%{size_header}\n",
    "nexus_dns=%{time_namelookup}\n",
    "nexus_connect=%{time_connect}\n",
    "nexus_tls=%{time_appconnect}\n",
    "nexus_ttfb=%{time_starttransfer}\n",
    "nexus_total=%{time_total}\n",
    "nexus_ssl_verify=%{ssl_verify_result}\n"
);

fn main() -> io::Result<()> {
    run_plugin(HttpInspectorPlugin::default())
}

#[derive(Default)]
struct HttpInspectorPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for HttpInspectorPlugin {
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
        let probe = match run_probe(&request.url, context) {
            Ok(probe) => probe,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values
            .insert("paste_url".into(), probe.final_url.clone());
        self.action_values
            .insert("copy_url".into(), probe.final_url.clone());
        self.action_values
            .insert("copy_status".into(), probe.status.to_string());
        if let Some(remote_ip) = &probe.remote_ip {
            self.action_values
                .insert("copy_remote_ip".into(), remote_ip.clone());
        }
        Ok(vec![inspection_result(&request, &probe)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("HTTP inspection is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive HTTP Inspector action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_url" | "copy_url" => "Final URL copied",
                "copy_status" => "HTTP status copied",
                "copy_remote_ip" => "Remote IP copied",
                _ => "HTTP value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HttpRequest {
    url: String,
}

#[derive(Clone, Debug, PartialEq)]
struct HttpProbe {
    status: u16,
    final_url: String,
    redirects: u32,
    remote_ip: Option<String>,
    remote_port: Option<u16>,
    http_version: String,
    content_type: Option<String>,
    header_bytes: u64,
    dns_seconds: f64,
    connect_seconds: f64,
    tls_seconds: f64,
    ttfb_seconds: f64,
    total_seconds: f64,
    ssl_verify_result: u64,
}

fn parse_request(query: &str) -> Result<Option<HttpRequest>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "http" | "http-head")
        && command != "网页状态"
    {
        return Ok(None);
    }
    let input = query[command_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace());
    if input.is_empty() {
        return Err("expected a URL after 'http', 'http-head', or '网页状态'".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("URL input exceeds 16 KiB".into());
    }
    let candidate = if has_explicit_scheme(input) {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let parsed = Url::parse(&candidate).map_err(|error| format!("invalid URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only HTTP and HTTPS URLs can be inspected".into());
    }
    if parsed.host_str().is_none() {
        return Err("HTTP URL must include a host".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs containing usernames or passwords are not accepted".into());
    }
    Ok(Some(HttpRequest {
        url: parsed.to_string(),
    }))
}

fn has_explicit_scheme(input: &str) -> bool {
    if input.starts_with('[') || looks_like_host_port(input) {
        return false;
    }
    let Some(colon) = input.find(':') else {
        return false;
    };
    let scheme = &input[..colon];
    !scheme.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn looks_like_host_port(input: &str) -> bool {
    let Some((host, _)) = input.split_once(':') else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host.contains('.')
        || host.parse::<std::net::Ipv4Addr>().is_ok()
}

fn curl_arguments(url: &str) -> Vec<String> {
    vec![
        "--silent".into(),
        "--show-error".into(),
        "--head".into(),
        "--location".into(),
        "--connect-timeout".into(),
        "3".into(),
        "--max-time".into(),
        "8".into(),
        "--max-redirs".into(),
        "5".into(),
        "--proto".into(),
        "=http,https".into(),
        "--proto-redir".into(),
        "=http,https".into(),
        "--user-agent".into(),
        "Nexus-HTTP-Inspector/0.1".into(),
        "--output".into(),
        "/dev/null".into(),
        "--write-out".into(),
        WRITE_OUT.into(),
        "--".into(),
        url.into(),
    ]
}

fn run_probe(url: &str, context: &SearchContext) -> Result<HttpProbe, String> {
    let output = clipkeep_plugin_sdk::command_output(
        Command::new(CURL_PATH).args(curl_arguments(url)),
        context,
        MAX_OUTPUT_BYTES,
    )
    .map_err(|error| format!("could not start HTTP probe: {error}"))?;
    if output.stdout.len() > MAX_OUTPUT_BYTES || output.stderr.len() > MAX_OUTPUT_BYTES {
        return Err("HTTP probe output exceeded 256 KiB".into());
    }
    if !output.status.success() {
        let detail = compact(
            &safe_text(String::from_utf8_lossy(&output.stderr).trim()),
            240,
        );
        return Err(if detail.is_empty() {
            format!("HTTP probe exited with {}", output.status)
        } else {
            detail
        });
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "HTTP probe returned non-UTF-8 metadata".to_owned())?;
    parse_probe_output(&output)
}

fn parse_probe_output(output: &str) -> Result<HttpProbe, String> {
    let mut fields = BTreeMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=')
            && key.starts_with("nexus_")
        {
            fields.insert(key, value);
        }
    }
    let status = parse_field::<u16>(&fields, "nexus_status")?;
    if !(100..=599).contains(&status) {
        return Err("HTTP probe did not return a valid status code".into());
    }
    let final_url = required_field(&fields, "nexus_url")?;
    let parsed = Url::parse(final_url).map_err(|_| "HTTP probe returned an invalid URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("HTTP probe redirected to an unsupported URL scheme".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("HTTP probe returned a URL containing credentials".into());
    }
    let final_url = parsed.to_string();
    if final_url.len() > MAX_EFFECTIVE_URL_BYTES {
        return Err("HTTP probe returned a URL longer than 64 KiB".into());
    }
    Ok(HttpProbe {
        status,
        final_url,
        redirects: parse_field(&fields, "nexus_redirects")?,
        remote_ip: optional_text(&fields, "nexus_remote_ip", 200),
        remote_port: optional_parse(&fields, "nexus_remote_port")?,
        http_version: compact(
            &safe_text(required_field(&fields, "nexus_http_version")?),
            32,
        ),
        content_type: optional_text(&fields, "nexus_content_type", 500),
        header_bytes: parse_field(&fields, "nexus_header_bytes")?,
        dns_seconds: parse_duration(&fields, "nexus_dns")?,
        connect_seconds: parse_duration(&fields, "nexus_connect")?,
        tls_seconds: parse_duration(&fields, "nexus_tls")?,
        ttfb_seconds: parse_duration(&fields, "nexus_ttfb")?,
        total_seconds: parse_duration(&fields, "nexus_total")?,
        ssl_verify_result: parse_field(&fields, "nexus_ssl_verify")?,
    })
}

fn required_field<'a>(fields: &'a BTreeMap<&str, &str>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("HTTP probe omitted {key}"))
}

fn parse_field<T: std::str::FromStr>(
    fields: &BTreeMap<&str, &str>,
    key: &str,
) -> Result<T, String> {
    required_field(fields, key)?
        .parse()
        .map_err(|_| format!("HTTP probe returned an invalid {key}"))
}

fn optional_parse<T: std::str::FromStr>(
    fields: &BTreeMap<&str, &str>,
    key: &str,
) -> Result<Option<T>, String> {
    let Some(value) = fields.get(key).copied().filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    value
        .parse()
        .map(Some)
        .map_err(|_| format!("HTTP probe returned an invalid {key}"))
}

fn optional_text(
    fields: &BTreeMap<&str, &str>,
    key: &str,
    maximum_characters: usize,
) -> Option<String> {
    fields
        .get(key)
        .copied()
        .filter(|value| !value.is_empty())
        .map(|value| compact(&safe_text(value), maximum_characters))
}

fn parse_duration(fields: &BTreeMap<&str, &str>, key: &str) -> Result<f64, String> {
    let value: f64 = parse_field(fields, key)?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(format!("HTTP probe returned an invalid {key}"))
    }
}

fn inspection_result(request: &HttpRequest, probe: &HttpProbe) -> SearchResult {
    let reason = status_reason(probe.status);
    let version = http_version_label(&probe.http_version);
    let final_url = Url::parse(&probe.final_url).ok();
    let host = final_url
        .as_ref()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown host".into());
    let remote = match (&probe.remote_ip, probe.remote_port) {
        (Some(ip), Some(port)) => format!("{ip}:{port}"),
        (Some(ip), None) => ip.clone(),
        _ => "not reported".into(),
    };
    let content_type = probe.content_type.as_deref().unwrap_or("not reported");
    let tls = if final_url
        .as_ref()
        .is_some_and(|url| url.scheme() == "https")
    {
        if probe.ssl_verify_result == 0 {
            "verified"
        } else {
            "verification failed"
        }
    } else {
        "not used"
    };
    let mut metadata = BTreeMap::from([
        (
            "symbol_name".into(),
            "network.badge.shield.half.filled".into(),
        ),
        ("preview_style".into(), "text".into()),
    ]);
    if probe.status >= 500 {
        metadata.insert("symbol_color".into(), "#FF453A".into());
    } else if probe.status >= 400 {
        metadata.insert("symbol_color".into(), "#FF9F0A".into());
    }
    let mut actions = vec![
        action("paste_url", "Paste Final URL", true, true, true),
        action("copy_url", "Copy Final URL", false, false, false),
        action("copy_status", "Copy Status Code", false, false, false),
    ];
    if probe.remote_ip.is_some() {
        actions.push(action(
            "copy_remote_ip",
            "Copy Remote IP",
            false,
            false,
            false,
        ));
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!("HTTP {} · {reason}", probe.status),
        subtitle: format!(
            "{host} · {version} · {} · {} redirect{}",
            format_duration(probe.total_seconds),
            probe.redirects,
            if probe.redirects == 1 { "" } else { "s" }
        ),
        preview: format!(
            "HTTP inspection\nRequested URL: {}\nFinal URL: {}\nStatus: {} {}\nProtocol: {}\nRedirects: {}\nRemote peer: {}\nContent type: {}\nHeader bytes: {}\nTLS: {}\n\nTimings\nDNS: {}\nConnect: {}\nTLS handshake: {}\nFirst byte: {}\nTotal: {}",
            compact(&safe_text(&request.url), 1_024),
            compact(&safe_text(&probe.final_url), 1_024),
            probe.status,
            reason,
            version,
            probe.redirects,
            remote,
            content_type,
            probe.header_bytes,
            tls,
            format_duration(probe.dns_seconds),
            format_duration(probe.connect_seconds),
            format_duration(probe.tls_seconds),
            format_duration(probe.ttfb_seconds),
            format_duration(probe.total_seconds)
        ),
        kind: ResultKind::Command,
        score: 3_440.0,
        metadata,
        actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "http-inspector:invalid".into(),
        title: compact(query.trim(), 200),
        subtitle: format!("HTTP inspection unavailable · {}", compact(message, 120)),
        preview: format!("HTTP inspection unavailable\n{message}"),
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

fn http_version_label(value: &str) -> String {
    match value {
        "1" | "1.0" => "HTTP/1.0".into(),
        "1.1" => "HTTP/1.1".into(),
        "2" => "HTTP/2".into(),
        "3" => "HTTP/3".into(),
        value => format!("HTTP/{}", compact(&safe_text(value), 20)),
    }
}

fn status_reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        418 => "I'm a Teapot",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => match status / 100 {
            1 => "Informational",
            2 => "Success",
            3 => "Redirection",
            4 => "Client Error",
            5 => "Server Error",
            _ => "Unknown",
        },
    }
}

fn format_duration(seconds: f64) -> String {
    if seconds < 0.001 {
        format!("{:.0} µs", seconds * 1_000_000.0)
    } else if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1_000.0)
    } else {
        format!("{seconds:.2} s")
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

    const SAMPLE: &str = concat!(
        "nexus_status=200\n",
        "nexus_url=https://example.com/final\n",
        "nexus_redirects=1\n",
        "nexus_remote_ip=93.184.216.34\n",
        "nexus_remote_port=443\n",
        "nexus_http_version=2\n",
        "nexus_content_type=text/html; charset=UTF-8\n",
        "nexus_header_bytes=512\n",
        "nexus_dns=0.010000\n",
        "nexus_connect=0.020000\n",
        "nexus_tls=0.050000\n",
        "nexus_ttfb=0.080000\n",
        "nexus_total=0.100000\n",
        "nexus_ssl_verify=0\n"
    );

    #[test]
    fn parses_explicit_commands_and_defaults_to_https() {
        assert_eq!(
            parse_request("http example.com/path").unwrap(),
            Some(HttpRequest {
                url: "https://example.com/path".into(),
            })
        );
        assert_eq!(
            parse_request("网页状态: http://example.com").unwrap(),
            Some(HttpRequest {
                url: "http://example.com/".into(),
            })
        );
        assert_eq!(
            parse_request("http example.com/?next=http://internal").unwrap(),
            Some(HttpRequest {
                url: "https://example.com/?next=http://internal".into(),
            })
        );
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn rejects_missing_unsupported_credentialed_and_oversized_urls() {
        assert!(parse_request("http").is_err());
        assert!(parse_request("http ftp://example.com/file").is_err());
        assert!(parse_request("http https://user:secret@example.com").is_err());
        assert!(
            parse_request(&format!(
                "http https://example.com/{}",
                "x".repeat(MAX_INPUT_BYTES)
            ))
            .is_err()
        );
    }

    #[test]
    fn curl_arguments_are_bounded_and_keep_the_url_in_one_argument() {
        let url = "https://example.com/?q=a;b";
        let arguments = curl_arguments(url);
        assert_eq!(arguments.last().unwrap(), url);
        assert_eq!(arguments[arguments.len() - 2], "--");
        assert!(arguments.windows(2).any(|pair| pair == ["--max-time", "8"]));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--proto-redir", "=http,https"])
        );
    }

    #[test]
    fn parses_complete_probe_metadata() {
        let probe = parse_probe_output(SAMPLE).unwrap();
        assert_eq!(probe.status, 200);
        assert_eq!(probe.final_url, "https://example.com/final");
        assert_eq!(probe.redirects, 1);
        assert_eq!(probe.remote_port, Some(443));
        assert_eq!(
            probe.content_type.as_deref(),
            Some("text/html; charset=UTF-8")
        );
        assert_eq!(probe.total_seconds, 0.1);
    }

    #[test]
    fn rejects_missing_invalid_and_unsafe_probe_metadata() {
        assert!(parse_probe_output("nexus_status=200\n").is_err());
        assert!(
            parse_probe_output(&SAMPLE.replace("nexus_status=200", "nexus_status=000")).is_err()
        );
        assert!(
            parse_probe_output(&SAMPLE.replace("nexus_total=0.100000", "nexus_total=NaN")).is_err()
        );
        assert!(
            parse_probe_output(&SAMPLE.replace("https://example.com/final", "file:///tmp/a"))
                .is_err()
        );
    }

    #[test]
    fn result_exposes_status_timings_and_copy_actions() {
        let request = parse_request("http http://example.com").unwrap().unwrap();
        let result = inspection_result(&request, &parse_probe_output(SAMPLE).unwrap());
        assert_eq!(result.title, "HTTP 200 · OK");
        assert!(result.subtitle.contains("HTTP/2 · 100 ms · 1 redirect"));
        assert!(result.preview.contains("TLS: verified"));
        assert_eq!(result.actions.len(), 4);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
    }

    #[test]
    fn error_statuses_remain_visible_and_are_colored() {
        let request = parse_request("http example.com").unwrap().unwrap();
        let probe =
            parse_probe_output(&SAMPLE.replace("nexus_status=200", "nexus_status=503")).unwrap();
        let result = inspection_result(&request, &probe);
        assert_eq!(result.title, "HTTP 503 · Service Unavailable");
        assert_eq!(result.metadata["symbol_color"], "#FF453A");
        assert!(!result.actions.is_empty());
    }

    #[test]
    fn formatting_and_sanitization_are_stable() {
        assert_eq!(format_duration(0.000_4), "400 µs");
        assert_eq!(format_duration(0.012_4), "12 ms");
        assert_eq!(format_duration(1.234), "1.23 s");
        assert_eq!(
            safe_text("safe\nspoof\u{202E}"),
            "safe\\u{000A}spoof\\u{202E}"
        );
        assert_eq!(status_reason(499), "Client Error");
    }

    #[test]
    fn nonmatching_and_zero_limit_searches_clear_stale_actions() {
        let mut plugin = HttpInspectorPlugin::default();
        plugin
            .action_values
            .insert("copy_url".into(), "stale".into());
        assert!(plugin.search("ordinary search", 10).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        plugin
            .action_values
            .insert("copy_url".into(), "stale".into());
        assert!(plugin.search("http example.com", 0).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
    }
}
