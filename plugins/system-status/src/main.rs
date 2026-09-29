use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    process::{Command, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.system-status";

fn main() -> io::Result<()> {
    run_plugin(SystemStatusPlugin::default())
}

#[derive(Default)]
struct SystemStatusPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for SystemStatusPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(request) = parse_request(query) else {
            return Ok(Vec::new());
        };
        let metrics: Vec<_> = match request {
            StatusRequest::All => Metric::ALL.into_iter().collect(),
            StatusRequest::One(metric) => vec![metric],
        };
        let readings = thread::scope(|scope| {
            metrics
                .iter()
                .copied()
                .map(|metric| (metric, scope.spawn(move || metric.read())))
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|(metric, handle)| {
                    handle.join().ok().flatten().map(|value| (metric, value))
                })
                .collect::<Vec<_>>()
        });
        Ok(readings
            .into_iter()
            .enumerate()
            .take(limit)
            .map(|(index, (metric, reading))| {
                let result_id = format!("system-status:{}", metric.id());
                self.recent_values
                    .insert(result_id.clone(), reading.value.clone());
                status_result(
                    metric,
                    reading,
                    result_id,
                    request == StatusRequest::All,
                    3_100.0 - index as f32,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown system status action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "system status result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("System status copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusRequest {
    All,
    One(Metric),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Metric {
    Battery,
    Disk,
    Memory,
    Cpu,
    Uptime,
    Network,
}

impl Metric {
    const ALL: [Self; 6] = [
        Self::Battery,
        Self::Disk,
        Self::Memory,
        Self::Cpu,
        Self::Uptime,
        Self::Network,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Battery => "battery",
            Self::Disk => "disk",
            Self::Memory => "memory",
            Self::Cpu => "cpu",
            Self::Uptime => "uptime",
            Self::Network => "network",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Battery => "Battery",
            Self::Disk => "Disk",
            Self::Memory => "Memory",
            Self::Cpu => "CPU",
            Self::Uptime => "Uptime",
            Self::Network => "Local IP",
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Battery => "battery.100percent",
            Self::Disk => "internaldrive",
            Self::Memory => "memorychip",
            Self::Cpu => "cpu",
            Self::Uptime => "clock.arrow.circlepath",
            Self::Network => "network",
        }
    }

    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Battery => &["battery", "batt", "电池"],
            Self::Disk => &["disk", "storage", "磁盘", "存储"],
            Self::Memory => &["memory", "ram", "内存"],
            Self::Cpu => &["cpu", "processor", "chip", "处理器", "芯片"],
            Self::Uptime => &["uptime", "boot time", "运行时间", "启动时间"],
            Self::Network => &["ip", "local ip", "network", "网络", "本地 ip"],
        }
    }

    fn read(self) -> Option<Reading> {
        match self {
            Self::Battery => read_battery(),
            Self::Disk => read_disk(),
            Self::Memory => read_memory(),
            Self::Cpu => read_cpu(),
            Self::Uptime => read_uptime(),
            Self::Network => read_network(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Reading {
    value: String,
    detail: String,
}

fn parse_request(query: &str) -> Option<StatusRequest> {
    let query = query.trim().to_lowercase();
    if matches!(
        query.as_str(),
        "system"
            | "sysinfo"
            | "system info"
            | "system status"
            | "status"
            | "系统"
            | "系统信息"
            | "系统状态"
    ) {
        return Some(StatusRequest::All);
    }
    for prefix in ["system ", "sysinfo ", "status ", "系统 "] {
        if let Some(metric) = query.strip_prefix(prefix).and_then(find_metric) {
            return Some(StatusRequest::One(metric));
        }
    }
    find_metric(&query).map(StatusRequest::One)
}

fn find_metric(query: &str) -> Option<Metric> {
    Metric::ALL
        .into_iter()
        .find(|metric| metric.aliases().contains(&query.trim()))
}

fn read_battery() -> Option<Reading> {
    parse_battery(&run("/usr/bin/pmset", &["-g", "batt"])?)
}

fn parse_battery(output: &str) -> Option<Reading> {
    let source = output
        .lines()
        .find_map(|line| {
            line.split_once('\'')
                .map(|(_, rest)| rest.split('\'').next().unwrap_or(rest))
        })
        .unwrap_or("Battery");
    let line = output.lines().find(|line| line.contains('%'))?;
    let percentage = line
        .split_whitespace()
        .find(|part| part.trim_matches(';').ends_with('%'))?
        .trim_matches(';');
    let lowercase = line.to_ascii_lowercase();
    let state = if lowercase.contains("not charging") {
        "Not Charging"
    } else if lowercase.contains("discharging") {
        "Discharging"
    } else if lowercase.contains("finishing charge") {
        "Finishing Charge"
    } else if lowercase.contains("charging") {
        "Charging"
    } else if lowercase.contains("charged") {
        "Fully Charged"
    } else {
        "Battery"
    };
    Some(Reading {
        value: format!("{percentage} · {state}"),
        detail: source.into(),
    })
}

fn read_disk() -> Option<Reading> {
    parse_disk(&run("/bin/df", &["-k", "/"])?)
}

fn parse_disk(output: &str) -> Option<Reading> {
    let fields: Vec<_> = output.lines().last()?.split_whitespace().collect();
    if fields.len() < 5 {
        return None;
    }
    let available_kib = fields.get(3)?.parse::<u64>().ok()?;
    let capacity = fields.get(4)?.to_owned();
    Some(Reading {
        value: format!(
            "{capacity} used · {} free",
            human_bytes(available_kib * 1_024)
        ),
        detail: "Startup disk".into(),
    })
}

fn read_memory() -> Option<Reading> {
    let total = run("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        .and_then(|value| value.trim().parse::<u64>().ok());
    let available =
        run("/usr/bin/memory_pressure", &["-Q"]).and_then(|output| parse_memory_available(&output));
    match (total, available) {
        (Some(total), Some(available)) => Some(Reading {
            value: format!("{} · {available}% available", human_bytes(total)),
            detail: "Physical memory".into(),
        }),
        (Some(total), None) => Some(Reading {
            value: human_bytes(total),
            detail: "Physical memory".into(),
        }),
        (None, Some(available)) => Some(Reading {
            value: format!("{available}% available"),
            detail: "Memory pressure".into(),
        }),
        (None, None) => None,
    }
}

fn parse_memory_available(output: &str) -> Option<u8> {
    let line = output
        .lines()
        .find(|line| line.contains("memory free percentage"))?;
    line.split_whitespace()
        .find_map(|part| part.strip_suffix('%')?.parse().ok())
}

fn read_cpu() -> Option<Reading> {
    let value = run("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"])?;
    Some(Reading {
        value: value.trim().into(),
        detail: "Processor".into(),
    })
}

fn read_uptime() -> Option<Reading> {
    let output = run("/usr/sbin/sysctl", &["-n", "kern.boottime"])?;
    let boot = parse_boot_time(&output)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Reading {
        value: human_duration(now.checked_sub(boot)?),
        detail: "Since last boot".into(),
    })
}

fn parse_boot_time(output: &str) -> Option<u64> {
    let value = output.split("sec = ").nth(1)?;
    value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn read_network() -> Option<Reading> {
    let mut addresses = Vec::new();
    let mut seen = HashSet::new();
    for interface in ["en0", "en1", "en2", "en3", "en4", "en5"] {
        let Some(address) = run("/usr/sbin/ipconfig", &["getifaddr", interface]) else {
            continue;
        };
        let address = address.trim();
        if !address.is_empty() && seen.insert(address.to_owned()) {
            addresses.push(format!("{address} ({interface})"));
        }
    }
    (!addresses.is_empty()).then(|| Reading {
        value: addresses.join(" · "),
        detail: "Active local IPv4 addresses".into(),
    })
}

fn run(path: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(path)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .filter(|value| !value.trim().is_empty())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_024.0 && unit + 1 < UNITS.len() {
        value /= 1_024.0;
        unit += 1;
    }
    if value >= 10.0 || unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn human_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn status_result(
    metric: Metric,
    reading: Reading,
    result_id: String,
    summary: bool,
    score: f32,
) -> SearchResult {
    let title = if summary {
        format!("System {} — {}", metric.title(), reading.value)
    } else {
        metric.title().into()
    };
    let subtitle = if summary {
        reading.detail
    } else {
        format!("{} · {}", reading.value, reading.detail)
    };
    let preview = format!("{} — {}", metric.title(), reading.value);
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: title.clone(),
        subtitle,
        preview,
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([("symbol_name".into(), metric.symbol().into())]),
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy".into(),
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
    fn parses_summary_and_metric_queries() {
        assert_eq!(parse_request("system"), Some(StatusRequest::All));
        assert_eq!(
            parse_request("system battery"),
            Some(StatusRequest::One(Metric::Battery))
        );
        assert_eq!(
            parse_request("内存"),
            Some(StatusRequest::One(Metric::Memory))
        );
        assert_eq!(parse_request("ordinary text"), None);
    }

    #[test]
    fn parses_battery_state_without_confusing_not_charging() {
        let reading = parse_battery(
            "Now drawing from 'AC Power'\n -InternalBattery-0\t80%; AC attached; not charging present: true\n",
        )
        .unwrap();
        assert_eq!(reading.value, "80% · Not Charging");
        assert_eq!(reading.detail, "AC Power");
    }

    #[test]
    fn parses_root_disk_capacity_and_available_space() {
        let reading = parse_disk(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3 1000000 250000 750000 25% /\n",
        )
        .unwrap();
        assert_eq!(reading.value, "25% used · 732 MiB free");
    }

    #[test]
    fn parses_memory_pressure_and_boot_time() {
        assert_eq!(
            parse_memory_available("System-wide memory free percentage: 88%\n"),
            Some(88)
        );
        assert_eq!(
            parse_boot_time("{ sec = 1700000000, usec = 12 } Tuesday"),
            Some(1_700_000_000)
        );
    }

    #[test]
    fn formats_binary_sizes_and_uptime() {
        assert_eq!(human_bytes(68_719_476_736), "64 GiB");
        assert_eq!(
            human_duration(3 * 86_400 + 9 * 3_600 + 34 * 60),
            "3d 9h 34m"
        );
    }

    #[test]
    fn summary_titles_remain_rankable_and_actions_are_safe() {
        let result = status_result(
            Metric::Cpu,
            Reading {
                value: "Apple M5 Pro".into(),
                detail: "Processor".into(),
            },
            "system-status:cpu".into(),
            true,
            3_100.0,
        );
        assert_eq!(result.title, "System CPU — Apple M5 Pro");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);

        let direct = status_result(
            Metric::Battery,
            Reading {
                value: "80% · Not Charging".into(),
                detail: "AC Power".into(),
            },
            "system-status:battery".into(),
            false,
            3_100.0,
        );
        assert_eq!(direct.title, "Battery");
        assert_eq!(direct.subtitle, "80% · Not Charging · AC Power");
        assert_eq!(direct.preview, "Battery — 80% · Not Charging");
    }
}
