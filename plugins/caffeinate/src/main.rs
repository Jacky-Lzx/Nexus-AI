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
    ends_at: SystemTime,
}

#[derive(Deserialize)]
struct CaffeinateInput {
    #[serde(default)]
    hours: u64,
    #[serde(default)]
    minutes: u64,
    #[serde(default)]
    seconds: u64,
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
        if duration_seconds == 0 {
            return Err("caffeinate duration must be greater than zero".into());
        }
        self.refresh();
        self.stop()?;
        let child = Command::new("/usr/bin/caffeinate")
            .args([
                "-di",
                "-t",
                &duration_seconds.to_string(),
                "-w",
                &std::process::id().to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start caffeinate: {error}"))?;
        self.active = Some(ActiveCaffeination {
            child,
            ends_at: SystemTime::now() + Duration::from_secs(duration_seconds),
        });
        Ok(Some(format!(
            "Keeping this Mac awake for {}",
            format_duration(duration_seconds)
        )))
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
        let countdown_until = active
            .ends_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        vec![PluginPresentationItem {
            id: "caffeinate-status".into(),
            title: "Caffeinated".into(),
            value: format!("{} remaining", remaining_duration(active)),
            style: PresentationStyle::Success,
            metadata: BTreeMap::from([
                ("countdown_until".into(), countdown_until.to_string()),
                ("symbol_name".into(), "cup.and.saucer.fill".into()),
            ]),
        }]
    }
}

fn start_result(active: Option<&ActiveCaffeination>) -> SearchResult {
    let subtitle = active
        .map(|active| {
            format!(
                "Active · {} remaining · choose a new end time",
                remaining_duration(active)
            )
        })
        .unwrap_or_else(|| "Keep the display and Mac awake until a chosen time".into());
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
    let subtitle = format!("Cancel the current {} session", remaining_duration(active));
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

fn parse_duration(input: &str) -> Result<u64, String> {
    let input: CaffeinateInput =
        serde_json::from_str(input).map_err(|error| format!("invalid duration: {error}"))?;
    if input.hours > 8_760 || input.minutes > 59 || input.seconds > 59 {
        return Err("duration must use hours 0–8760 and minutes/seconds 0–59".into());
    }
    input
        .hours
        .checked_mul(3_600)
        .and_then(|hours| {
            input
                .minutes
                .checked_mul(60)
                .and_then(|minutes| hours.checked_add(minutes))
        })
        .and_then(|duration| duration.checked_add(input.seconds))
        .ok_or_else(|| "caffeinate duration is too large".into())
}

fn matches_query(query: &str, candidates: &[&str]) -> bool {
    query.is_empty() || candidates.iter().any(|candidate| candidate.contains(query))
}

fn remaining_duration(active: &ActiveCaffeination) -> String {
    format_duration(
        active
            .ends_at
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
            5_400
        );
        assert_eq!(parse_duration(r#"{"minutes":15}"#).unwrap(), 900);
        for invalid in [
            r#"{"hours":8761}"#,
            r#"{"minutes":60}"#,
            r#"{"hours":-1}"#,
            r#"{"minutes":1.5}"#,
            r#"{"hours":"one"}"#,
        ] {
            assert!(parse_duration(invalid).is_err(), "{invalid}");
        }
        assert!(CaffeinatePlugin::default().start("{}").is_err());
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
}
