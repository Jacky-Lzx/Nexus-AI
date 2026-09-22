use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.date-calculator";
const MAX_QUERY_BYTES: usize = 512;
const MAX_AMOUNT: i64 = 1_000_000;

fn main() -> io::Result<()> {
    run_plugin(DateCalculatorPlugin::default())
}

#[derive(Default)]
struct DateCalculatorPlugin {
    active_values: BTreeMap<String, String>,
}

impl Plugin for DateCalculatorPlugin {
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
        let today = Local::now().date_naive();
        let evaluations = match evaluate(request, today) {
            Ok(evaluations) => evaluations,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        Ok(evaluations
            .into_iter()
            .take(limit)
            .map(|evaluation| {
                self.active_values
                    .insert(evaluation.id.clone(), evaluation.copy_value.clone());
                result(evaluation)
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "paste" | "copy") {
            return Err(format!("unknown Date Calculator action: {action_id}"));
        }
        let value = self
            .active_values
            .get(result_id)
            .ok_or_else(|| "date calculation is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Date calculation copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DateInput {
    Fixed(NaiveDate),
    Today,
    Tomorrow,
    Yesterday,
}

impl DateInput {
    fn resolve(&self, today: NaiveDate) -> Result<NaiveDate, String> {
        match self {
            Self::Fixed(date) => Ok(*date),
            Self::Today => Ok(today),
            Self::Tomorrow => checked_add_days(today, 1),
            Self::Yesterday => checked_add_days(today, -1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Days,
    Weeks,
    Months,
    Years,
    BusinessDays,
}

impl Unit {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "d" | "day" | "days" => Some(Self::Days),
            "w" | "week" | "weeks" => Some(Self::Weeks),
            "mo" | "month" | "months" => Some(Self::Months),
            "y" | "year" | "years" => Some(Self::Years),
            "bd" | "business" | "business-day" | "business-days" | "weekday" | "weekdays" => {
                Some(Self::BusinessDays)
            }
            _ => None,
        }
    }

    fn label(self, amount: i64) -> String {
        let singular = amount.unsigned_abs() == 1;
        match self {
            Self::Days => {
                if singular {
                    "day"
                } else {
                    "days"
                }
            }
            Self::Weeks => {
                if singular {
                    "week"
                } else {
                    "weeks"
                }
            }
            Self::Months => {
                if singular {
                    "month"
                } else {
                    "months"
                }
            }
            Self::Years => {
                if singular {
                    "year"
                } else {
                    "years"
                }
            }
            Self::BusinessDays => {
                if singular {
                    "weekday"
                } else {
                    "weekdays"
                }
            }
        }
        .into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Request {
    Inspect(DateInput),
    Add {
        date: DateInput,
        amount: i64,
        unit: Unit,
    },
    Difference {
        start: DateInput,
        end: DateInput,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Evaluation {
    id: String,
    title: String,
    subtitle: String,
    preview: String,
    copy_value: String,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "datecalc" | "dcalc")
        && command != "日期计算"
    {
        return Ok(None);
    }
    let arguments = query[command_end..]
        .strip_prefix(':')
        .unwrap_or(&query[command_end..])
        .trim();
    if arguments.is_empty() {
        return Err("expected a date calculation after 'datecalc'".into());
    }
    if arguments.len() > MAX_QUERY_BYTES {
        return Err("date calculation exceeds 512 bytes".into());
    }
    let tokens: Vec<&str> = arguments.split_whitespace().collect();
    if tokens[0].eq_ignore_ascii_case("diff") {
        if tokens.len() != 3 {
            return Err("expected 'datecalc diff <start> <end>'".into());
        }
        return Ok(Some(Request::Difference {
            start: parse_date_input(tokens[1])?,
            end: parse_date_input(tokens[2])?,
        }));
    }
    let date = parse_date_input(tokens[0])?;
    if tokens.len() == 1 {
        return Ok(Some(Request::Inspect(date)));
    }
    if tokens.len() != 4 || !matches!(tokens[1], "+" | "-") {
        return Err("expected 'datecalc <date> <+|-> <amount> <unit>'".into());
    }
    let amount = tokens[2]
        .parse::<i64>()
        .map_err(|_| "date amount must be a non-negative integer".to_owned())?;
    if !(0..=MAX_AMOUNT).contains(&amount) {
        return Err(format!("date amount must be between 0 and {MAX_AMOUNT}"));
    }
    let signed_amount = if tokens[1] == "-" { -amount } else { amount };
    let unit = Unit::parse(tokens[3])
        .ok_or_else(|| "unit must be days, weeks, months, years, or business-days".to_owned())?;
    Ok(Some(Request::Add {
        date,
        amount: signed_amount,
        unit,
    }))
}

fn parse_date_input(value: &str) -> Result<DateInput, String> {
    match value.to_ascii_lowercase().as_str() {
        "today" => Ok(DateInput::Today),
        "tomorrow" => Ok(DateInput::Tomorrow),
        "yesterday" => Ok(DateInput::Yesterday),
        _ => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(DateInput::Fixed)
            .map_err(|_| {
                format!(
                    "invalid date '{value}'; expected YYYY-MM-DD, today, tomorrow, or yesterday"
                )
            }),
    }
}

fn evaluate(request: Request, today: NaiveDate) -> Result<Vec<Evaluation>, String> {
    match request {
        Request::Inspect(date) => Ok(vec![inspect_evaluation(date.resolve(today)?, today)?]),
        Request::Add { date, amount, unit } => {
            Ok(vec![add_evaluation(date.resolve(today)?, amount, unit)?])
        }
        Request::Difference { start, end } => {
            difference_evaluations(start.resolve(today)?, end.resolve(today)?)
        }
    }
}

fn inspect_evaluation(date: NaiveDate, today: NaiveDate) -> Result<Evaluation, String> {
    let difference = date.signed_duration_since(today).num_days();
    let iso_week = date.iso_week();
    Ok(Evaluation {
        id: "date-calculator:inspect".into(),
        title: format!("{} · {}", format_date(date), weekday_name(date.weekday())),
        subtitle: format!(
            "Day {} · ISO week {}-W{:02}",
            date.ordinal(),
            iso_week.year(),
            iso_week.week()
        ),
        preview: format!(
            "Date: {}\nWeekday: {}\nDay of year: {}\nISO week: {}-W{:02}\nDays from today: {difference}",
            format_date(date),
            weekday_name(date.weekday()),
            date.ordinal(),
            iso_week.year(),
            iso_week.week()
        ),
        copy_value: format_date(date),
    })
}

fn add_evaluation(start: NaiveDate, amount: i64, unit: Unit) -> Result<Evaluation, String> {
    let result = match unit {
        Unit::Days => checked_add_days(start, amount)?,
        Unit::Weeks => checked_add_days(
            start,
            amount
                .checked_mul(7)
                .ok_or_else(|| "week calculation is out of range".to_owned())?,
        )?,
        Unit::Months => add_months(start, amount)?,
        Unit::Years => add_months(
            start,
            amount
                .checked_mul(12)
                .ok_or_else(|| "year calculation is out of range".to_owned())?,
        )?,
        Unit::BusinessDays => add_business_days(start, amount)?,
    };
    let operator = if amount < 0 { "−" } else { "+" };
    let absolute = amount.unsigned_abs();
    let label = unit.label(amount);
    let adjusted = matches!(unit, Unit::Months | Unit::Years) && start.day() != result.day();
    let adjustment = if adjusted {
        "\nDay adjusted to the last valid day of the target month."
    } else {
        ""
    };
    let business_note = if unit == Unit::BusinessDays {
        "\nWeekdays exclude Saturday and Sunday; holidays are not excluded."
    } else {
        ""
    };
    Ok(Evaluation {
        id: "date-calculator:result".into(),
        title: format!(
            "{} · {}",
            format_date(result),
            weekday_name(result.weekday())
        ),
        subtitle: format!("{} {operator} {absolute} {label}", format_date(start)),
        preview: format!(
            "Start: {} ({})\nOperation: {operator} {absolute} {label}\nResult: {} ({}){adjustment}{business_note}",
            format_date(start),
            weekday_name(start.weekday()),
            format_date(result),
            weekday_name(result.weekday())
        ),
        copy_value: format_date(result),
    })
}

fn difference_evaluations(start: NaiveDate, end: NaiveDate) -> Result<Vec<Evaluation>, String> {
    let calendar_days = end.signed_duration_since(start).num_days();
    let weekdays = business_days_between(start, end)?;
    let preview = format!(
        "From: {} ({})\nTo: {} ({})\nCalendar days: {calendar_days}\nWeekdays crossed: {weekdays}\n\nThe weekday difference is signed and excludes Saturday and Sunday; holidays are not excluded.",
        format_date(start),
        weekday_name(start.weekday()),
        format_date(end),
        weekday_name(end.weekday())
    );
    Ok(vec![
        Evaluation {
            id: "date-calculator:calendar-difference".into(),
            title: format!(
                "Calendar difference · {}",
                signed_count(calendar_days, "day", "days")
            ),
            subtitle: format!("{} → {}", format_date(start), format_date(end)),
            preview: preview.clone(),
            copy_value: calendar_days.to_string(),
        },
        Evaluation {
            id: "date-calculator:weekday-difference".into(),
            title: format!(
                "Weekday difference · {}",
                signed_count(weekdays, "weekday", "weekdays")
            ),
            subtitle: "Weekend-only business-day difference".into(),
            preview,
            copy_value: weekdays.to_string(),
        },
    ])
}

fn checked_add_days(date: NaiveDate, days: i64) -> Result<NaiveDate, String> {
    let duration =
        Duration::try_days(days).ok_or_else(|| "day calculation is out of range".to_owned())?;
    date.checked_add_signed(duration)
        .ok_or_else(|| "resulting date is out of range".to_owned())
}

fn add_months(date: NaiveDate, months: i64) -> Result<NaiveDate, String> {
    let month_index = i64::from(date.year())
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(date.month0())))
        .and_then(|value| value.checked_add(months))
        .ok_or_else(|| "month calculation is out of range".to_owned())?;
    let year = i32::try_from(month_index.div_euclid(12))
        .map_err(|_| "resulting year is out of range".to_owned())?;
    let month = u32::try_from(month_index.rem_euclid(12) + 1)
        .map_err(|_| "resulting month is out of range".to_owned())?;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| "resulting date is out of range".to_owned())
}

fn add_business_days(mut date: NaiveDate, amount: i64) -> Result<NaiveDate, String> {
    if amount == 0 {
        return Ok(date);
    }
    let direction = amount.signum();
    let mut remaining = amount.unsigned_abs();
    if is_weekend(date.weekday()) {
        loop {
            date = checked_add_days(date, direction)?;
            if !is_weekend(date.weekday()) {
                remaining -= 1;
                break;
            }
        }
    }
    let weeks = remaining / 5;
    if weeks != 0 {
        let calendar_days = i64::try_from(weeks)
            .ok()
            .and_then(|value| value.checked_mul(7))
            .and_then(|value| value.checked_mul(direction))
            .ok_or_else(|| "business-day calculation is out of range".to_owned())?;
        date = checked_add_days(date, calendar_days)?;
        remaining %= 5;
    }
    while remaining != 0 {
        date = checked_add_days(date, direction)?;
        if !is_weekend(date.weekday()) {
            remaining -= 1;
        }
    }
    Ok(date)
}

fn business_days_between(start: NaiveDate, end: NaiveDate) -> Result<i64, String> {
    if end < start {
        return business_days_between(end, start)?
            .checked_neg()
            .ok_or_else(|| "weekday difference is out of range".to_owned());
    }
    let calendar_days = end.signed_duration_since(start).num_days();
    if calendar_days == 0 {
        return Ok(0);
    }
    let absolute_days = calendar_days.unsigned_abs();
    let full_weeks = absolute_days / 7;
    let mut weekdays = i64::try_from(full_weeks)
        .ok()
        .and_then(|value| value.checked_mul(5))
        .ok_or_else(|| "weekday difference is out of range".to_owned())?;
    let mut date = checked_add_days(
        start,
        i64::try_from(full_weeks)
            .ok()
            .and_then(|value| value.checked_mul(7))
            .ok_or_else(|| "weekday difference is out of range".to_owned())?,
    )?;
    for _ in 0..(absolute_days % 7) {
        date = checked_add_days(date, 1)?;
        if !is_weekend(date.weekday()) {
            weekdays += 1;
        }
    }
    Ok(weekdays)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => unreachable!("month is normalized to 1 through 12"),
    }
}

fn is_leap_year(year: i32) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

fn is_weekend(weekday: Weekday) -> bool {
    matches!(weekday, Weekday::Sat | Weekday::Sun)
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn signed_count(value: i64, singular: &str, plural: &str) -> String {
    format!(
        "{value} {}",
        if value.unsigned_abs() == 1 {
            singular
        } else {
            plural
        }
    )
}

fn format_date(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn result(evaluation: Evaluation) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: evaluation.id,
        title: evaluation.title,
        subtitle: evaluation.subtitle,
        preview: evaluation.preview,
        kind: ResultKind::Command,
        score: 3_396.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "calendar.badge.clock".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste", "Paste Result", true, true),
            action("copy", "Copy Result", false, false),
        ],
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "date-calculator:invalid".into(),
        title: "Date Calculator".into(),
        subtitle: format!("Invalid date calculation · {message}"),
        preview: format!("Invalid date calculation\n{message}"),
        kind: ResultKind::Command,
        score: 3_396.0,
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

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn parses_commands_without_stealing_world_clock_date_queries() {
        assert_eq!(parse_request("date Shanghai").unwrap(), None);
        assert_eq!(
            parse_request("datecalc 2026-09-23 + 3 days").unwrap(),
            Some(Request::Add {
                date: DateInput::Fixed(date(2026, 9, 23)),
                amount: 3,
                unit: Unit::Days,
            })
        );
        assert_eq!(
            parse_request("日期计算 today").unwrap(),
            Some(Request::Inspect(DateInput::Today))
        );
        assert!(parse_request("dcalc").is_err());
    }

    #[test]
    fn resolves_relative_dates_against_local_today() {
        let today = date(2026, 9, 23);
        assert_eq!(DateInput::Today.resolve(today).unwrap(), today);
        assert_eq!(
            DateInput::Tomorrow.resolve(today).unwrap(),
            date(2026, 9, 24)
        );
        assert_eq!(
            DateInput::Yesterday.resolve(today).unwrap(),
            date(2026, 9, 22)
        );
    }

    #[test]
    fn adds_days_and_weeks_in_both_directions() {
        let start = date(2026, 9, 23);
        assert_eq!(checked_add_days(start, 10).unwrap(), date(2026, 10, 3));
        assert_eq!(
            evaluate(
                Request::Add {
                    date: DateInput::Fixed(start),
                    amount: -2,
                    unit: Unit::Weeks
                },
                start
            )
            .unwrap()[0]
                .copy_value,
            "2026-09-09"
        );
    }

    #[test]
    fn clamps_months_and_years_to_valid_days() {
        assert_eq!(add_months(date(2026, 1, 31), 1).unwrap(), date(2026, 2, 28));
        assert_eq!(
            add_months(date(2024, 2, 29), 12).unwrap(),
            date(2025, 2, 28)
        );
        assert_eq!(
            add_months(date(2024, 3, 31), -1).unwrap(),
            date(2024, 2, 29)
        );
        let result = add_evaluation(date(2026, 1, 31), 1, Unit::Months).unwrap();
        assert!(result.preview.contains("last valid day"));
    }

    #[test]
    fn adds_weekdays_across_weekends_and_from_weekends() {
        assert_eq!(
            add_business_days(date(2026, 9, 25), 1).unwrap(),
            date(2026, 9, 28)
        );
        assert_eq!(
            add_business_days(date(2026, 9, 26), 1).unwrap(),
            date(2026, 9, 28)
        );
        assert_eq!(
            add_business_days(date(2026, 9, 27), -1).unwrap(),
            date(2026, 9, 25)
        );
        assert_eq!(
            add_business_days(date(2026, 9, 28), 5).unwrap(),
            date(2026, 10, 5)
        );
        assert_eq!(
            add_business_days(date(2026, 9, 28), -5).unwrap(),
            date(2026, 9, 21)
        );
    }

    #[test]
    fn computes_signed_calendar_and_weekday_differences() {
        let friday = date(2026, 9, 25);
        let monday = date(2026, 9, 28);
        assert_eq!(business_days_between(friday, monday).unwrap(), 1);
        assert_eq!(business_days_between(monday, friday).unwrap(), -1);
        let saturday = date(2026, 9, 26);
        assert_eq!(business_days_between(saturday, monday).unwrap(), 1);
        assert_eq!(business_days_between(monday, saturday).unwrap(), -1);
        let values = difference_evaluations(friday, monday).unwrap();
        assert_eq!(values[0].copy_value, "3");
        assert_eq!(values[1].copy_value, "1");
        assert!(values[1].preview.contains("difference is signed"));
    }

    #[test]
    fn inspects_weekday_ordinal_and_iso_week() {
        let inspected = inspect_evaluation(date(2026, 9, 23), date(2026, 9, 23)).unwrap();
        assert_eq!(inspected.title, "2026-09-23 · Wednesday");
        assert!(inspected.subtitle.contains("Day 266"));
        assert!(inspected.subtitle.contains("2026-W39"));
        assert!(inspected.preview.contains("Days from today: 0"));
    }

    #[test]
    fn results_have_safe_actions_and_invalidate_old_values() {
        let mut plugin = DateCalculatorPlugin::default();
        let result = plugin
            .search("datecalc 2026-09-23 + 1 day", 20)
            .unwrap()
            .remove(0);
        assert_eq!(result.title, "2026-09-24 · Thursday");
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(plugin.active_values[&result.id], "2026-09-24");
        plugin.search("ordinary text", 20).unwrap();
        assert!(plugin.active_values.is_empty());
    }

    #[test]
    fn invalid_queries_are_visible_without_actions() {
        let mut plugin = DateCalculatorPlugin::default();
        let invalid = plugin.search("datecalc 2026-02-30", 20).unwrap().remove(0);
        assert!(invalid.subtitle.starts_with("Invalid date calculation · "));
        assert!(invalid.actions.is_empty());
        assert_eq!(invalid.metadata["symbol_color"], "#FF9F0A");
        assert!(parse_request("datecalc today + 1000001 days").is_err());
    }

    #[test]
    fn leap_year_rules_cover_century_boundaries() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
    }
}
