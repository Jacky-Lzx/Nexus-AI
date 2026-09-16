use block2::RcBlock;
use chrono::{DateTime, Local};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use objc2::{AnyThread, rc::autoreleasepool, runtime::Bool};
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
use objc2_foundation::{NSDate, NSError};
use std::{
    collections::BTreeMap,
    io,
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

const PLUGIN_ID: &str = "io.clipkeep.calendar";

fn main() -> io::Result<()> {
    run_plugin(CalendarPlugin::new())
}

struct CalendarPlugin {
    store: objc2::rc::Retained<EKEventStore>,
    permission: Option<Receiver<bool>>,
    last_refresh: Instant,
    future_days: i64,
}

impl CalendarPlugin {
    fn new() -> Self {
        let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };
        Self {
            store,
            permission: None,
            last_refresh: Instant::now(),
            future_days: 14,
        }
    }

    fn authorization_status() -> EKAuthorizationStatus {
        unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) }
    }

    fn request_access(&mut self) {
        if self.permission.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = sender.send(granted.as_bool());
        });
        unsafe {
            self.store
                .requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
        }
        self.permission = Some(receiver);
    }

    fn permission_result(&self, denied: bool) -> SearchResult {
        let (title, subtitle, action) = if denied {
            (
                "Calendar Access Required",
                "Allow Nexus to read events in System Settings",
                Some(ResultAction {
                    id: "open-settings".into(),
                    title: "Open Privacy Settings".into(),
                    destructive: false,
                    default: true,
                    closes_window: true,
                    paste_to_previous: false,
                    input_fields: Vec::new(),
                }),
            )
        } else {
            (
                "Requesting Calendar Access…",
                "Choose Allow in the macOS permission prompt",
                None,
            )
        };
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: "calendar:permission".into(),
            title: title.into(),
            subtitle: subtitle.into(),
            preview: subtitle.into(),
            kind: ResultKind::Command,
            score: 3_000.0,
            metadata: BTreeMap::new(),
            actions: action.into_iter().collect(),
        }
    }

    fn events(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        autoreleasepool(|_| unsafe {
            let start = NSDate::dateWithTimeIntervalSinceNow(0.0);
            let end = NSDate::dateWithTimeIntervalSinceNow(self.future_days as f64 * 86_400.0);
            let predicate = self
                .store
                .predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            let events = self.store.eventsMatchingPredicate(&predicate);
            let query = query.trim().to_lowercase();
            let now = NSDate::dateWithTimeIntervalSinceNow(0.0).timeIntervalSince1970();
            let mut values = Vec::new();
            for index in 0..events.count() {
                let event = events.objectAtIndex(index);
                let title = event.title().to_string();
                let location = event
                    .location()
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                let notes = event
                    .notes()
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                let calendar = event
                    .calendar()
                    .map(|value| value.title().to_string())
                    .unwrap_or_default();
                let haystack = format!("{title} {location} {notes} {calendar}").to_lowercase();
                if !query.is_empty()
                    && !query.split_whitespace().all(|word| haystack.contains(word))
                {
                    continue;
                }
                let start_timestamp = event.startDate().timeIntervalSince1970();
                let end_timestamp = event.endDate().timeIntervalSince1970();
                let all_day = event.isAllDay();
                let id = event
                    .eventIdentifier()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| format!("{start_timestamp}:{title}"));
                let when = format_event_time(start_timestamp, end_timestamp, all_day);
                let mut metadata = BTreeMap::from([
                    ("calendar".into(), calendar.clone()),
                    ("starts".into(), when.clone()),
                    ("symbol_name".into(), "calendar".into()),
                ]);
                if !location.is_empty() {
                    metadata.insert("location".into(), location.clone());
                }
                let preview = [when.clone(), location.clone(), notes]
                    .into_iter()
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                values.push((
                    start_timestamp,
                    SearchResult {
                        plugin_id: PLUGIN_ID.into(),
                        id: format!("calendar:{id}"),
                        title,
                        subtitle: if location.is_empty() {
                            when
                        } else {
                            format!("{when} · {location}")
                        },
                        preview,
                        kind: ResultKind::Other,
                        score: 2_500.0 - ((start_timestamp - now).max(0.0) / 6_000.0) as f32,
                        metadata,
                        actions: vec![ResultAction {
                            id: "open-calendar".into(),
                            title: "Open Calendar".into(),
                            destructive: false,
                            default: true,
                            closes_window: true,
                            paste_to_previous: false,
                            input_fields: Vec::new(),
                        }],
                    },
                ));
            }
            values.sort_by(|left, right| left.0.total_cmp(&right.0));
            values
                .into_iter()
                .take(limit)
                .map(|(_, result)| result)
                .collect()
        })
    }
}

impl Plugin for CalendarPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(if self.permission.is_some() {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(60)
        })
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        match Self::authorization_status() {
            status if status == EKAuthorizationStatus::FullAccess => Ok(self.events(query, limit)),
            status if status == EKAuthorizationStatus::NotDetermined => {
                self.request_access();
                Ok(vec![self.permission_result(false)])
            }
            _ => Ok(vec![self.permission_result(true)]),
        }
    }

    fn execute(&mut self, _result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let status = match action_id {
            "open-calendar" => Command::new("/usr/bin/open")
                .args(["-a", "Calendar"])
                .status(),
            "open-settings" => Command::new("/usr/bin/open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars")
                .status(),
            _ => return Err(format!("unknown calendar action: {action_id}")),
        }
        .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("could not open calendar: {status}"));
        }
        Ok(Some("Calendar opened".into()))
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let days = settings
            .get("future_days")
            .and_then(|value| value.parse().ok())
            .unwrap_or(14);
        let days = days.clamp(1, 365);
        let changed = days != self.future_days;
        self.future_days = days;
        Ok(changed)
    }

    fn tick(&mut self) -> Result<bool, String> {
        if let Some(receiver) = &self.permission {
            match receiver.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    self.permission = None;
                    self.last_refresh = Instant::now();
                    return Ok(true);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.last_refresh.elapsed() >= Duration::from_secs(60) {
            self.last_refresh = Instant::now();
            return Ok(true);
        }
        Ok(false)
    }
}

fn format_event_time(start: f64, end: f64, all_day: bool) -> String {
    let start = DateTime::from_timestamp(start as i64, 0)
        .unwrap_or_default()
        .with_timezone(&Local);
    let end = DateTime::from_timestamp(end as i64, 0)
        .unwrap_or_default()
        .with_timezone(&Local);
    if all_day {
        return start.format("%a, %b %-d · All day").to_string();
    }
    if start.date_naive() == end.date_naive() {
        format!(
            "{}–{}",
            start.format("%a, %b %-d · %-I:%M %p"),
            end.format("%-I:%M %p")
        )
    } else {
        format!(
            "{} – {}",
            start.format("%b %-d, %-I:%M %p"),
            end.format("%b %-d, %-I:%M %p")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timed_and_all_day_events() {
        assert!(format_event_time(1_735_732_800.0, 1_735_736_400.0, false).contains('–'));
        assert!(format_event_time(1_735_732_800.0, 1_735_819_200.0, true).contains("All day"));
    }
}
