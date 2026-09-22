use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.base-converter";
const LIMB_BASE: u64 = 1_000_000_000;
const MAX_ARGUMENT_BYTES: usize = 8 * 1024;
const MAX_INPUT_DIGITS: usize = 2_048;
const MAX_TITLE_CHARACTERS: usize = 120;
const MAX_REPORT_VALUE_CHARACTERS: usize = 2_048;
const STANDARD_BASES: [u32; 5] = [2, 8, 10, 16, 36];

fn main() -> io::Result<()> {
    run_plugin(BaseConverterPlugin::default())
}

#[derive(Default)]
struct BaseConverterPlugin {
    active_values: BTreeMap<String, ConvertedValue>,
}

impl Plugin for BaseConverterPlugin {
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
        let evaluated = match evaluate(request) {
            Ok(evaluated) => evaluated,
            Err(message) => return Ok(vec![invalid_result(&message)]),
        };
        let report = build_report(&evaluated);
        let mut results = Vec::new();
        for converted in evaluated.outputs.into_iter().take(limit) {
            let result_id = format!("base-converter:{}", converted.base);
            self.active_values
                .insert(result_id.clone(), converted.clone());
            results.push(conversion_result(
                result_id,
                converted,
                evaluated.source_base,
                evaluated.input_digits,
                &report,
            ));
        }
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let converted = self
            .active_values
            .get(result_id)
            .ok_or_else(|| "base conversion is no longer active".to_owned())?;
        let value = match action_id {
            "paste" | "copy" => &converted.formatted,
            "copy_digits" => &converted.signed_digits,
            _ => return Err(format!("unknown Base Converter action: {action_id}")),
        };
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(format!("{} value copied", base_label(converted.base))))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    value: String,
    source_base: Option<u32>,
    target_base: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedNumber {
    negative: bool,
    magnitude: BigUnsigned,
    source_base: u32,
    input_digits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Evaluation {
    source_base: u32,
    input_digits: usize,
    report_values: Vec<ConvertedValue>,
    outputs: Vec<ConvertedValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConvertedValue {
    base: u32,
    signed_digits: String,
    formatted: String,
    grouped: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BigUnsigned {
    limbs: Vec<u32>,
}

impl BigUnsigned {
    fn from_digits(digits: &str, radix: u32) -> Result<Self, String> {
        let mut value = Self::default();
        for character in digits.chars() {
            let digit = character.to_digit(36).ok_or_else(|| {
                format!(
                    "invalid digit '{}' for base {radix}",
                    character.escape_default()
                )
            })?;
            if digit >= radix {
                return Err(format!(
                    "digit '{}' is not valid in base {radix}",
                    character.escape_default()
                ));
            }
            value.multiply_add(radix, digit);
        }
        Ok(value)
    }

    fn multiply_add(&mut self, multiplier: u32, addend: u32) {
        let mut carry = u64::from(addend);
        for limb in &mut self.limbs {
            let expanded = u64::from(*limb) * u64::from(multiplier) + carry;
            *limb = (expanded % LIMB_BASE) as u32;
            carry = expanded / LIMB_BASE;
        }
        while carry != 0 {
            self.limbs.push((carry % LIMB_BASE) as u32);
            carry /= LIMB_BASE;
        }
        self.trim();
    }

    fn to_radix(&self, radix: u32) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let mut working = self.clone();
        let mut digits = Vec::new();
        while !working.is_zero() {
            let remainder = working.divide_small(radix);
            digits.push(digit_character(remainder));
        }
        digits.iter().rev().collect()
    }

    fn divide_small(&mut self, divisor: u32) -> u32 {
        let mut remainder = 0_u64;
        for limb in self.limbs.iter_mut().rev() {
            let value = remainder * LIMB_BASE + u64::from(*limb);
            *limb = (value / u64::from(divisor)) as u32;
            remainder = value % u64::from(divisor);
        }
        self.trim();
        remainder as u32
    }

    fn trim(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(command.to_ascii_lowercase().as_str(), "base" | "radix") && command != "进制" {
        return Ok(None);
    }
    let arguments = query[command_end..]
        .strip_prefix(':')
        .unwrap_or(&query[command_end..])
        .trim();
    if arguments.is_empty() {
        return Err("expected an integer after 'base', 'radix', or '进制'".into());
    }
    if arguments.len() > MAX_ARGUMENT_BYTES {
        return Err("base conversion input exceeds 8 KiB".into());
    }
    let tokens: Vec<&str> = arguments.split_whitespace().collect();
    let value = tokens[0].to_owned();
    let mut source_base = None;
    let mut target_base = None;
    let mut index = 1;
    if tokens
        .get(index)
        .is_some_and(|token| token.eq_ignore_ascii_case("from"))
    {
        let radix = tokens
            .get(index + 1)
            .ok_or_else(|| "expected a source base after 'from'".to_owned())?;
        source_base = Some(parse_radix(radix)?);
        index += 2;
    }
    if tokens
        .get(index)
        .is_some_and(|token| token.eq_ignore_ascii_case("to"))
    {
        let radix = tokens
            .get(index + 1)
            .ok_or_else(|| "expected a target base after 'to'".to_owned())?;
        target_base = Some(parse_radix(radix)?);
        index += 2;
    }
    if index != tokens.len() {
        return Err("expected '<value> [from <base>] [to <base>]'".into());
    }
    Ok(Some(Request {
        value,
        source_base,
        target_base,
    }))
}

fn parse_radix(value: &str) -> Result<u32, String> {
    let normalized = value.to_ascii_lowercase();
    let radix = match normalized.as_str() {
        "bin" | "binary" => 2,
        "oct" | "octal" => 8,
        "dec" | "decimal" => 10,
        "hex" | "hexadecimal" => 16,
        _ => normalized
            .strip_prefix("base")
            .unwrap_or(&normalized)
            .parse::<u32>()
            .map_err(|_| format!("invalid base '{value}'"))?,
    };
    if !(2..=36).contains(&radix) {
        return Err("base must be between 2 and 36".into());
    }
    Ok(radix)
}

fn evaluate(request: Request) -> Result<Evaluation, String> {
    let parsed = parse_number(&request.value, request.source_base)?;
    let output_bases = request
        .target_base
        .map_or_else(|| STANDARD_BASES.to_vec(), |base| vec![base]);
    let mut report_bases = STANDARD_BASES.to_vec();
    for base in &output_bases {
        if !report_bases.contains(base) {
            report_bases.push(*base);
        }
    }
    let report_values = report_bases
        .into_iter()
        .map(|base| convert(&parsed, base))
        .collect::<Vec<_>>();
    let outputs = output_bases
        .iter()
        .map(|base| {
            report_values
                .iter()
                .find(|value| value.base == *base)
                .expect("every output base is included in the report")
                .clone()
        })
        .collect();
    Ok(Evaluation {
        source_base: parsed.source_base,
        input_digits: parsed.input_digits,
        report_values,
        outputs,
    })
}

fn parse_number(value: &str, explicit_base: Option<u32>) -> Result<ParsedNumber, String> {
    let (negative, unsigned) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else if let Some(value) = value.strip_prefix('+') {
        (false, value)
    } else {
        (false, value)
    };
    if unsigned.is_empty() {
        return Err("integer digits cannot be empty".into());
    }
    let (source_base, digits) = if let Some(base) = explicit_base {
        if unsigned.contains(':') {
            return Err("do not combine '<base>:<digits>' with 'from <base>'".into());
        }
        (base, unsigned)
    } else if let Some((base, digits)) = unsigned.split_once(':') {
        (parse_radix(base)?, digits)
    } else if let Some(digits) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (16, digits)
    } else if let Some(digits) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (2, digits)
    } else if let Some(digits) = unsigned
        .strip_prefix("0o")
        .or_else(|| unsigned.strip_prefix("0O"))
    {
        (8, digits)
    } else if let Some(digits) = unsigned
        .strip_prefix("0d")
        .or_else(|| unsigned.strip_prefix("0D"))
    {
        (10, digits)
    } else {
        (10, unsigned)
    };
    if digits.is_empty() {
        return Err("integer digits cannot be empty".into());
    }
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return Err("underscores must separate integer digits".into());
    }
    let compact_digits: String = digits
        .chars()
        .filter(|character| *character != '_')
        .collect();
    if compact_digits.chars().count() > MAX_INPUT_DIGITS {
        return Err(format!("integer exceeds {MAX_INPUT_DIGITS} digits"));
    }
    let magnitude = BigUnsigned::from_digits(&compact_digits, source_base)?;
    Ok(ParsedNumber {
        negative: negative && !magnitude.is_zero(),
        magnitude,
        source_base,
        input_digits: compact_digits.chars().count(),
    })
}

fn convert(number: &ParsedNumber, base: u32) -> ConvertedValue {
    let digits = number.magnitude.to_radix(base);
    let signed_digits = apply_sign(&digits, number.negative);
    let formatted = format_value(&digits, number.negative, base);
    let grouped = format_value(
        &group_digits(&digits, group_width(base)),
        number.negative,
        base,
    );
    ConvertedValue {
        base,
        signed_digits,
        formatted,
        grouped,
    }
}

fn format_value(digits: &str, negative: bool, base: u32) -> String {
    let prefix = match base {
        2 => "0b".into(),
        8 => "0o".into(),
        10 => String::new(),
        16 => "0x".into(),
        _ => format!("{base}:"),
    };
    format!("{}{}{}", if negative { "-" } else { "" }, prefix, digits)
}

fn apply_sign(digits: &str, negative: bool) -> String {
    if negative {
        format!("-{digits}")
    } else {
        digits.to_owned()
    }
}

fn group_digits(digits: &str, width: usize) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / width);
    let first = digits.len() % width;
    for (index, character) in digits.chars().enumerate() {
        if index != 0 && index % width == first {
            grouped.push('_');
        }
        grouped.push(character);
    }
    grouped
}

fn group_width(base: u32) -> usize {
    match base {
        2 | 16 => 4,
        8 | 10 => 3,
        _ => 4,
    }
}

fn digit_character(digit: u32) -> char {
    char::from_digit(digit, 36).expect("base conversion digit is always below 36")
}

fn build_report(evaluation: &Evaluation) -> String {
    let mut lines = vec![format!(
        "Input: base {} · {}",
        evaluation.source_base,
        digit_count_label(evaluation.input_digits)
    )];
    for value in &evaluation.report_values {
        lines.push(format!(
            "{}: {}",
            base_label(value.base),
            compact_middle(&value.grouped, MAX_REPORT_VALUE_CHARACTERS)
        ));
    }
    lines.join("\n")
}

fn conversion_result(
    result_id: String,
    converted: ConvertedValue,
    source_base: u32,
    input_digits: usize,
    report: &str,
) -> SearchResult {
    let mut actions = vec![
        action("paste", "Paste Value", true, true),
        action("copy", "Copy Value", false, false),
    ];
    if converted.base != 10 {
        actions.push(action("copy_digits", "Copy Digits Only", false, false));
    }
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: format!(
            "{} · {}",
            base_label(converted.base),
            compact_middle(&converted.formatted, MAX_TITLE_CHARACTERS)
        ),
        subtitle: format!(
            "From base {source_base} · {} · arbitrary precision",
            input_digit_count_label(input_digits)
        ),
        preview: report.into(),
        kind: ResultKind::Command,
        score: 3_398.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "number.square".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions,
    }
}

fn invalid_result(message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "base-converter:invalid".into(),
        title: "Base Converter".into(),
        subtitle: format!(
            "Invalid integer conversion · {}",
            compact_middle(message, 160)
        ),
        preview: format!("Invalid integer conversion\n{message}"),
        kind: ResultKind::Command,
        score: 3_398.0,
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

fn base_label(base: u32) -> String {
    match base {
        2 => "Binary".into(),
        8 => "Octal".into(),
        10 => "Decimal".into(),
        16 => "Hexadecimal".into(),
        36 => "Base 36".into(),
        _ => format!("Base {base}"),
    }
}

fn digit_count_label(count: usize) -> String {
    if count == 1 {
        "1 digit".into()
    } else {
        format!("{count} digits")
    }
}

fn input_digit_count_label(count: usize) -> String {
    if count == 1 {
        "1 input digit".into()
    } else {
        format!("{count} input digits")
    }
}

fn compact_middle(value: &str, maximum_characters: usize) -> String {
    let count = value.chars().count();
    if count <= maximum_characters {
        return value.to_owned();
    }
    let left_count = maximum_characters / 2;
    let right_count = maximum_characters - left_count;
    let left: String = value.chars().take(left_count).collect();
    let right: String = value
        .chars()
        .rev()
        .take(right_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{left}…{right}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_commands_and_ignores_other_queries() {
        assert_eq!(
            parse_request("base 0xff to binary").unwrap(),
            Some(Request {
                value: "0xff".into(),
                source_base: None,
                target_base: Some(2),
            })
        );
        assert_eq!(
            parse_request("进制 z from 36 to 10").unwrap(),
            Some(Request {
                value: "z".into(),
                source_base: Some(36),
                target_base: Some(10),
            })
        );
        assert_eq!(parse_request("ordinary 42").unwrap(), None);
        assert!(parse_request("radix").is_err());
    }

    #[test]
    fn parses_prefixes_signs_underscores_and_explicit_radices() {
        let hexadecimal = parse_number("-0xdead_beef", None).unwrap();
        assert!(hexadecimal.negative);
        assert_eq!(hexadecimal.source_base, 16);
        assert_eq!(hexadecimal.input_digits, 8);
        assert_eq!(hexadecimal.magnitude.to_radix(16), "deadbeef");

        let base_36 = parse_number("36:Hello", None).unwrap();
        assert_eq!(base_36.source_base, 36);
        assert_eq!(base_36.magnitude.to_radix(36), "hello");
        assert!(!parse_number("-0", None).unwrap().negative);
    }

    #[test]
    fn validates_radices_digits_and_expression_shape() {
        assert_eq!(parse_radix("base12").unwrap(), 12);
        assert!(parse_radix("1").is_err());
        assert!(parse_radix("37").is_err());
        assert!(parse_number("0b102", None).is_err());
        assert!(parse_number("12__34", None).is_err());
        assert!(parse_number("16:ff", Some(16)).is_err());
        assert!(parse_request("base 10 to 2 extra").is_err());
    }

    #[test]
    fn converts_values_larger_than_u128() {
        let value = parse_number("340282366920938463463374607431768211456", None).unwrap();
        assert_eq!(
            value.magnitude.to_radix(16),
            "100000000000000000000000000000000"
        );
        assert_eq!(value.magnitude.to_radix(2).len(), 129);
    }

    #[test]
    fn round_trips_large_base_36_values() {
        let digits = "z1a2b3c4d5e6f7g8h9i0".repeat(8);
        let value = parse_number(&format!("36:{digits}"), None).unwrap();
        assert_eq!(value.magnitude.to_radix(36), digits);
    }

    #[test]
    fn formats_standard_and_custom_outputs() {
        let number = parse_number("-255", None).unwrap();
        assert_eq!(convert(&number, 2).formatted, "-0b11111111");
        assert_eq!(convert(&number, 8).formatted, "-0o377");
        assert_eq!(convert(&number, 10).formatted, "-255");
        assert_eq!(convert(&number, 16).formatted, "-0xff");
        assert_eq!(convert(&number, 3).formatted, "-3:100110");
        assert_eq!(group_digits("123456789", 3), "123_456_789");
    }

    #[test]
    fn search_returns_target_or_standard_results_and_invalidates_old_values() {
        let mut plugin = BaseConverterPlugin::default();
        let targeted = plugin.search("base 0xff to 2", 20).unwrap();
        assert_eq!(targeted.len(), 1);
        assert_eq!(targeted[0].title, "Binary · 0b11111111");
        assert!(targeted[0].actions[0].default);
        assert!(targeted[0].actions[0].paste_to_previous);
        assert_eq!(
            plugin.active_values[&targeted[0].id].signed_digits,
            "11111111"
        );

        let standard = plugin.search("base 42", 3).unwrap();
        assert_eq!(standard.len(), 3);
        assert_eq!(standard[0].title, "Binary · 0b101010");
        assert_eq!(standard[1].title, "Octal · 0o52");
        assert_eq!(standard[2].title, "Decimal · 42");

        plugin.search("ordinary text", 20).unwrap();
        assert!(plugin.active_values.is_empty());
    }

    #[test]
    fn invalid_queries_are_visible_without_actions() {
        let mut plugin = BaseConverterPlugin::default();
        let result = plugin.search("base 0xnothex", 20).unwrap().remove(0);
        assert!(result.subtitle.starts_with("Invalid integer conversion · "));
        assert!(result.actions.is_empty());
        assert_eq!(result.metadata["symbol_color"], "#FF9F0A");
    }

    #[test]
    fn input_limits_and_middle_compaction_are_unicode_safe() {
        let oversized = "1".repeat(MAX_INPUT_DIGITS + 1);
        assert!(parse_number(&oversized, None).is_err());
        assert_eq!(compact_middle("甲乙丙丁戊己", 4), "甲乙…戊己");
        assert_eq!(digit_character(35), 'z');
        assert_eq!(digit_count_label(1), "1 digit");
        assert_eq!(input_digit_count_label(2), "2 input digits");
    }
}
