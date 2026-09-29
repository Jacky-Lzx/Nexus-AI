use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, collections::HashMap, io};

const PLUGIN_ID: &str = "io.clipkeep.color-tools";

fn main() -> io::Result<()> {
    run_plugin(ColorToolsPlugin::default())
}

#[derive(Default)]
struct ColorToolsPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for ColorToolsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.recent_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let query = query.trim();
        let Some(request) = parse_request(query) else {
            return Ok(Vec::new());
        };
        if request == ColorRequest::Hint {
            return Ok(vec![hint_result()]);
        }
        let ColorRequest::Convert(color) = request else {
            unreachable!()
        };
        let formats = color.formats();
        let preview = formats
            .iter()
            .map(|format| format!("{}  {}", format.label, format.value))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(formats
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, format)| {
                let result_id = format!("color-tools:{}", format.id);
                self.recent_values
                    .insert(result_id.clone(), format.value.clone());
                color_result(
                    query,
                    color,
                    format,
                    result_id,
                    preview.clone(),
                    3_300.0 - index as f32,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown color tools action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "color result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Color copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorRequest {
    Hint,
    Convert(Color),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Color {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ColorFormat {
    id: &'static str,
    label: &'static str,
    value: String,
}

impl Color {
    fn formats(self) -> [ColorFormat; 3] {
        let (hue, saturation, lightness) = self.hsl();
        let alpha = format_alpha(self.alpha);
        let hex = if self.alpha == u8::MAX {
            format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
        } else {
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                self.red, self.green, self.blue, self.alpha
            )
        };
        let rgb = if self.alpha == u8::MAX {
            format!("rgb({}, {}, {})", self.red, self.green, self.blue)
        } else {
            format!("rgba({}, {}, {}, {alpha})", self.red, self.green, self.blue)
        };
        let hsl = if self.alpha == u8::MAX {
            format!(
                "hsl({}, {}%, {}%)",
                format_component(hue),
                format_component(saturation),
                format_component(lightness)
            )
        } else {
            format!(
                "hsla({}, {}%, {}%, {alpha})",
                format_component(hue),
                format_component(saturation),
                format_component(lightness)
            )
        };
        [
            ColorFormat {
                id: "hex",
                label: "HEX",
                value: hex,
            },
            ColorFormat {
                id: "rgb",
                label: "RGB",
                value: rgb,
            },
            ColorFormat {
                id: "hsl",
                label: "HSL",
                value: hsl,
            },
        ]
    }

    fn hsl(self) -> (f64, f64, f64) {
        let red = f64::from(self.red) / 255.0;
        let green = f64::from(self.green) / 255.0;
        let blue = f64::from(self.blue) / 255.0;
        let maximum = red.max(green).max(blue);
        let minimum = red.min(green).min(blue);
        let delta = maximum - minimum;
        let lightness = (maximum + minimum) / 2.0;
        if delta == 0.0 {
            return (0.0, 0.0, lightness * 100.0);
        }
        let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
        let hue = if maximum == red {
            60.0 * ((green - blue) / delta).rem_euclid(6.0)
        } else if maximum == green {
            60.0 * ((blue - red) / delta + 2.0)
        } else {
            60.0 * ((red - green) / delta + 4.0)
        };
        (hue, saturation * 100.0, lightness * 100.0)
    }

    fn swatch(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

fn parse_request(query: &str) -> Option<ColorRequest> {
    if query.eq_ignore_ascii_case("color")
        || query.eq_ignore_ascii_case("colour")
        || matches!(query, "颜色" | "色彩")
    {
        return Some(ColorRequest::Hint);
    }
    let lowercase = query.to_ascii_lowercase();
    for prefix in ["color ", "colour ", "color:", "colour:"] {
        if lowercase.starts_with(prefix) {
            return parse_color(query[prefix.len()..].trim()).map(ColorRequest::Convert);
        }
    }
    for prefix in ["颜色 ", "色彩 ", "颜色:", "颜色：", "色彩:", "色彩："] {
        if let Some(value) = query.strip_prefix(prefix) {
            return parse_color(value.trim()).map(ColorRequest::Convert);
        }
    }
    (query.starts_with('#')
        || lowercase.starts_with("rgb(")
        || lowercase.starts_with("rgba(")
        || lowercase.starts_with("hsl(")
        || lowercase.starts_with("hsla("))
    .then(|| parse_color(query).map(ColorRequest::Convert))
    .flatten()
}

fn parse_color(input: &str) -> Option<Color> {
    if let Some(hex) = input.strip_prefix('#') {
        return parse_hex(hex);
    }
    if !input.contains('(') {
        return named_color(input);
    }
    let opening = input.find('(')?;
    if !input.ends_with(')') {
        return None;
    }
    let function = input[..opening].trim().to_ascii_lowercase();
    let arguments = &input[opening + 1..input.len() - 1];
    match function.as_str() {
        "rgb" | "rgba" => parse_rgb(arguments),
        "hsl" | "hsla" => parse_hsl(arguments),
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let expanded = match hex.len() {
        3 | 4 => hex
            .chars()
            .flat_map(|character| [character, character])
            .collect::<String>(),
        6 | 8 => hex.to_owned(),
        _ => return None,
    };
    let value = u32::from_str_radix(&expanded, 16).ok()?;
    match expanded.len() {
        6 => Some(Color {
            red: (value >> 16) as u8,
            green: (value >> 8) as u8,
            blue: value as u8,
            alpha: u8::MAX,
        }),
        8 => Some(Color {
            red: (value >> 24) as u8,
            green: (value >> 16) as u8,
            blue: (value >> 8) as u8,
            alpha: value as u8,
        }),
        _ => None,
    }
}

fn parse_rgb(arguments: &str) -> Option<Color> {
    let values = color_arguments(arguments);
    if !(3..=4).contains(&values.len()) {
        return None;
    }
    Some(Color {
        red: parse_rgb_channel(values[0])?,
        green: parse_rgb_channel(values[1])?,
        blue: parse_rgb_channel(values[2])?,
        alpha: values
            .get(3)
            .map_or(Some(u8::MAX), |value| parse_alpha(value))?,
    })
}

fn parse_hsl(arguments: &str) -> Option<Color> {
    let values = color_arguments(arguments);
    if !(3..=4).contains(&values.len()) {
        return None;
    }
    let hue = values[0]
        .strip_suffix("deg")
        .unwrap_or(values[0])
        .parse::<f64>()
        .ok()?
        .rem_euclid(360.0);
    let saturation = parse_percentage(values[1])? / 100.0;
    let lightness = parse_percentage(values[2])? / 100.0;
    let alpha = values
        .get(3)
        .map_or(Some(u8::MAX), |value| parse_alpha(value))?;
    Some(hsl_to_rgb(hue, saturation, lightness, alpha))
}

fn color_arguments(input: &str) -> Vec<&str> {
    input
        .split(|character: char| character == ',' || character == '/' || character.is_whitespace())
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_rgb_channel(value: &str) -> Option<u8> {
    if let Some(value) = value.strip_suffix('%') {
        let percentage = value.parse::<f64>().ok()?;
        return (0.0..=100.0)
            .contains(&percentage)
            .then(|| (percentage * 255.0 / 100.0).round() as u8);
    }
    value.parse::<u8>().ok()
}

fn parse_percentage(value: &str) -> Option<f64> {
    let percentage = value.strip_suffix('%')?.parse::<f64>().ok()?;
    (0.0..=100.0).contains(&percentage).then_some(percentage)
}

fn parse_alpha(value: &str) -> Option<u8> {
    if let Some(value) = value.strip_suffix('%') {
        let percentage = value.parse::<f64>().ok()?;
        return (0.0..=100.0)
            .contains(&percentage)
            .then(|| (percentage * 255.0 / 100.0).round() as u8);
    }
    let alpha = value.parse::<f64>().ok()?;
    (0.0..=1.0)
        .contains(&alpha)
        .then(|| (alpha * 255.0).round() as u8)
}

fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64, alpha: u8) -> Color {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let section = hue / 60.0;
    let secondary = chroma * (1.0 - (section.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match section.floor() as u8 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma / 2.0;
    Color {
        red: ((red + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
        green: ((green + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
        blue: ((blue + offset) * 255.0).round().clamp(0.0, 255.0) as u8,
        alpha,
    }
}

fn named_color(input: &str) -> Option<Color> {
    let (red, green, blue, alpha) = match input.trim().to_ascii_lowercase().as_str() {
        "black" => (0, 0, 0, 255),
        "silver" => (192, 192, 192, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "white" => (255, 255, 255, 255),
        "maroon" => (128, 0, 0, 255),
        "red" => (255, 0, 0, 255),
        "purple" => (128, 0, 128, 255),
        "fuchsia" | "magenta" => (255, 0, 255, 255),
        "green" => (0, 128, 0, 255),
        "lime" => (0, 255, 0, 255),
        "olive" => (128, 128, 0, 255),
        "yellow" => (255, 255, 0, 255),
        "navy" => (0, 0, 128, 255),
        "blue" => (0, 0, 255, 255),
        "teal" => (0, 128, 128, 255),
        "aqua" | "cyan" => (0, 255, 255, 255),
        "orange" => (255, 165, 0, 255),
        "rebeccapurple" => (102, 51, 153, 255),
        "transparent" => (0, 0, 0, 0),
        _ => return None,
    };
    Some(Color {
        red,
        green,
        blue,
        alpha,
    })
}

fn format_component(value: f64) -> String {
    let rounded = value.round();
    if (value - rounded).abs() < 0.05 {
        format!("{rounded:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn format_alpha(alpha: u8) -> String {
    let formatted = format!("{:.3}", f64::from(alpha) / 255.0);
    formatted.trim_end_matches('0').trim_end_matches('.').into()
}

fn hint_result() -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "color-tools:hint".into(),
        title: "Color".into(),
        subtitle: "Enter #FF5733, rgb(255, 87, 51), or hsl(11, 100%, 60%)".into(),
        preview: "Convert HEX, RGB, HSL, alpha values, and common named colors locally".into(),
        kind: ResultKind::Other,
        score: 3_200.0,
        metadata: BTreeMap::from([("symbol_name".into(), "paintpalette.fill".into())]),
        actions: Vec::new(),
    }
}

fn color_result(
    query: &str,
    color: Color,
    format: ColorFormat,
    result_id: String,
    preview: String,
    score: f32,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: format!("{query} — {} {}", format.label, format.value),
        subtitle: "Local color conversion".into(),
        preview,
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([
            ("preview_style".into(), "text".into()),
            ("symbol_color".into(), color.swatch()),
            ("symbol_name".into(), "square.fill".into()),
        ]),
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste Color".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy Color".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_long_and_alpha_hex() {
        assert_eq!(
            parse_color("#0f8"),
            Some(Color {
                red: 0,
                green: 255,
                blue: 136,
                alpha: 255,
            })
        );
        assert_eq!(
            parse_color("#33669980"),
            Some(Color {
                red: 51,
                green: 102,
                blue: 153,
                alpha: 128,
            })
        );
        assert!(parse_color("#12").is_none());
        assert!(parse_color("#GGGGGG").is_none());
    }

    #[test]
    fn parses_rgb_numbers_percentages_and_modern_spaces() {
        assert_eq!(
            parse_color("rgb(255, 87, 51)"),
            Some(Color {
                red: 255,
                green: 87,
                blue: 51,
                alpha: 255,
            })
        );
        assert_eq!(
            parse_color("rgb(100% 0% 50% / 25%)"),
            Some(Color {
                red: 255,
                green: 0,
                blue: 128,
                alpha: 64,
            })
        );
        assert!(parse_color("rgb(256, 0, 0)").is_none());
        assert!(parse_color("rgba(1, 2, 3, 2)").is_none());
    }

    #[test]
    fn parses_hsl_and_normalizes_hue() {
        assert_eq!(
            parse_color("hsl(0, 100%, 50%)"),
            Some(Color {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255,
            })
        );
        assert_eq!(
            parse_color("hsla(480deg 100% 50% / .5)"),
            Some(Color {
                red: 0,
                green: 255,
                blue: 0,
                alpha: 128,
            })
        );
        assert!(parse_color("hsl(0, 101%, 50%)").is_none());
    }

    #[test]
    fn only_named_colors_with_an_explicit_command_are_matched() {
        assert_eq!(parse_request("red"), None);
        assert_eq!(
            parse_request("color rebeccapurple"),
            Some(ColorRequest::Convert(Color {
                red: 102,
                green: 51,
                blue: 153,
                alpha: 255,
            }))
        );
        assert_eq!(parse_request("color"), Some(ColorRequest::Hint));
    }

    #[test]
    fn formats_hex_rgb_and_hsl_consistently() {
        let formats = Color {
            red: 255,
            green: 87,
            blue: 51,
            alpha: 255,
        }
        .formats();
        assert_eq!(formats[0].value, "#FF5733");
        assert_eq!(formats[1].value, "rgb(255, 87, 51)");
        assert_eq!(formats[2].value, "hsl(10.6, 100%, 60%)");

        let alpha = Color {
            red: 51,
            green: 102,
            blue: 153,
            alpha: 128,
        }
        .formats();
        assert_eq!(alpha[0].value, "#33669980");
        assert_eq!(alpha[1].value, "rgba(51, 102, 153, 0.502)");
    }

    #[test]
    fn search_results_remain_rankable_and_expose_safe_actions() {
        let mut plugin = ColorToolsPlugin::default();
        let results = plugin.search("color #ff5733", 20).unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].title.starts_with("color #ff5733"));
        assert_eq!(results[0].metadata["symbol_color"], "#FF5733");
        assert_eq!(results[0].metadata["preview_style"], "text");
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
    }

    #[test]
    fn subsequent_search_invalidates_old_results() {
        let mut plugin = ColorToolsPlugin::default();
        plugin.search("#ff0000", 20).unwrap();
        assert!(plugin.recent_values.contains_key("color-tools:hex"));
        plugin.search("not a color", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }
}
