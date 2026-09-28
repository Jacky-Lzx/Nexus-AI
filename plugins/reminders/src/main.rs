use chrono::{DateTime, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{
    ActionInputField, ActionInputKind, PluginNotification, ResultAction, ResultKind, SearchResult,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.reminders";

fn main() -> io::Result<()> {
    run_plugin(RemindersPlugin::new()?)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Reminder {
    id: u64,
    content: String,
    due_at: i64,
    delivered: bool,
    created_at: i64,
}

#[derive(Deserialize)]
struct NewReminder {
    content: String,
    when: String,
}

struct RemindersPlugin {
    reminders: Vec<Reminder>,
    next_id: u64,
    storage_path: PathBuf,
    last_notification_check: Instant,
}

impl RemindersPlugin {
    fn new() -> io::Result<Self> {
        let project_dirs = ProjectDirs::from("io", "clipkeep", "ClipKeep")
            .ok_or_else(|| io::Error::other("could not determine reminders data directory"))?;
        Ok(Self::from_path(
            project_dirs.data_dir().join("reminders.json"),
        ))
    }

    fn from_path(storage_path: PathBuf) -> Self {
        let reminders: Vec<Reminder> = fs::read(&storage_path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let next_id = reminders
            .iter()
            .map(|reminder| reminder.id)
            .max()
            .unwrap_or(0)
            + 1;
        Self {
            reminders,
            next_id,
            storage_path,
            last_notification_check: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = self.storage_path.with_extension("json.tmp");
        let data = serde_json::to_vec(&self.reminders).map_err(|error| error.to_string())?;
        fs::write(&temporary, data).map_err(|error| error.to_string())?;
        fs::rename(temporary, &self.storage_path).map_err(|error| error.to_string())
    }

    fn create(&mut self, input: &str) -> Result<Option<String>, String> {
        let reminder: NewReminder =
            serde_json::from_str(input).map_err(|error| format!("invalid reminder: {error}"))?;
        let content = reminder.content.trim();
        if content.is_empty() {
            return Err("reminder content cannot be empty".into());
        }
        let now = Local::now();
        let due_at = parse_due_time(&reminder.when, now)?;
        self.reminders.push(Reminder {
            id: self.next_id,
            content: content.into(),
            due_at,
            delivered: false,
            created_at: now.timestamp(),
        });
        self.next_id += 1;
        self.reminders.sort_by_key(|item| item.due_at);
        self.save()?;
        Ok(Some("Reminder created".into()))
    }
}

impl Plugin for RemindersPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        let mut results = Vec::new();
        if query.is_empty()
            || [
                "add reminder",
                "create reminder",
                "new reminder",
                "添加提醒",
                "新建提醒",
            ]
            .iter()
            .any(|term| term.contains(&query))
        {
            results.push(new_reminder_result());
        }
        results.extend(
            self.reminders
                .iter()
                .filter(|reminder| !reminder.delivered)
                .filter(|reminder| {
                    query.is_empty() || reminder.content.to_lowercase().contains(&query)
                })
                .take(limit.saturating_sub(results.len()))
                .map(reminder_result),
        );
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id == "reminder:new" && action_id == "create" {
            Err("new reminder details are required".into())
        } else {
            Err(format!("unknown reminder action: {action_id}"))
        }
    }

    fn execute_with_input(
        &mut self,
        result_id: &str,
        action_id: &str,
        input: Option<&str>,
    ) -> Result<Option<String>, String> {
        if result_id == "reminder:new" && action_id == "create" {
            return self.create(input.ok_or("new reminder details are required")?);
        }
        self.execute(result_id, action_id)
    }

    fn notifications(&mut self) -> Result<Vec<PluginNotification>, String> {
        if self.last_notification_check.elapsed() < Duration::from_secs(1) {
            return Ok(Vec::new());
        }
        self.last_notification_check = Instant::now();
        let now = now_seconds();
        let mut notifications = Vec::new();
        for reminder in self
            .reminders
            .iter_mut()
            .filter(|reminder| !reminder.delivered && reminder.due_at <= now)
        {
            reminder.delivered = true;
            notifications.push(PluginNotification {
                id: format!("reminder:{}", reminder.id),
                title: "Reminder".into(),
                body: reminder.content.clone(),
            });
        }
        if !notifications.is_empty() {
            self.save()?;
        }
        Ok(notifications)
    }
}

fn new_reminder_result() -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "reminder:new".into(),
        title: "Add Reminder".into(),
        subtitle: "Enter a message and when to be reminded".into(),
        preview:
            "Create a reminder: message, then when (15m, 2h, tomorrow 09:00, or YYYY-MM-DD HH:MM)"
                .into(),
        kind: ResultKind::Command,
        score: 4_000.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "bell.badge".into()),
            (
                "host_action.create".into(),
                "notifications.authorize".into(),
            ),
        ]),
        actions: vec![ResultAction {
            id: "create".into(),
            title: "Create Reminder".into(),
            destructive: false,
            default: true,
            closes_window: false,
            paste_to_previous: false,
            input_fields: vec![
                ActionInputField {
                    id: "content".into(),
                    label: "message".into(),
                    placeholder: "Message".into(),
                    kind: ActionInputKind::Text,
                    required: true,
                    minimum: None,
                    maximum: None,
                },
                ActionInputField {
                    id: "when".into(),
                    label: "when".into(),
                    placeholder: "15m / tomorrow 09:00".into(),
                    kind: ActionInputKind::Text,
                    required: true,
                    minimum: None,
                    maximum: None,
                },
            ],
        }],
    }
}

fn parse_due_time(input: &str, now: DateTime<Local>) -> Result<i64, String> {
    let value = input.trim();
    let invalid = || "Use 15m, 2h, tomorrow 09:00, or YYYY-MM-DD HH:MM".to_string();

    let relative = value
        .strip_suffix('m')
        .map(|count| (count, 60_i64))
        .or_else(|| value.strip_suffix('h').map(|count| (count, 3_600)))
        .or_else(|| value.strip_suffix('d').map(|count| (count, 86_400)));
    let due = if let Some((count, unit_seconds)) = relative {
        let seconds = count
            .parse::<i64>()
            .ok()
            .filter(|count| *count > 0)
            .and_then(|count| count.checked_mul(unit_seconds))
            .ok_or_else(invalid)?;
        now.timestamp().checked_add(seconds).ok_or_else(invalid)?
    } else {
        let lower = value.to_ascii_lowercase();
        let tomorrow_time = lower
            .strip_prefix("tomorrow ")
            .or_else(|| value.strip_prefix("明天 "));
        let naive = if let Some(time) = tomorrow_time {
            let time = NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| invalid())?;
            let date = now.date_naive().succ_opt().ok_or_else(invalid)?;
            date.and_time(time)
        } else {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|_| invalid())?
        };
        match Local.from_local_datetime(&naive) {
            LocalResult::Single(due) => due.timestamp(),
            LocalResult::Ambiguous(first, second) => [first, second]
                .into_iter()
                .map(|time| time.timestamp())
                .filter(|time| *time > now.timestamp())
                .min()
                .ok_or_else(|| "Reminder time must be in the future".to_string())?,
            LocalResult::None => return Err("That local time does not exist".into()),
        }
    };
    if due <= now.timestamp() {
        return Err("Reminder time must be in the future".into());
    }
    Ok(due)
}

fn reminder_result(reminder: &Reminder) -> SearchResult {
    let due = format_due_time(reminder.due_at);
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: format!("reminder:{}", reminder.id),
        title: reminder.content.clone(),
        subtitle: due.clone(),
        preview: format!("{}\n{}", reminder.content, due),
        kind: ResultKind::Other,
        score: 3_000.0 - ((reminder.due_at - now_seconds()).max(0) as f32 / 60.0),
        metadata: BTreeMap::from([
            ("due_at".into(), reminder.due_at.to_string()),
            ("symbol_name".into(), "bell".into()),
        ]),
        actions: Vec::new(),
    }
}

fn format_due_time(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|date| {
            date.with_timezone(&Local)
                .format("%a, %b %-d · %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|| "Unknown time".into())
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn test_plugin() -> RemindersPlugin {
        let path = std::env::temp_dir().join(format!(
            "nexus-reminders-{}-{}.json",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_file(&path);
        RemindersPlugin::from_path(path)
    }

    #[test]
    fn creates_and_searches_reminders() {
        let mut plugin = test_plugin();
        let input = serde_json::json!({
            "content": "Call Alice",
            "when": "2h",
        });
        plugin
            .execute_with_input("reminder:new", "create", Some(&input.to_string()))
            .unwrap();
        let results = plugin.search("Alice", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Call Alice");
        assert_eq!(
            plugin.reminders[0].due_at - plugin.reminders[0].created_at,
            7_200
        );
        let command = plugin.search("reminder", 10).unwrap().remove(0);
        assert_eq!(command.title, "Add Reminder");
        assert_eq!(command.actions[0].input_fields.len(), 2);
        assert_eq!(command.actions[0].input_fields[0].id, "content");
        assert_eq!(command.actions[0].input_fields[1].id, "when");
        assert_eq!(
            command.metadata["host_action.create"],
            "notifications.authorize"
        );
        let reloaded = RemindersPlugin::from_path(plugin.storage_path.clone());
        assert_eq!(reloaded.reminders[0].content, "Call Alice");
        let _ = fs::remove_file(plugin.storage_path);
    }

    #[test]
    fn parses_relative_and_local_times() {
        let now = Local::now();
        assert_eq!(parse_due_time("15m", now).unwrap(), now.timestamp() + 900);
        assert_eq!(parse_due_time("2h", now).unwrap(), now.timestamp() + 7_200);
        assert_eq!(parse_due_time("1d", now).unwrap(), now.timestamp() + 86_400);
        let tomorrow = now.date_naive().succ_opt().unwrap();
        let expected = Local
            .from_local_datetime(&tomorrow.and_hms_opt(9, 0, 0).unwrap())
            .earliest()
            .unwrap()
            .timestamp();
        assert_eq!(parse_due_time("明天 09:00", now).unwrap(), expected);
        assert_eq!(parse_due_time("tomorrow 09:00", now).unwrap(), expected);
        let future = Local
            .with_ymd_and_hms(2099, 1, 2, 10, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(parse_due_time("2099-01-02 10:30", now).unwrap(), future);
    }

    #[test]
    fn rejects_bad_or_past_times_without_saving() {
        let now = Local::now();
        for input in [
            "",
            "0m",
            "-2h",
            "999999999999999999999d",
            "next Tuesday",
            "明天 25:00",
        ] {
            assert!(parse_due_time(input, now).is_err(), "{input}");
        }
        assert!(parse_due_time("2000-01-01 09:00", now).is_err());
        let mut plugin = test_plugin();
        let input = serde_json::json!({"content": "Invalid", "when": "0m"});
        assert!(
            plugin
                .execute_with_input("reminder:new", "create", Some(&input.to_string()))
                .is_err()
        );
        assert!(plugin.reminders.is_empty());
        assert!(!plugin.storage_path.exists());
    }

    #[test]
    fn due_reminders_notify_only_once() {
        let mut plugin = test_plugin();
        plugin.reminders.push(Reminder {
            id: 1,
            content: "Done".into(),
            due_at: now_seconds() - 1,
            delivered: false,
            created_at: now_seconds() - 10,
        });
        plugin.last_notification_check =
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert_eq!(plugin.notifications().unwrap().len(), 1);
        plugin.last_notification_check =
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(plugin.notifications().unwrap().is_empty());
        let _ = fs::remove_file(plugin.storage_path);
    }
}
