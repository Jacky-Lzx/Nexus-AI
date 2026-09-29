use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.number-statistics";
const RESULT_ID: &str = "number-statistics:result";
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_VALUES: usize = 10_000;

fn main() -> io::Result<()> {
    run_plugin(NumberStatisticsPlugin::default())
}

#[derive(Default)]
struct NumberStatisticsPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for NumberStatisticsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let input = match parse_request(query) {
            Ok(Some(input)) => input,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let values = match parse_values(input) {
            Ok(values) => values,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let statistics = calculate(&values);
        let report = build_report(&statistics);
        self.action_values
            .insert("paste_mean".into(), format_number(statistics.mean));
        self.action_values
            .insert("copy_mean".into(), format_number(statistics.mean));
        self.action_values
            .insert("copy_median".into(), format_number(statistics.median));
        self.action_values
            .insert("copy_report".into(), report.clone());
        Ok(vec![statistics_result(&statistics, report)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("number statistics result is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive Number Statistics action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_mean" | "copy_mean" => "Mean copied",
                "copy_median" => "Median copied",
                "copy_report" => "Statistics report copied",
                _ => "Statistics value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Statistics {
    count: usize,
    sum: Option<f64>,
    mean: f64,
    minimum: f64,
    q1: f64,
    median: f64,
    q3: f64,
    maximum: f64,
    range: Option<f64>,
    population_std_dev: Option<f64>,
    sample_std_dev: Option<f64>,
}

fn parse_request(query: &str) -> Result<Option<&str>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "stats" | "statistics" | "numstats"
    ) && command != "数据统计"
    {
        return Ok(None);
    }
    let remainder = &query[command_end..];
    let input = remainder.strip_prefix(':').unwrap_or(remainder).trim();
    if input.is_empty() {
        return Err("expected numbers separated by commas, semicolons, or whitespace".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("numeric dataset exceeds 64 KiB".into());
    }
    Ok(Some(input))
}

fn parse_values(input: &str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    for token in input
        .split(|character: char| character.is_whitespace() || character == ',' || character == ';')
    {
        if token.is_empty() {
            continue;
        }
        if values.len() == MAX_VALUES {
            return Err(format!("numeric dataset exceeds {MAX_VALUES} values"));
        }
        let value = token
            .parse::<f64>()
            .map_err(|_| format!("invalid number: {}", compact(&safe_text(token), 80)))?;
        if !value.is_finite() {
            return Err(format!("number must be finite: {}", safe_text(token)));
        }
        values.push(value);
    }
    if values.is_empty() {
        return Err("numeric dataset does not contain any values".into());
    }
    Ok(values)
}

fn calculate(values: &[f64]) -> Statistics {
    debug_assert!(!values.is_empty());
    let count = values.len();
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let scale = values
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    let mean = if scale == 0.0 {
        0.0
    } else {
        let scaled_sum = compensated_sum(values.iter().map(|value| *value / scale))
            .unwrap_or_else(|| values.iter().map(|value| *value / scale).sum());
        scale * (scaled_sum / count as f64)
    };
    let sum = compensated_sum(values.iter().copied());
    let minimum = sorted[0];
    let maximum = sorted[count - 1];
    let range = finite(maximum - minimum);
    let population_std_dev = standard_deviation(values, mean, count);
    let sample_std_dev = (count > 1)
        .then(|| standard_deviation(values, mean, count - 1))
        .flatten();
    Statistics {
        count,
        sum,
        mean,
        minimum,
        q1: percentile(&sorted, 0.25),
        median: percentile(&sorted, 0.5),
        q3: percentile(&sorted, 0.75),
        maximum,
        range,
        population_std_dev,
        sample_std_dev,
    }
}

fn compensated_sum(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut sum = 0.0_f64;
    let mut compensation = 0.0_f64;
    for value in values {
        let next = sum + value;
        if !next.is_finite() {
            return None;
        }
        if sum.abs() >= value.abs() {
            compensation += (sum - next) + value;
        } else {
            compensation += (value - next) + sum;
        }
        if !compensation.is_finite() {
            return None;
        }
        sum = next;
    }
    finite(sum + compensation)
}

fn standard_deviation(values: &[f64], mean: f64, denominator: usize) -> Option<f64> {
    let deviations = values.iter().map(|value| value - mean).collect::<Vec<_>>();
    if deviations.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let scale = deviations
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    if scale == 0.0 {
        return Some(0.0);
    }
    let scaled_squares = compensated_sum(deviations.iter().map(|value| {
        let scaled = *value / scale;
        scaled * scaled
    }))?;
    finite(scale * (scaled_squares / denominator as f64).sqrt())
}

fn percentile(sorted: &[f64], probability: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = probability * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let fraction = rank - lower as f64;
        sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn build_report(statistics: &Statistics) -> String {
    [
        "Number statistics".into(),
        format!("Count: {}", statistics.count),
        format_optional("Sum", statistics.sum),
        format!("Mean: {}", format_number(statistics.mean)),
        format!("Minimum: {}", format_number(statistics.minimum)),
        format!("Q1: {}", format_number(statistics.q1)),
        format!("Median: {}", format_number(statistics.median)),
        format!("Q3: {}", format_number(statistics.q3)),
        format!("Maximum: {}", format_number(statistics.maximum)),
        format_optional("Range", statistics.range),
        format_optional(
            "Population standard deviation",
            statistics.population_std_dev,
        ),
        if statistics.count > 1 {
            format_optional("Sample standard deviation", statistics.sample_std_dev)
        } else {
            "Sample standard deviation: requires at least 2 values".into()
        },
        "Quartiles use linear interpolation at p × (n - 1).".into(),
    ]
    .join("\n")
}

fn format_optional(label: &str, value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{label}: {}", format_number(value)),
        None => format!("{label}: outside the finite f64 range"),
    }
}

fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let absolute = value.abs();
    if !(1e-9..1e12).contains(&absolute) {
        let formatted = format!("{value:.12e}");
        let (mantissa, exponent) = formatted
            .split_once('e')
            .expect("scientific notation contains an exponent");
        return format!("{}e{exponent}", trim_decimal(mantissa));
    }
    trim_decimal(&format!("{value:.12}"))
}

fn trim_decimal(value: &str) -> String {
    if !value.contains('.') {
        return value.into();
    }
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

fn statistics_result(statistics: &Statistics, report: String) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: format!("Mean {}", format_number(statistics.mean)),
        subtitle: format!(
            "{} values · median {} · range {}",
            statistics.count,
            format_number(statistics.median),
            statistics
                .range
                .map(format_number)
                .unwrap_or_else(|| "overflow".into())
        ),
        preview: report,
        kind: ResultKind::Command,
        score: 3_440.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "chart.bar.xaxis".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action("paste_mean", "Paste Mean", true, true, true),
            action("copy_mean", "Copy Mean", false, false, false),
            action("copy_median", "Copy Median", false, false, false),
            action("copy_report", "Copy Statistics Report", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "number-statistics:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Number statistics unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Number statistics unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_440.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "exclamationmark.triangle.fill".into()),
            ("symbol_color".into(), "#FF9F0A".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: Vec::new(),
    }
}

fn action(
    id: &str,
    title: &str,
    default: bool,
    closes_window: bool,
    paste_to_previous: bool,
) -> ResultAction {
    ResultAction {
        host_action: None,
        id: id.into(),
        title: title.into(),
        destructive: false,
        default,
        closes_window,
        paste_to_previous,
        input_fields: Vec::new(),
    }
}

fn safe_text(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_control()
            || matches!(
                character,
                '\u{061C}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
            )
        {
            output.push_str(&format!("\\u{{{:04X}}}", character as u32));
        } else {
            output.push(character);
        }
    }
    output
}

fn compact(value: &str, maximum_characters: usize) -> String {
    let mut characters = value.chars();
    let prefix: String = characters.by_ref().take(maximum_characters).collect();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f64, right: f64) {
        assert!((left - right).abs() < 1e-12, "{left} != {right}");
    }

    #[test]
    fn parses_explicit_commands_and_multiple_separators() {
        assert!(parse_request("ordinary 1 2 3").unwrap().is_none());
        assert_eq!(parse_request("stats: 1,2").unwrap(), Some("1,2"));
        assert_eq!(
            parse_values("1, 2;\n3\t-4.5e1").unwrap(),
            [1.0, 2.0, 3.0, -45.0]
        );
        assert!(parse_request("数据统计 1 2").unwrap().is_some());
    }

    #[test]
    fn rejects_missing_invalid_nonfinite_and_oversized_inputs() {
        assert!(parse_request("stats").is_err());
        assert!(parse_values(", ; ").is_err());
        assert!(parse_values("1 two 3").is_err());
        assert!(parse_values("NaN").is_err());
        assert!(parse_values("inf").is_err());
        let too_many = std::iter::repeat_n("1", MAX_VALUES + 1)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(parse_values(&too_many).is_err());
        assert!(parse_request(&format!("stats {}", "1".repeat(MAX_INPUT_BYTES + 1))).is_err());
    }

    #[test]
    fn calculates_basic_descriptive_statistics() {
        let statistics = calculate(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(statistics.count, 4);
        close(statistics.sum.unwrap(), 10.0);
        close(statistics.mean, 2.5);
        close(statistics.minimum, 1.0);
        close(statistics.q1, 1.75);
        close(statistics.median, 2.5);
        close(statistics.q3, 3.25);
        close(statistics.maximum, 4.0);
        close(statistics.range.unwrap(), 3.0);
    }

    #[test]
    fn calculates_population_and_sample_standard_deviation() {
        let statistics = calculate(&[1.0, 2.0, 3.0]);
        close(
            statistics.population_std_dev.unwrap(),
            (2.0_f64 / 3.0).sqrt(),
        );
        close(statistics.sample_std_dev.unwrap(), 1.0);
        let single = calculate(&[7.0]);
        close(single.population_std_dev.unwrap(), 0.0);
        assert_eq!(single.sample_std_dev, None);
    }

    #[test]
    fn compensated_sum_retains_small_residuals() {
        assert_eq!(compensated_sum([1e16, 1.0, -1e16]), Some(1.0));
        let statistics = calculate(&[1e16, 1.0, -1e16]);
        close(statistics.sum.unwrap(), 1.0);
    }

    #[test]
    fn calculates_finite_mean_when_the_raw_sum_overflows() {
        let statistics = calculate(&[1e308, 1e308]);
        assert_eq!(statistics.sum, None);
        assert_eq!(statistics.mean, 1e308);
        assert_eq!(statistics.population_std_dev, Some(0.0));
    }

    #[test]
    fn reports_unrepresentable_ranges_and_deviations_without_panicking() {
        let statistics = calculate(&[-1e308, 1e308]);
        assert_eq!(statistics.mean, 0.0);
        assert_eq!(statistics.range, None);
        assert!(statistics.population_std_dev.is_some());
        let report = build_report(&statistics);
        assert!(report.contains("Range: outside the finite f64 range"));
    }

    #[test]
    fn formats_regular_small_large_and_negative_zero_values() {
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1e-10), "1e-10");
        assert_eq!(format_number(1e12), "1e12");
    }

    #[test]
    fn report_documents_quartile_method_and_sample_requirement() {
        let report = build_report(&calculate(&[5.0]));
        assert!(report.contains("Q1: 5"));
        assert!(report.contains("requires at least 2 values"));
        assert!(report.contains("p × (n - 1)"));
    }

    #[test]
    fn results_are_actionable_safe_and_invalidate_stale_values() {
        let mut plugin = NumberStatisticsPlugin::default();
        let result = plugin.search("stats 1, 2, 3", 1).unwrap().remove(0);
        assert_eq!(result.id, RESULT_ID);
        assert_eq!(result.actions.len(), 4);
        assert!(result.actions[0].default);
        assert_eq!(plugin.action_values["copy_mean"], "2");
        let invalid = plugin.search("stats 1 nope", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        let unsafe_result = plugin.search("stats 1\u{202E}", 1).unwrap().remove(0);
        assert!(!unsafe_result.title.contains('\u{202E}'));
        assert!(unsafe_result.title.contains("\\u{202E}"));
        assert!(plugin.search("stats 1", 0).unwrap().is_empty());
    }
}
