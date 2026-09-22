use chrono::{
    DateTime, Datelike, Days, Local, LocalResult, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone,
};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const PLUGIN_ID: &str = "io.clipkeep.cron-schedule";
const MAX_EXPRESSION_BYTES: usize = 256;
const RESULT_COUNT: usize = 5;
const HORIZON_DAYS: u64 = 366 * 8 + 2;
const MONTH_NAMES: [(&str, u32); 12] = [
    ("JAN", 1),
    ("FEB", 2),
    ("MAR", 3),
    ("APR", 4),
    ("MAY", 5),
    ("JUN", 6),
    ("JUL", 7),
    ("AUG", 8),
    ("SEP", 9),
    ("OCT", 10),
    ("NOV", 11),
    ("DEC", 12),
];
const WEEKDAY_NAMES: [(&str, u32); 7] = [
    ("SUN", 0),
    ("MON", 1),
    ("TUE", 2),
    ("WED", 3),
    ("THU", 4),
    ("FRI", 5),
    ("SAT", 6),
];

fn main() -> io::Result<()> {
    run_plugin(CronSchedulePlugin::default())
}

#[derive(Default)]
struct CronSchedulePlugin {
    active_values: BTreeMap<String, String>,
}

impl Plugin for CronSchedulePlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let schedule = match parse_schedule(&request.expression) {
            Ok(schedule) => schedule,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let now = Local::now();
        let occurrences = match next_local_occurrences(&schedule, now, RESULT_COUNT) {
            Ok(occurrences) => occurrences,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let preview = build_report(&request.expression, &schedule, &occurrences, now);
        Ok(occurrences
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, occurrence)| {
                let result_id = format!("cron-schedule:{index}");
                let value = occurrence.to_rfc3339_opts(SecondsFormat::Secs, true);
                self.active_values.insert(result_id.clone(), value.clone());
                occurrence_result(result_id, index, occurrence, &schedule.expression, &preview)
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Cron Schedule action: {action_id}"));
        }
        let value = self
            .active_values
            .get(result_id)
            .ok_or_else(|| "cron occurrence is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Cron occurrence copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    expression: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    expression: String,
    allowed: BTreeSet<u32>,
    unrestricted: bool,
}

impl Field {
    fn contains(&self, value: u32) -> bool {
        self.allowed.contains(&value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Schedule {
    expression: String,
    minute: Field,
    hour: Field,
    day_of_month: Field,
    month: Field,
    day_of_week: Field,
}

#[derive(Clone, Copy)]
enum FieldKind {
    Minute,
    Hour,
    DayOfMonth,
    Month,
    DayOfWeek,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "cron" | "crontab") && command != "定时" {
        return Ok(None);
    }
    let arguments = query[command_end..]
        .strip_prefix(':')
        .unwrap_or(&query[command_end..])
        .trim();
    if arguments.is_empty() {
        return Err("expected a five-field cron expression, such as '*/15 9-17 * * 1-5'".into());
    }
    if arguments.len() > MAX_EXPRESSION_BYTES {
        return Err("cron expression exceeds 256 bytes".into());
    }
    Ok(Some(Request {
        expression: arguments.to_owned(),
    }))
}

fn parse_schedule(expression: &str) -> Result<Schedule, String> {
    if expression.eq_ignore_ascii_case("@reboot") {
        return Err("@reboot is event-based and has no upcoming wall-clock time to preview".into());
    }
    let normalized = expand_macro(expression).unwrap_or(expression);
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(
            "cron expressions must have exactly 5 fields: minute hour day month weekday".into(),
        );
    }
    Ok(Schedule {
        expression: parts.join(" "),
        minute: parse_field(parts[0], 0, 59, &[], false, "minute")?,
        hour: parse_field(parts[1], 0, 23, &[], false, "hour")?,
        day_of_month: parse_field(parts[2], 1, 31, &[], false, "day of month")?,
        month: parse_field(parts[3], 1, 12, &MONTH_NAMES, false, "month")?,
        day_of_week: parse_field(parts[4], 0, 7, &WEEKDAY_NAMES, true, "day of week")?,
    })
}

fn expand_macro(expression: &str) -> Option<&'static str> {
    match expression.to_ascii_lowercase().as_str() {
        "@hourly" => Some("0 * * * *"),
        "@daily" | "@midnight" => Some("0 0 * * *"),
        "@weekly" => Some("0 0 * * 0"),
        "@monthly" => Some("0 0 1 * *"),
        "@yearly" | "@annually" => Some("0 0 1 1 *"),
        _ => None,
    }
}

fn parse_field(
    expression: &str,
    minimum: u32,
    maximum: u32,
    names: &[(&str, u32)],
    sunday_alias: bool,
    label: &str,
) -> Result<Field, String> {
    if expression.is_empty() {
        return Err(format!("{label} field is empty"));
    }
    let mut allowed = BTreeSet::new();
    for segment in expression.split(',') {
        if segment.is_empty() {
            return Err(format!("{label} field contains an empty list item"));
        }
        let slash_parts: Vec<&str> = segment.split('/').collect();
        if slash_parts.len() > 2 {
            return Err(format!("{label} field has more than one step separator"));
        }
        let base = slash_parts[0];
        let step = if slash_parts.len() == 2 {
            slash_parts[1]
                .parse::<u32>()
                .ok()
                .filter(|step| *step != 0)
                .ok_or_else(|| format!("{label} step must be a positive integer"))?
        } else {
            1
        };
        let (start, end) = if base == "*" {
            (minimum, maximum)
        } else if let Some((start, end)) = split_range(base, label)? {
            (
                parse_field_value(start, minimum, maximum, names, label)?,
                parse_field_value(end, minimum, maximum, names, label)?,
            )
        } else {
            if slash_parts.len() == 2 {
                return Err(format!("{label} step requires '*' or a range"));
            }
            let value = parse_field_value(base, minimum, maximum, names, label)?;
            (value, value)
        };
        if start > end {
            return Err(format!("{label} range start must not exceed its end"));
        }
        for value in (start..=end).step_by(step as usize) {
            allowed.insert(normalize_value(value, sunday_alias));
        }
    }
    Ok(Field {
        expression: expression.to_ascii_uppercase(),
        unrestricted: expression == "*"
            || (expression.starts_with("*/") && !expression.contains(',')),
        allowed,
    })
}

fn split_range<'a>(value: &'a str, label: &str) -> Result<Option<(&'a str, &'a str)>, String> {
    let parts: Vec<&str> = value.split('-').collect();
    match parts.as_slice() {
        [_] => Ok(None),
        [start, end] if !start.is_empty() && !end.is_empty() => Ok(Some((start, end))),
        _ => Err(format!("{label} field contains an invalid range")),
    }
}

fn parse_field_value(
    value: &str,
    minimum: u32,
    maximum: u32,
    names: &[(&str, u32)],
    label: &str,
) -> Result<u32, String> {
    let uppercase = value.to_ascii_uppercase();
    let parsed = names
        .iter()
        .find_map(|(name, number)| (*name == uppercase).then_some(*number))
        .or_else(|| value.parse::<u32>().ok())
        .ok_or_else(|| format!("invalid {label} value '{value}'"))?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(format!(
            "{label} value {parsed} is outside {minimum} through {maximum}"
        ));
    }
    Ok(parsed)
}

fn normalize_value(value: u32, sunday_alias: bool) -> u32 {
    if sunday_alias && value == 7 { 0 } else { value }
}

fn matches_date(schedule: &Schedule, date: NaiveDate) -> bool {
    if !schedule.month.contains(date.month()) {
        return false;
    }
    let month_day_matches = schedule.day_of_month.contains(date.day());
    let weekday_matches = schedule
        .day_of_week
        .contains(date.weekday().num_days_from_sunday());
    match (
        schedule.day_of_month.unrestricted,
        schedule.day_of_week.unrestricted,
    ) {
        (true, _) | (_, true) => month_day_matches && weekday_matches,
        (false, false) => month_day_matches || weekday_matches,
    }
}

fn matching_times(schedule: &Schedule, date: NaiveDate) -> Vec<NaiveDateTime> {
    let mut values = Vec::new();
    for hour in &schedule.hour.allowed {
        for minute in &schedule.minute.allowed {
            if let Some(value) = date.and_hms_opt(*hour, *minute, 0) {
                values.push(value);
            }
        }
    }
    values
}

#[cfg(test)]
fn next_naive_occurrences(
    schedule: &Schedule,
    after: NaiveDateTime,
    count: usize,
) -> Result<Vec<NaiveDateTime>, String> {
    let mut occurrences = Vec::new();
    for offset in 0..=HORIZON_DAYS {
        let date = after
            .date()
            .checked_add_days(Days::new(offset))
            .ok_or_else(|| "cron search date is out of range".to_owned())?;
        if !matches_date(schedule, date) {
            continue;
        }
        for candidate in matching_times(schedule, date) {
            if candidate > after {
                occurrences.push(candidate);
                if occurrences.len() == count {
                    return Ok(occurrences);
                }
            }
        }
    }
    Err(format!(
        "fewer than {count} matching runs were found within 8 years"
    ))
}

fn next_local_occurrences(
    schedule: &Schedule,
    after: DateTime<Local>,
    count: usize,
) -> Result<Vec<DateTime<Local>>, String> {
    let mut occurrences = Vec::new();
    for offset in 0..=HORIZON_DAYS {
        let date = after
            .date_naive()
            .checked_add_days(Days::new(offset))
            .ok_or_else(|| "cron search date is out of range".to_owned())?;
        if !matches_date(schedule, date) {
            continue;
        }
        let mut resolved_for_day = Vec::new();
        for candidate in matching_times(schedule, date) {
            let resolved = match Local.from_local_datetime(&candidate) {
                LocalResult::Single(value) => vec![value],
                LocalResult::Ambiguous(first, second) => vec![first, second],
                LocalResult::None => Vec::new(),
            };
            resolved_for_day.extend(resolved.into_iter().filter(|value| *value > after));
        }
        resolved_for_day.sort_by_key(DateTime::timestamp_millis);
        for occurrence in resolved_for_day {
            occurrences.push(occurrence);
            if occurrences.len() == count {
                return Ok(occurrences);
            }
        }
    }
    Err(format!(
        "fewer than {count} matching runs were found within 8 years"
    ))
}

fn build_report(
    source: &str,
    schedule: &Schedule,
    occurrences: &[DateTime<Local>],
    now: DateTime<Local>,
) -> String {
    let expanded = if expand_macro(source).is_some() {
        format!("\nExpanded: {}", schedule.expression)
    } else {
        String::new()
    };
    let day_rule = match (
        schedule.day_of_month.unrestricted,
        schedule.day_of_week.unrestricted,
    ) {
        (false, false) => "\nDay rule: day-of-month OR day-of-week (standard cron semantics)",
        (true, true)
            if schedule.day_of_month.expression == "*"
                && schedule.day_of_week.expression == "*" =>
        {
            ""
        }
        _ => "\nDay rule: day-of-month AND day-of-week (one field uses * syntax)",
    };
    let next_runs = occurrences
        .iter()
        .enumerate()
        .map(|(index, occurrence)| {
            format!(
                "{}. {}",
                index + 1,
                occurrence.to_rfc3339_opts(SecondsFormat::Secs, true)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Cron schedule\nExpression: {source}{expanded}\nTime zone: {}\nDST rule: nonexistent local times are skipped; repeated local times run twice\n\nMinute: {}\nHour: {}\nDay of month: {}\nMonth: {}\nDay of week: {}{day_rule}\n\nNext runs:\n{next_runs}",
        timezone_label(now),
        field_summary(&schedule.minute, FieldKind::Minute),
        field_summary(&schedule.hour, FieldKind::Hour),
        field_summary(&schedule.day_of_month, FieldKind::DayOfMonth),
        field_summary(&schedule.month, FieldKind::Month),
        field_summary(&schedule.day_of_week, FieldKind::DayOfWeek),
    )
}

fn field_summary(field: &Field, kind: FieldKind) -> String {
    if let Some(step) = field.expression.strip_prefix("*/")
        && step.bytes().all(|byte| byte.is_ascii_digit())
    {
        let unit = match kind {
            FieldKind::Minute => "minutes within each hour, starting at 0",
            FieldKind::Hour => "hours within each day, starting at 0",
            FieldKind::DayOfMonth => "days within each month, starting at 1",
            FieldKind::Month => "months within each year, starting at JAN",
            FieldKind::DayOfWeek => "day-of-week values, starting at SUN",
        };
        return format!("every {step} {unit}");
    }
    if field.unrestricted {
        return match kind {
            FieldKind::Minute => "every minute",
            FieldKind::Hour => "every hour",
            FieldKind::DayOfMonth => "every day",
            FieldKind::Month => "every month",
            FieldKind::DayOfWeek => "every day of week",
        }
        .into();
    }
    compact_values(&field.allowed, kind)
}

fn timezone_label(now: DateTime<Local>) -> String {
    let name = now.format("%Z").to_string();
    let offset = now.format("%:z").to_string();
    if name.is_empty() || name == offset {
        offset
    } else {
        format!("{name} ({offset})")
    }
}

fn compact_values(values: &BTreeSet<u32>, kind: FieldKind) -> String {
    let values: Vec<u32> = values.iter().copied().collect();
    let mut groups = Vec::new();
    let mut index = 0;
    while index < values.len() {
        let start = values[index];
        let mut end = start;
        while index + 1 < values.len() && values[index + 1] == end + 1 {
            index += 1;
            end = values[index];
        }
        if start == end {
            groups.push(value_label(start, kind));
        } else {
            groups.push(format!(
                "{}–{}",
                value_label(start, kind),
                value_label(end, kind)
            ));
        }
        index += 1;
    }
    groups.join(", ")
}

fn value_label(value: u32, kind: FieldKind) -> String {
    match kind {
        FieldKind::Month => MONTH_NAMES
            .iter()
            .find_map(|(name, number)| (*number == value).then_some((*name).to_owned()))
            .unwrap_or_else(|| value.to_string()),
        FieldKind::DayOfWeek => WEEKDAY_NAMES
            .iter()
            .find_map(|(name, number)| (*number == value).then_some((*name).to_owned()))
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn occurrence_result(
    result_id: String,
    index: usize,
    occurrence: DateTime<Local>,
    expression: &str,
    preview: &str,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: occurrence.format("%a, %b %-d, %Y · %H:%M %Z").to_string(),
        subtitle: format!("Run {} of {RESULT_COUNT} · {expression}", index + 1),
        preview: preview.into(),
        kind: ResultKind::Command,
        score: 3_394.0 - index as f32,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "calendar.badge.clock".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Timestamp", true, true),
            action("copy", "Copy Timestamp", false, false),
        ],
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "cron-schedule:invalid".into(),
        title: "Cron Schedule".into(),
        subtitle: format!("Invalid cron expression · {message}"),
        preview: format!("Invalid cron expression\n{message}"),
        kind: ResultKind::Command,
        score: 3_394.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn action(id: &str, title: &str, default: bool, paste_to_previous: bool) -> ResultAction {
    ResultAction {
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window: paste_to_previous,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn parses_explicit_commands_without_stealing_other_queries() {
        assert_eq!(
            parse_request("cron */15 9-17 * * 1-5")
                .unwrap()
                .unwrap()
                .expression,
            "*/15 9-17 * * 1-5"
        );
        assert_eq!(
            parse_request("crontab:@daily").unwrap().unwrap().expression,
            "@daily"
        );
        assert_eq!(parse_request("calendar tomorrow").unwrap(), None);
        assert!(parse_request("cron").is_err());
    }

    #[test]
    fn parses_lists_ranges_steps_names_and_macros() {
        let schedule = parse_schedule("*/15 9-17 * JAN,MAR MON-FRI").unwrap();
        assert_eq!(schedule.minute.allowed, BTreeSet::from([0, 15, 30, 45]));
        assert!(schedule.hour.contains(9));
        assert!(schedule.hour.contains(17));
        assert_eq!(schedule.month.allowed, BTreeSet::from([1, 3]));
        assert_eq!(
            schedule.day_of_week.allowed,
            BTreeSet::from([1, 2, 3, 4, 5])
        );
        assert_eq!(parse_schedule("@daily").unwrap().expression, "0 0 * * *");
    }

    #[test]
    fn sunday_zero_and_seven_are_equivalent() {
        let zero = parse_schedule("0 0 * * 0").unwrap().day_of_week;
        let seven = parse_schedule("0 0 * * 7").unwrap().day_of_week;
        assert_eq!(zero.allowed, seven.allowed);
        assert_eq!(zero.unrestricted, seven.unrestricted);
        assert_eq!(
            parse_schedule("0 0 * * SUN").unwrap().day_of_week.allowed,
            BTreeSet::from([0])
        );
    }

    #[test]
    fn rejects_invalid_field_shapes_and_bounds() {
        assert!(parse_schedule("0 0 * *").is_err());
        assert!(parse_schedule("60 0 * * *").is_err());
        assert!(parse_schedule("*/0 0 * * *").is_err());
        assert!(parse_schedule("10-5 0 * * *").is_err());
        assert!(parse_schedule("1//2 0 * * *").is_err());
        assert!(parse_schedule("1/2 0 * * *").is_err());
        assert!(parse_schedule("0 0 * FOO *").is_err());
        assert!(
            parse_schedule("@reboot")
                .unwrap_err()
                .contains("event-based")
        );
    }

    #[test]
    fn restricted_month_day_and_weekday_use_standard_or_semantics() {
        let schedule = parse_schedule("0 0 13 * FRI").unwrap();
        assert!(matches_date(
            &schedule,
            NaiveDate::from_ymd_opt(2026, 2, 13).unwrap()
        ));
        assert!(matches_date(
            &schedule,
            NaiveDate::from_ymd_opt(2026, 4, 13).unwrap()
        ));
        assert!(matches_date(
            &schedule,
            NaiveDate::from_ymd_opt(2026, 3, 20).unwrap()
        ));
        assert!(!matches_date(
            &schedule,
            NaiveDate::from_ymd_opt(2026, 3, 19).unwrap()
        ));

        let syntactically_restricted = parse_schedule("0 0 1-31 * MON").unwrap();
        assert!(!syntactically_restricted.day_of_month.unrestricted);
        assert!(matches_date(
            &syntactically_restricted,
            NaiveDate::from_ymd_opt(2026, 3, 17).unwrap()
        ));

        let star_step = parse_schedule("0 0 */2 * MON").unwrap();
        assert!(star_step.day_of_month.unrestricted);
        assert!(matches_date(
            &star_step,
            NaiveDate::from_ymd_opt(2026, 3, 23).unwrap()
        ));
        assert!(!matches_date(
            &star_step,
            NaiveDate::from_ymd_opt(2026, 3, 16).unwrap()
        ));
    }

    #[test]
    fn calculates_repeated_intraday_occurrences() {
        let schedule = parse_schedule("*/15 9-10 * * MON-FRI").unwrap();
        let values = next_naive_occurrences(&schedule, at(2026, 9, 21, 9, 7), 5).unwrap();
        assert_eq!(
            values,
            vec![
                at(2026, 9, 21, 9, 15),
                at(2026, 9, 21, 9, 30),
                at(2026, 9, 21, 9, 45),
                at(2026, 9, 21, 10, 0),
                at(2026, 9, 21, 10, 15),
            ]
        );
    }

    #[test]
    fn crosses_weekends_and_finds_leap_days() {
        let weekday = parse_schedule("0 9 * * MON-FRI").unwrap();
        assert_eq!(
            next_naive_occurrences(&weekday, at(2026, 9, 25, 9, 0), 1).unwrap()[0],
            at(2026, 9, 28, 9, 0)
        );
        let leap_day = parse_schedule("0 0 29 FEB *").unwrap();
        assert_eq!(
            next_naive_occurrences(&leap_day, at(2025, 3, 1, 0, 0), 1).unwrap()[0],
            at(2028, 2, 29, 0, 0)
        );
    }

    #[test]
    fn field_summaries_are_compact_and_readable() {
        let schedule = parse_schedule("*/15 9-17 * JAN,MAR MON-FRI").unwrap();
        assert_eq!(
            field_summary(&schedule.minute, FieldKind::Minute),
            "every 15 minutes within each hour, starting at 0"
        );
        assert_eq!(field_summary(&schedule.hour, FieldKind::Hour), "9–17");
        assert_eq!(field_summary(&schedule.month, FieldKind::Month), "JAN, MAR");
        assert_eq!(
            field_summary(&schedule.day_of_week, FieldKind::DayOfWeek),
            "MON–FRI"
        );
    }

    #[test]
    fn results_are_bounded_actionable_and_invalidate_stale_values() {
        let mut plugin = CronSchedulePlugin::default();
        let results = plugin.search("cron @hourly", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.active_values.len(), 2);
        plugin.search("ordinary text", 10).unwrap();
        assert!(plugin.active_values.is_empty());
        assert!(
            plugin.search("cron 99 * * * *", 10).unwrap()[0]
                .actions
                .is_empty()
        );
    }

    #[test]
    fn request_size_is_bounded() {
        let oversized = format!("cron {}", "*".repeat(257));
        assert!(parse_request(&oversized).is_err());
    }
}
