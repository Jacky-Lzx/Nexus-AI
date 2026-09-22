use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.time-date";

fn main() -> io::Result<()> {
    run_plugin(TimeDatePlugin::default())
}

#[derive(Default)]
struct TimeDatePlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for TimeDatePlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }

        if let Some(mode) = parse_timestamp_query(query) {
            return self.timestamp_results(mode, limit);
        }
        let Some(filter) = parse_time_query(query) else {
            return Ok(Vec::new());
        };
        let seconds = current_unix_time()?.0;
        let zones: Vec<_> = ZONES
            .iter()
            .enumerate()
            .filter(|(_, zone)| zone.matches(&filter))
            .take(limit)
            .collect();
        let mut results = Vec::with_capacity(zones.len());
        for (index, zone, formatted) in format_zones(seconds, &zones)? {
            let value = format!("{} {}", formatted.time, formatted.abbreviation);
            let result_id = format!("time-date:zone:{}:{seconds}", zone.id);
            self.recent_values.insert(result_id.clone(), value.clone());
            results.push(time_result(
                zone,
                formatted,
                result_id,
                value,
                3_100.0 - index as f32,
            ));
        }
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown time and date action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "time result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Time copied".into()))
    }
}

impl TimeDatePlugin {
    fn timestamp_results(
        &mut self,
        mode: TimestampMode,
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        match mode {
            TimestampMode::Current => {
                let (seconds, milliseconds) = current_unix_time()?;
                let values = [
                    ("seconds", seconds.to_string(), "Unix timestamp · seconds"),
                    (
                        "milliseconds",
                        milliseconds.to_string(),
                        "Unix timestamp · milliseconds",
                    ),
                ];
                Ok(values
                    .into_iter()
                    .enumerate()
                    .take(limit)
                    .map(|(index, (kind, value, subtitle))| {
                        let result_id = format!("time-date:timestamp:{kind}:{value}");
                        self.recent_values.insert(result_id.clone(), value.clone());
                        simple_result(
                            result_id,
                            format!("Timestamp — {value}"),
                            subtitle.into(),
                            "number",
                            3_100.0 - index as f32,
                        )
                    })
                    .collect())
            }
            TimestampMode::Parse(seconds) => {
                let mut results = Vec::new();
                for zone in ZONES.iter().take(2).take(limit) {
                    let formatted = format_in_zone(seconds, zone)?;
                    let value = format!(
                        "{} {} {}",
                        formatted.date, formatted.time, formatted.abbreviation
                    );
                    let result_id = format!("time-date:parsed:{}:{seconds}", zone.id);
                    self.recent_values.insert(result_id.clone(), value.clone());
                    results.push(simple_result(
                        result_id,
                        format!("Timestamp {seconds} — {} — {value}", zone.name),
                        format!("{} · {}", zone.name, formatted.weekday),
                        "clock.arrow.circlepath",
                        3_100.0 - results.len() as f32,
                    ));
                }
                Ok(results)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Zone {
    id: &'static str,
    name: &'static str,
    timezone: Option<&'static str>,
    aliases: &'static [&'static str],
}

impl Zone {
    fn matches(self, filter: &str) -> bool {
        filter.is_empty()
            || self.name.to_ascii_lowercase().contains(filter)
            || self.aliases.contains(&filter)
    }
}

const ZONES: &[Zone] = &[
    Zone {
        id: "local",
        name: "Local Time",
        timezone: None,
        aliases: &["local", "here", "本地"],
    },
    Zone {
        id: "utc",
        name: "UTC",
        timezone: Some("UTC"),
        aliases: &["utc", "gmt"],
    },
    Zone {
        id: "shanghai",
        name: "Shanghai",
        timezone: Some("Asia/Shanghai"),
        aliases: &["shanghai", "beijing", "china", "上海", "北京", "中国"],
    },
    Zone {
        id: "tokyo",
        name: "Tokyo",
        timezone: Some("Asia/Tokyo"),
        aliases: &["tokyo", "japan", "东京", "日本"],
    },
    Zone {
        id: "london",
        name: "London",
        timezone: Some("Europe/London"),
        aliases: &["london", "uk", "britain", "伦敦", "英国"],
    },
    Zone {
        id: "new-york",
        name: "New York",
        timezone: Some("America/New_York"),
        aliases: &["new york", "newyork", "nyc", "eastern", "纽约"],
    },
    Zone {
        id: "los-angeles",
        name: "Los Angeles",
        timezone: Some("America/Los_Angeles"),
        aliases: &["los angeles", "losangeles", "la", "pacific", "洛杉矶"],
    },
    Zone {
        id: "paris",
        name: "Paris",
        timezone: Some("Europe/Paris"),
        aliases: &["paris", "france", "巴黎", "法国"],
    },
    Zone {
        id: "sydney",
        name: "Sydney",
        timezone: Some("Australia/Sydney"),
        aliases: &["sydney", "australia", "悉尼", "澳大利亚"],
    },
    Zone {
        id: "singapore",
        name: "Singapore",
        timezone: Some("Asia/Singapore"),
        aliases: &["singapore", "新加坡"],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimestampMode {
    Current,
    Parse(i64),
}

fn parse_timestamp_query(query: &str) -> Option<TimestampMode> {
    let query = query.trim().to_ascii_lowercase();
    if matches!(query.as_str(), "timestamp" | "epoch" | "unix time") {
        return Some(TimestampMode::Current);
    }
    for prefix in ["timestamp ", "epoch ", "unix "] {
        let Some(value) = query.strip_prefix(prefix) else {
            continue;
        };
        let value = value.trim().parse::<i64>().ok()?;
        if value < 0 {
            return None;
        }
        let seconds = if value > 10_000_000_000 {
            value / 1_000
        } else {
            value
        };
        return Some(TimestampMode::Parse(seconds));
    }
    None
}

fn parse_time_query(query: &str) -> Option<String> {
    let query = query.trim();
    let lowercase = query.to_lowercase();
    if matches!(
        lowercase.as_str(),
        "time" | "date" | "timezone" | "时间" | "时区"
    ) {
        return Some(String::new());
    }
    for prefix in ["time ", "date ", "timezone ", "时间 ", "时区 "] {
        if let Some(filter) = lowercase.strip_prefix(prefix) {
            return Some(filter.trim().to_owned());
        }
    }
    ZONES
        .iter()
        .any(|zone| zone.aliases.contains(&lowercase.as_str()))
        .then_some(lowercase)
}

#[derive(Debug, PartialEq, Eq)]
struct FormattedTime {
    date: String,
    time: String,
    abbreviation: String,
    weekday: String,
}

fn format_in_zone(seconds: i64, zone: &Zone) -> Result<FormattedTime, String> {
    let output = date_command(seconds, zone)
        .output()
        .map_err(|error| format!("could not read system time: {error}"))?;
    parse_date_output(output)
}

fn format_zones<'a>(
    seconds: i64,
    zones: &[(usize, &'a Zone)],
) -> Result<Vec<(usize, &'a Zone, FormattedTime)>, String> {
    let mut pending = Vec::with_capacity(zones.len());
    for &(index, zone) in zones {
        match date_command(seconds, zone).spawn() {
            Ok(child) => pending.push((index, zone, child)),
            Err(error) => {
                for (_, _, mut child) in pending {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(format!("could not start system time query: {error}"));
            }
        }
    }

    pending
        .into_iter()
        .map(|(index, zone, child)| {
            let output = child
                .wait_with_output()
                .map_err(|error| format!("could not read system time: {error}"))?;
            Ok((index, zone, parse_date_output(output)?))
        })
        .collect()
}

fn date_command(seconds: i64, zone: &Zone) -> Command {
    let mut command = Command::new("/bin/date");
    command
        .arg("-r")
        .arg(seconds.to_string())
        .arg("+%Y-%m-%d|%H:%M:%S|%Z|%A")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(timezone) = zone.timezone {
        command.env("TZ", timezone);
    }
    command
}

fn parse_date_output(output: std::process::Output) -> Result<FormattedTime, String> {
    if !output.status.success() {
        return Err(format!("system date returned {}", output.status));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "system date returned invalid UTF-8".to_owned())?;
    let mut fields = output.trim().split('|');
    let formatted = FormattedTime {
        date: fields.next().unwrap_or_default().into(),
        time: fields.next().unwrap_or_default().into(),
        abbreviation: fields.next().unwrap_or_default().into(),
        weekday: fields.next().unwrap_or_default().into(),
    };
    if formatted.date.is_empty()
        || formatted.time.is_empty()
        || formatted.abbreviation.is_empty()
        || formatted.weekday.is_empty()
        || fields.next().is_some()
    {
        return Err("system date returned an unexpected format".into());
    }
    Ok(formatted)
}

fn current_unix_time() -> Result<(i64, i128), String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?;
    let seconds = i64::try_from(duration.as_secs())
        .map_err(|_| "current timestamp is too large".to_owned())?;
    Ok((seconds, duration.as_millis() as i128))
}

fn time_result(
    zone: &Zone,
    formatted: FormattedTime,
    result_id: String,
    value: String,
    score: f32,
) -> SearchResult {
    simple_result(
        result_id,
        format!("Time {} — {value}", zone.name),
        format!("{} · {}, {}", zone.name, formatted.weekday, formatted.date),
        "clock",
        score,
    )
}

fn simple_result(
    result_id: String,
    title: String,
    subtitle: String,
    symbol: &str,
    score: f32,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: title.clone(),
        subtitle,
        preview: title,
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([("symbol_name".into(), symbol.into())]),
        actions: vec![
            ResultAction {
                id: "paste".into(),
                title: "Paste".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
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
    fn parses_time_queries_and_exact_city_aliases() {
        assert_eq!(parse_time_query("time"), Some(String::new()));
        assert_eq!(parse_time_query("time New York"), Some("new york".into()));
        assert_eq!(parse_time_query("东京"), Some("东京".into()));
        assert_eq!(parse_time_query("ordinary text"), None);
    }

    #[test]
    fn filters_named_time_zones() {
        let matches: Vec<_> = ZONES
            .iter()
            .filter(|zone| zone.matches("new york"))
            .map(|zone| zone.id)
            .collect();
        assert_eq!(matches, ["new-york"]);
    }

    #[test]
    fn parses_seconds_and_millisecond_timestamps() {
        assert_eq!(
            parse_timestamp_query("timestamp"),
            Some(TimestampMode::Current)
        );
        assert_eq!(
            parse_timestamp_query("epoch 1700000000"),
            Some(TimestampMode::Parse(1_700_000_000))
        );
        assert_eq!(
            parse_timestamp_query("timestamp 1700000000123"),
            Some(TimestampMode::Parse(1_700_000_000))
        );
        assert_eq!(parse_timestamp_query("timestamp -1"), None);
    }

    #[test]
    fn utc_epoch_uses_the_system_timezone_database() {
        let formatted = format_in_zone(0, &ZONES[1]).unwrap();
        assert_eq!(formatted.date, "1970-01-01");
        assert_eq!(formatted.time, "00:00:00");
        assert_eq!(formatted.abbreviation, "UTC");
        assert_eq!(formatted.weekday, "Thursday");
    }

    #[test]
    fn results_have_copy_and_paste_actions() {
        let result = simple_result(
            "time-date:test".into(),
            "12:00:00 UTC".into(),
            "UTC".into(),
            "clock",
            3_100.0,
        );
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
    }

    #[test]
    fn world_clock_titles_keep_the_time_query_rankable() {
        let result = time_result(
            &ZONES[3],
            FormattedTime {
                date: "2026-09-23".into(),
                time: "01:30:00".into(),
                abbreviation: "JST".into(),
                weekday: "Wednesday".into(),
            },
            "time-date:test".into(),
            "01:30:00 JST".into(),
            3_100.0,
        );
        assert_eq!(result.title, "Time Tokyo — 01:30:00 JST");
    }
}
