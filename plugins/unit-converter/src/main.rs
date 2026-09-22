use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, collections::HashMap, io};

const PLUGIN_ID: &str = "io.clipkeep.unit-converter";

fn main() -> io::Result<()> {
    run_plugin(UnitConverterPlugin::default())
}

#[derive(Default)]
struct UnitConverterPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for UnitConverterPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Ok(conversion) = convert(query) else {
            return Ok(Vec::new());
        };
        let output = format!(
            "{} {}",
            format_number(conversion.output),
            conversion.to.symbol
        );
        let input = format!(
            "{} {}",
            format_number(conversion.input),
            conversion.from.symbol
        );
        let result_id = format!("unit-converter:{input}:{output}");
        self.recent_values.insert(result_id.clone(), output.clone());
        Ok(vec![SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: result_id,
            title: output.clone(),
            subtitle: format!("{input} = {output}"),
            preview: format!("{input} = {output}"),
            kind: ResultKind::Command,
            score: 3_000.0,
            metadata: BTreeMap::from([
                ("result".into(), output),
                ("symbol_name".into(), "ruler".into()),
            ]),
            actions: vec![
                ResultAction {
                    id: "paste".into(),
                    title: "Paste Result".into(),
                    destructive: false,
                    default: true,
                    closes_window: true,
                    paste_to_previous: true,
                    input_fields: Vec::new(),
                },
                ResultAction {
                    id: "copy".into(),
                    title: "Copy Result".into(),
                    destructive: false,
                    default: false,
                    closes_window: false,
                    paste_to_previous: false,
                    input_fields: Vec::new(),
                },
            ],
        }])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown unit converter action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "conversion result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Conversion copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Length,
    Mass,
    Temperature,
    Time,
    Data,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Unit {
    symbol: &'static str,
    aliases: &'static [&'static str],
    category: Category,
    scale: f64,
    offset: f64,
}

const UNITS: &[Unit] = &[
    linear(
        "mm",
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
        Category::Length,
        0.001,
    ),
    linear(
        "cm",
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
        Category::Length,
        0.01,
    ),
    linear(
        "m",
        &["m", "meter", "meters", "metre", "metres"],
        Category::Length,
        1.0,
    ),
    linear(
        "km",
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
        Category::Length,
        1_000.0,
    ),
    linear("in", &["in", "inch", "inches"], Category::Length, 0.0254),
    linear("ft", &["ft", "foot", "feet"], Category::Length, 0.3048),
    linear("yd", &["yd", "yard", "yards"], Category::Length, 0.9144),
    linear("mi", &["mi", "mile", "miles"], Category::Length, 1_609.344),
    linear(
        "mg",
        &["mg", "milligram", "milligrams"],
        Category::Mass,
        0.001,
    ),
    linear("g", &["g", "gram", "grams"], Category::Mass, 1.0),
    linear(
        "kg",
        &["kg", "kilogram", "kilograms"],
        Category::Mass,
        1_000.0,
    ),
    linear(
        "oz",
        &["oz", "ounce", "ounces"],
        Category::Mass,
        28.349_523_125,
    ),
    linear(
        "lb",
        &["lb", "lbs", "pound", "pounds"],
        Category::Mass,
        453.592_37,
    ),
    Unit {
        symbol: "°C",
        aliases: &["c", "°c", "celsius"],
        category: Category::Temperature,
        scale: 1.0,
        offset: 0.0,
    },
    Unit {
        symbol: "°F",
        aliases: &["f", "°f", "fahrenheit"],
        category: Category::Temperature,
        scale: 5.0 / 9.0,
        offset: -160.0 / 9.0,
    },
    Unit {
        symbol: "K",
        aliases: &["k", "kelvin"],
        category: Category::Temperature,
        scale: 1.0,
        offset: -273.15,
    },
    linear(
        "ms",
        &["ms", "millisecond", "milliseconds"],
        Category::Time,
        0.001,
    ),
    linear("s", &["s", "sec", "second", "seconds"], Category::Time, 1.0),
    linear("min", &["min", "minute", "minutes"], Category::Time, 60.0),
    linear("h", &["h", "hr", "hour", "hours"], Category::Time, 3_600.0),
    linear("d", &["d", "day", "days"], Category::Time, 86_400.0),
    linear("B", &["b", "byte", "bytes"], Category::Data, 1.0),
    linear(
        "KB",
        &["kb", "kilobyte", "kilobytes"],
        Category::Data,
        1_000.0,
    ),
    linear(
        "MB",
        &["mb", "megabyte", "megabytes"],
        Category::Data,
        1_000_000.0,
    ),
    linear(
        "GB",
        &["gb", "gigabyte", "gigabytes"],
        Category::Data,
        1_000_000_000.0,
    ),
    linear(
        "TB",
        &["tb", "terabyte", "terabytes"],
        Category::Data,
        1_000_000_000_000.0,
    ),
    linear(
        "KiB",
        &["kib", "kibibyte", "kibibytes"],
        Category::Data,
        1_024.0,
    ),
    linear(
        "MiB",
        &["mib", "mebibyte", "mebibytes"],
        Category::Data,
        1_048_576.0,
    ),
    linear(
        "GiB",
        &["gib", "gibibyte", "gibibytes"],
        Category::Data,
        1_073_741_824.0,
    ),
    linear(
        "TiB",
        &["tib", "tebibyte", "tebibytes"],
        Category::Data,
        1_099_511_627_776.0,
    ),
];

const fn linear(
    symbol: &'static str,
    aliases: &'static [&'static str],
    category: Category,
    scale: f64,
) -> Unit {
    Unit {
        symbol,
        aliases,
        category,
        scale,
        offset: 0.0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Conversion {
    input: f64,
    output: f64,
    from: &'static Unit,
    to: &'static Unit,
}

fn convert(query: &str) -> Result<Conversion, String> {
    let query = query.trim();
    let query = query
        .strip_prefix("convert ")
        .or_else(|| query.strip_prefix("Convert "))
        .unwrap_or(query)
        .trim();
    let lowercase = query.to_lowercase();
    let separator = lowercase
        .find(" to ")
        .ok_or_else(|| "expected '<value> <unit> to <unit>'".to_owned())?;
    let (quantity, target) = query.split_at(separator);
    let target = target[4..].trim();
    let (input, source) = parse_quantity(quantity)?;
    let from = find_unit(source).ok_or_else(|| format!("unknown unit: {source}"))?;
    let to = find_unit(target).ok_or_else(|| format!("unknown unit: {target}"))?;
    if from.category != to.category {
        return Err("units belong to different categories".into());
    }
    let base = input * from.scale + from.offset;
    if from.category == Category::Temperature && base < -273.15 - 1e-10 {
        return Err("temperature is below absolute zero".into());
    }
    let output = (base - to.offset) / to.scale;
    if !input.is_finite() || !output.is_finite() {
        return Err("conversion is not finite".into());
    }
    Ok(Conversion {
        input,
        output,
        from,
        to,
    })
}

fn parse_quantity(value: &str) -> Result<(f64, &str), String> {
    let value = value.trim();
    let mut parsed = None;
    for (index, _) in value.char_indices().skip(1) {
        let number = value[..index].trim();
        let unit = value[index..].trim();
        if !unit.is_empty()
            && let Ok(number) = number.parse::<f64>()
        {
            parsed = Some((number, unit));
        }
    }
    parsed.ok_or_else(|| "expected a number followed by a unit".into())
}

fn find_unit(value: &str) -> Option<&'static Unit> {
    let value = value.trim().to_lowercase();
    UNITS
        .iter()
        .find(|unit| unit.aliases.contains(&value.as_str()))
}

fn format_number(value: f64) -> String {
    if value == 0.0 || value.abs() < 1e-12 {
        return "0".into();
    }
    if value.abs() >= 1e12 || value.abs() < 1e-9 {
        return format!("{value:.10e}").replace("e+", "e");
    }
    let value = format!("{value:.12}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_spaced_and_attached_length_units() {
        let spaced = convert("10 km to mi").unwrap();
        let attached = convert("10km to mi").unwrap();
        assert!((spaced.output - 6.213_711_922_373_339_5).abs() < 1e-12);
        assert_eq!(spaced, attached);
    }

    #[test]
    fn converts_temperature_with_offsets() {
        assert!((convert("72 f to c").unwrap().output - 22.222_222_222_2).abs() < 1e-9);
        assert_eq!(convert("0 c to k").unwrap().output, 273.15);
        assert!(convert("-274 c to k").is_err());
    }

    #[test]
    fn converts_time_mass_and_decimal_or_binary_data() {
        assert_eq!(convert("2 h to min").unwrap().output, 120.0);
        assert!((convert("1 lb to kg").unwrap().output - 0.453_592_37).abs() < 1e-12);
        assert_eq!(convert("1 gb to mb").unwrap().output, 1_000.0);
        assert_eq!(convert("1 gib to mib").unwrap().output, 1_024.0);
    }

    #[test]
    fn rejects_unknown_or_incompatible_units() {
        assert!(convert("10 km to kg").is_err());
        assert!(convert("10 parsecs to m").is_err());
        assert!(convert("10 km").is_err());
    }

    #[test]
    fn search_result_can_be_copied_or_pasted() {
        let mut plugin = UnitConverterPlugin::default();
        let result = plugin.search("10 km to mi", 20).unwrap().remove(0);
        assert_eq!(result.title, "6.213711922373 mi");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values[&result.id], result.title);
    }
}
