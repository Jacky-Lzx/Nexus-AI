use chrono::{DateTime, Local};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{PluginNotification, ResultAction, ResultKind, SearchResult};
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
    due_at: i64,
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
        if reminder.due_at <= now_seconds() {
            return Err("reminder time must be in the future".into());
        }
        self.reminders.push(Reminder {
            id: self.next_id,
            content: content.into(),
            due_at: reminder.due_at,
            delivered: false,
            created_at: now_seconds(),
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
        subtitle: "Choose the reminder content and time".into(),
        preview: "Create a reminder".into(),
        kind: ResultKind::Command,
        score: 4_000.0,
        metadata: BTreeMap::from([
            ("input_form".into(), "reminder".into()),
            ("host_action.create".into(), "form.reminder".into()),
            ("symbol_name".into(), "bell.badge".into()),
        ]),
        actions: vec![ResultAction {
            id: "create".into(),
            title: "Create Reminder".into(),
            destructive: false,
            default: true,
            closes_window: false,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
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
            "due_at": now_seconds() + 3_600,
        });
        plugin
            .execute_with_input("reminder:new", "create", Some(&input.to_string()))
            .unwrap();
        let results = plugin.search("Alice", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Call Alice");
        let _ = fs::remove_file(plugin.storage_path);
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
