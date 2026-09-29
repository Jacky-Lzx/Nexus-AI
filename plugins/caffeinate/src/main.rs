use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{
    ActionInputField, ActionInputKind, PluginPresentationItem, PresentationStyle, ResultAction,
    ResultKind, SearchResult,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io,
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime},
};

const PLUGIN_ID: &str = "io.clipkeep.caffeinate";

fn main() -> io::Result<()> {
    run_plugin(CaffeinatePlugin::default())
}

#[derive(Default)]
struct CaffeinatePlugin {
    active: Option<ActiveCaffeination>,
}

struct ActiveCaffeination {
    child: Child,
    ends_at: Option<SystemTime>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaffeinateInput {
    #[serde(default)]
    hours: Option<u64>,
    #[serde(default)]
    minutes: Option<u64>,
    #[serde(default)]
    seconds: Option<u64>,
}

impl CaffeinatePlugin {
    fn refresh(&mut self) {
        let finished = self
            .active
            .as_mut()
            .and_then(|active| active.child.try_wait().ok().flatten())
            .is_some();
        if finished {
            self.active = None;
        }
    }

    fn start(&mut self, input: &str) -> Result<Option<String>, String> {
        let duration_seconds = parse_duration(input)?;
        if duration_seconds == Some(0) {
            return Err("caffeinate duration must be greater than zero".into());
        }
        self.refresh();
        self.stop()?;
        let mut command = Command::new("/usr/bin/caffeinate");
        command.arg("-di");
        if let Some(seconds) = duration_seconds {
            command.arg("-t").arg(seconds.to_string());
        }
        let child = command
            .arg("-w")
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start caffeinate: {error}"))?;
        self.active = Some(ActiveCaffeination {
            child,
            ends_at: duration_seconds
                .map(|seconds| SystemTime::now() + Duration::from_secs(seconds)),
        });
        Ok(Some(match duration_seconds {
            Some(seconds) => format!("Keeping this Mac awake for {}", format_duration(seconds)),
            None => "Keeping this Mac awake indefinitely".into(),
        }))
    }

    fn stop(&mut self) -> Result<Option<String>, String> {
        if let Some(mut active) = self.active.take() {
            if active
                .child
                .try_wait()
                .map_err(|error| format!("could not inspect caffeinate: {error}"))?
                .is_none()
            {
                active
                    .child
                    .kill()
                    .map_err(|error| format!("could not stop caffeinate: {error}"))?;
            }
            let _ = active.child.wait();
            Ok(Some("Caffeination ended".into()))
        } else {
            Ok(Some("No active caffeination".into()))
        }
    }
}

impl Drop for CaffeinatePlugin {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl Plugin for CaffeinatePlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.active.as_ref().map(|_| Duration::from_secs(1))
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.refresh();
        let query = query.trim().to_lowercase();
        let mut results = Vec::new();
        if matches_query(
            &query,
            &[
                "caffeinate",
                "keep awake",
                "prevent sleep",
                "保持唤醒",
                "常亮",
            ],
        ) {
            results.push(start_result(self.active.as_ref()));
        }
        if let Some(active) = self.active.as_ref()
            && matches_query(&query, &["decaffeinate", "stop", "cancel", "停止", "取消"])
        {
            results.push(stop_result(active));
        }
        results.truncate(limit);
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        match (result_id, action_id) {
            ("caffeinate:start", "start") => Err("a caffeinate duration is required".into()),
            ("caffeinate:stop", "stop") => self.stop(),
            _ => Err(format!("unknown caffeinate action: {action_id}")),
        }
    }

    fn execute_with_input(
        &mut self,
        result_id: &str,
        action_id: &str,
        input: Option<&str>,
    ) -> Result<Option<String>, String> {
        if result_id == "caffeinate:start" && action_id == "start" {
            return self.start(input.ok_or("a caffeinate duration is required")?);
        }
        self.execute(result_id, action_id)
    }

    fn tick(&mut self) -> Result<bool, String> {
        let was_active = self.active.is_some();
        self.refresh();
        Ok(was_active && self.active.is_none())
    }

    fn presentation_items(&self) -> Vec<PluginPresentationItem> {
        let Some(active) = self.active.as_ref() else {
            return Vec::new();
        };
        let mut metadata = BTreeMap::from([("symbol_name".into(), "cup.and.saucer.fill".into())]);
        if let Some(ends_at) = active.ends_at {
            let countdown_until = ends_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            metadata.insert("countdown_until".into(), countdown_until.to_string());
        }
        vec![PluginPresentationItem {
            id: "caffeinate-status".into(),
            title: "Caffeinated".into(),
            value: active
                .ends_at
                .map(|_| format!("{} remaining", remaining_duration(active)))
                .unwrap_or_else(|| "Indefinitely".into()),
            compact_value: Some(if active.ends_at.is_some() {
                "Time left".into()
            } else {
                "Awake".into()
            }),
            style: PresentationStyle::Success,
            metadata,
        }]
    }
}

fn start_result(active: Option<&ActiveCaffeination>) -> SearchResult {
    let subtitle = active
        .map(|active| match active.ends_at {
            Some(_) => format!(
                "Active · {} remaining · choose a new end time",
                remaining_duration(active)
            ),
            None => "Active · no end time · choose a duration".into(),
        })
        .unwrap_or_else(|| {
            "Keep the display and Mac awake indefinitely or for a chosen time".into()
        });
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "caffeinate:start".into(),
        title: "Caffeinate".into(),
        subtitle: subtitle.clone(),
        preview: subtitle,
        kind: ResultKind::Command,
        score: 4_000.0,
        metadata: BTreeMap::from([("symbol_name".into(), "cup.and.saucer.fill".into())]),
        actions: vec![ResultAction {
            host_action: None,
            id: "start".into(),
            title: "Caffeinate".into(),
            destructive: false,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: vec![
                duration_field("hours", "hour", 8_760),
                duration_field("minutes", "min", 59),
            ],
        }],
    }
}

fn stop_result(active: &ActiveCaffeination) -> SearchResult {
    let subtitle = match active.ends_at {
        Some(_) => format!("Cancel the current {} session", remaining_duration(active)),
        None => "Cancel the current indefinite session".into(),
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "caffeinate:stop".into(),
        title: "Decaffeinate".into(),
        subtitle: subtitle.clone(),
        preview: subtitle,
        kind: ResultKind::Command,
        score: 3_900.0,
        metadata: BTreeMap::from([("symbol_name".into(), "cup.and.saucer".into())]),
        actions: vec![ResultAction {
            host_action: None,
            id: "stop".into(),
            title: "Decaffeinate".into(),
            destructive: false,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

fn duration_field(id: &str, label: &str, maximum: i64) -> ActionInputField {
    ActionInputField {
        id: id.into(),
        label: label.into(),
        placeholder: label.into(),
        kind: ActionInputKind::Integer,
        required: false,
        minimum: Some(0),
        maximum: Some(maximum),
    }
}

fn parse_duration(input: &str) -> Result<Option<u64>, String> {
    let input: CaffeinateInput =
        serde_json::from_str(input).map_err(|error| format!("invalid duration: {error}"))?;
    if input.hours.is_none() && input.minutes.is_none() && input.seconds.is_none() {
        return Ok(None);
    }
    let hours = input.hours.unwrap_or(0);
    let minutes = input.minutes.unwrap_or(0);
    let seconds = input.seconds.unwrap_or(0);
    if hours > 8_760 || minutes > 59 || seconds > 59 {
        return Err("duration must use hours 0–8760 and minutes/seconds 0–59".into());
    }
    hours
        .checked_mul(3_600)
        .and_then(|hours| {
            minutes
                .checked_mul(60)
                .and_then(|minutes| hours.checked_add(minutes))
        })
        .and_then(|duration| duration.checked_add(seconds))
        .map(Some)
        .ok_or_else(|| "caffeinate duration is too large".into())
}

fn matches_query(query: &str, candidates: &[&str]) -> bool {
    query.is_empty() || candidates.iter().any(|candidate| candidate.contains(query))
}

fn remaining_duration(active: &ActiveCaffeination) -> String {
    format_duration(
        active
            .ends_at
            .expect("timed caffeination has an end time")
            .duration_since(SystemTime::now())
            .unwrap_or_default()
            .as_secs(),
    )
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600).div_ceil(60);
    match (hours, minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_arguments_match_duration_input() {
        let result = start_result(None);
        let fields = &result.actions[0].input_fields;
        assert_eq!(fields.len(), 2);
        assert_eq!((&*fields[0].id, &*fields[0].label), ("hours", "hour"));
        assert_eq!((&*fields[1].id, &*fields[1].label), ("minutes", "min"));
        assert_eq!(
            parse_duration(r#"{"hours":1,"minutes":30}"#).unwrap(),
            Some(5_400)
        );
        assert_eq!(parse_duration(r#"{"minutes":15}"#).unwrap(), Some(900));
        assert_eq!(parse_duration("{}").unwrap(), None);
        assert_eq!(
            parse_duration(r#"{"hours":null,"minutes":null}"#).unwrap(),
            None
        );
        for invalid in [
            r#"{"hours":8761}"#,
            r#"{"minutes":60}"#,
            r#"{"hours":-1}"#,
            r#"{"minutes":1.5}"#,
            r#"{"hours":"one"}"#,
            r#"{"duration":1}"#,
        ] {
            assert!(parse_duration(invalid).is_err(), "{invalid}");
        }
        assert!(
            CaffeinatePlugin::default()
                .start(r#"{"hours":0,"minutes":0}"#)
                .is_err()
        );
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3_600), "1h");
        assert_eq!(format_duration(5_400), "1h 30m");
    }

    #[test]
    fn inactive_search_only_contains_start() {
        let mut plugin = CaffeinatePlugin::default();
        let results = plugin.search("", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "caffeinate:start");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn starts_and_stops_system_caffeinate() {
        let mut plugin = CaffeinatePlugin::default();
        plugin
            .start(r#"{"hours":0,"minutes":0,"seconds":5}"#)
            .unwrap();
        assert!(plugin.active.is_some());
        plugin.stop().unwrap();
        assert!(plugin.active.is_none());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn starts_and_stops_indefinite_caffeinate() {
        let mut plugin = CaffeinatePlugin::default();
        assert_eq!(
            plugin.start("{}").unwrap().as_deref(),
            Some("Keeping this Mac awake indefinitely")
        );
        assert!(
            plugin
                .active
                .as_mut()
                .unwrap()
                .child
                .try_wait()
                .unwrap()
                .is_none()
        );
        assert_eq!(plugin.presentation_items()[0].value, "Indefinitely");
        assert!(
            !plugin.presentation_items()[0]
                .metadata
                .contains_key("countdown_until")
        );
        plugin.stop().unwrap();
        assert!(plugin.active.is_none());
    }
}
