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
        let modes = if query.trim().is_empty() {
            vec![LocalQuery::Time, LocalQuery::Date]
        } else {
            match parse_local_query(query) {
                Some(mode) => vec![mode],
                None => return Ok(Vec::new()),
            }
        };
        let seconds = current_unix_time()?.0;
        let formatted = format_local(seconds)?;
        let mut results = Vec::with_capacity(modes.len().min(limit));
        for (index, mode) in modes.into_iter().take(limit).enumerate() {
            let (result_id, title, subtitle, value, symbol) = match mode {
                LocalQuery::Time => (
                    format!("time-date:local-time:{seconds}"),
                    format!("Time — {}", formatted.time),
                    format!(
                        "Local time · {} · {}",
                        formatted.abbreviation, formatted.date
                    ),
                    formatted.time.clone(),
                    "clock",
                ),
                LocalQuery::Date => (
                    format!("time-date:local-date:{seconds}"),
                    format!("Date — {}", formatted.date),
                    format!("Local date · {}", formatted.weekday),
                    formatted.date.clone(),
                    "calendar",
                ),
            };
            self.recent_values.insert(result_id.clone(), value);
            results.push(simple_result(
                result_id,
                title,
                subtitle,
                symbol,
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
        Ok(Some("Copied".into()))
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
                let formatted = format_local(seconds)?;
                let value = format!(
                    "{} {} {}",
                    formatted.date, formatted.time, formatted.abbreviation
                );
                let result_id = format!("time-date:parsed:local:{seconds}");
                self.recent_values.insert(result_id.clone(), value.clone());
                Ok(vec![simple_result(
                    result_id,
                    format!("Timestamp {seconds} — {value}"),
                    format!("Local time · {}", formatted.weekday),
                    "clock.arrow.circlepath",
                    3_100.0,
                )])
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalQuery {
    Time,
    Date,
}

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

fn parse_local_query(query: &str) -> Option<LocalQuery> {
    match query.trim().to_lowercase().as_str() {
        "time" | "time local" | "timezone" | "local" | "here" | "时间" | "时区" | "本地"
        | "本地时间" => Some(LocalQuery::Time),
        "date" | "date local" | "日期" | "本地日期" => Some(LocalQuery::Date),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FormattedTime {
    date: String,
    time: String,
    abbreviation: String,
    weekday: String,
}

fn format_local(seconds: i64) -> Result<FormattedTime, String> {
    let output = date_command(seconds)
        .output()
        .map_err(|error| format!("could not read system time: {error}"))?;
    parse_date_output(output)
}

fn date_command(seconds: i64) -> Command {
    let mut command = Command::new("/bin/date");
    command
        .arg("-r")
        .arg(seconds.to_string())
        .arg("+%Y.%m.%d|%H:%M:%S|%Z|%A")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
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
    fn recognizes_only_local_time_and_date_queries() {
        assert_eq!(parse_local_query("time"), Some(LocalQuery::Time));
        assert_eq!(parse_local_query("日期"), Some(LocalQuery::Date));
        assert_eq!(parse_local_query("time local"), Some(LocalQuery::Time));
        assert_eq!(parse_local_query("timezone"), Some(LocalQuery::Time));
        assert_eq!(parse_local_query("time New York"), None);
        assert_eq!(parse_local_query("东京"), None);
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
    fn local_format_uses_the_system_timezone() {
        let output = date_command(0).env("TZ", "UTC").output().unwrap();
        let formatted = parse_date_output(output).unwrap();
        assert_eq!(formatted.date, "1970.01.01");
        assert_eq!(formatted.time, "00:00:00");
        assert_eq!(formatted.abbreviation, "UTC");
        assert_eq!(formatted.weekday, "Thursday");
    }

    #[test]
    fn search_returns_one_local_result_and_date_is_copyable() {
        let mut plugin = TimeDatePlugin::default();
        let time = plugin.search("time", 10).unwrap();
        assert_eq!(time.len(), 1);
        assert!(time[0].title.starts_with("Time — "));
        assert!(time[0].subtitle.starts_with("Local time · "));

        let date = plugin.search("date", 10).unwrap();
        assert_eq!(date.len(), 1);
        let value = plugin.recent_values.get(&date[0].id).unwrap();
        assert_eq!(value.len(), 10);
        assert_eq!(value.as_bytes()[4], b'.');
        assert_eq!(value.as_bytes()[7], b'.');
        assert_eq!(date[0].title, format!("Date — {value}"));
    }

    #[test]
    fn empty_query_shows_time_then_date_and_respects_limit() {
        let mut plugin = TimeDatePlugin::default();
        let results = plugin.search("", 10).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].title.starts_with("Time — "));
        assert!(results[1].title.starts_with("Date — "));
        assert!(plugin.recent_values.contains_key(&results[0].id));
        assert!(plugin.recent_values.contains_key(&results[1].id));

        let limited = plugin.search(" ", 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert!(limited[0].title.starts_with("Time — "));
    }

    #[test]
    fn parsed_timestamp_returns_only_local_time() {
        let mut plugin = TimeDatePlugin::default();
        let results = plugin.search("timestamp 0", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].subtitle.split('·').next(), Some("Local time "));
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
}
