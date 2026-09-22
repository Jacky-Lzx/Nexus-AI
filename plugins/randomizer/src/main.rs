use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

const PLUGIN_ID: &str = "io.clipkeep.randomizer";
const MAX_DICE_COUNT: usize = 100;
const MAX_DIE_SIDES: u64 = 1_000_000_000;
const MAX_OPTIONS: usize = 100;
const MAX_OPTION_CHARACTERS: usize = 256;
const U64_RANGE_SIZE: u128 = u64::MAX as u128 + 1;

fn main() -> io::Result<()> {
    run_plugin(RandomizerPlugin::default())
}

#[derive(Default)]
struct RandomizerPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for RandomizerPlugin {
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
        let generated = generate(request)?;
        let result_id = format!("randomizer:{}", generated.id);
        self.recent_values
            .insert(result_id.clone(), generated.value.clone());
        Ok(vec![randomizer_result(query, generated, result_id)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown randomizer action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "random result is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Random result copied".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RandomRequest {
    Dice(DiceSpec),
    Coin,
    Integer { minimum: i64, maximum: i64 },
    Choose(Vec<String>),
    Shuffle(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiceSpec {
    count: usize,
    sides: u64,
    modifier: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GeneratedRandomValue {
    id: &'static str,
    label: &'static str,
    symbol: &'static str,
    value: String,
    detail: String,
}

fn parse_request(query: &str) -> Option<RandomRequest> {
    let (command, arguments) = split_command(query)?;
    match command.to_lowercase().as_str() {
        "roll" | "dice" | "骰子" | "掷骰子" => parse_dice(arguments).map(RandomRequest::Dice),
        "coin" | "flip" | "硬币" | "抛硬币" => {
            arguments.is_empty().then_some(RandomRequest::Coin)
        }
        "random" | "rand" | "随机数" => parse_integer_range(arguments),
        "choose" | "pick" | "选择" => parse_options(arguments).map(RandomRequest::Choose),
        "shuffle" | "洗牌" | "打乱" => parse_options(arguments).map(RandomRequest::Shuffle),
        _ => None,
    }
}

fn split_command(query: &str) -> Option<(&str, &str)> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    if let Some(index) = query.find(char::is_whitespace) {
        Some((&query[..index], query[index..].trim()))
    } else {
        Some((query, ""))
    }
}

fn parse_dice(arguments: &str) -> Option<DiceSpec> {
    let compact: String = if arguments.is_empty() {
        "1d6".into()
    } else {
        arguments
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_lowercase()
    };
    let modifier_index = compact
        .char_indices()
        .skip(1)
        .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index));
    let (dice, modifier) = if let Some(index) = modifier_index {
        (&compact[..index], compact[index..].parse::<i64>().ok()?)
    } else {
        (compact.as_str(), 0)
    };
    let (count, sides) = if let Some((count, sides)) = dice.split_once('d') {
        let count = if count.is_empty() {
            1
        } else {
            count.parse::<usize>().ok()?
        };
        (count, sides.parse::<u64>().ok()?)
    } else {
        (1, dice.parse::<u64>().ok()?)
    };
    ((1..=MAX_DICE_COUNT).contains(&count) && (2..=MAX_DIE_SIDES).contains(&sides)).then_some(
        DiceSpec {
            count,
            sides,
            modifier,
        },
    )
}

fn parse_integer_range(arguments: &str) -> Option<RandomRequest> {
    if arguments.is_empty() {
        return Some(RandomRequest::Integer {
            minimum: 1,
            maximum: 100,
        });
    }
    let values: Vec<i64> = arguments
        .split_whitespace()
        .map(str::parse::<i64>)
        .collect::<Result<_, _>>()
        .ok()?;
    let (minimum, maximum) = match values.as_slice() {
        [maximum] => (1, *maximum),
        [minimum, maximum] => (*minimum, *maximum),
        _ => return None,
    };
    (minimum <= maximum).then_some(RandomRequest::Integer { minimum, maximum })
}

fn parse_options(arguments: &str) -> Option<Vec<String>> {
    let separator = if arguments.contains('|') {
        '|'
    } else if arguments.contains(',') {
        ','
    } else {
        return None;
    };
    let options: Vec<String> = arguments
        .split(separator)
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    ((2..=MAX_OPTIONS).contains(&options.len())
        && options.iter().all(|option| {
            let length = option.chars().count();
            (1..=MAX_OPTION_CHARACTERS).contains(&length)
        }))
    .then_some(options)
}

fn generate(request: RandomRequest) -> Result<GeneratedRandomValue, String> {
    match request {
        RandomRequest::Dice(specification) => roll_dice(specification),
        RandomRequest::Coin => {
            let value = if secure_index(2)? == 0 {
                "Heads"
            } else {
                "Tails"
            };
            Ok(GeneratedRandomValue {
                id: "coin",
                label: "Coin Flip",
                symbol: "circle.lefthalf.filled",
                value: value.into(),
                detail: "One unbiased coin flip · system secure randomness".into(),
            })
        }
        RandomRequest::Integer { minimum, maximum } => {
            let value = random_i64_inclusive(minimum, maximum)?;
            Ok(GeneratedRandomValue {
                id: "integer",
                label: "Random Integer",
                symbol: "number",
                value: value.to_string(),
                detail: format!("Inclusive range {minimum}…{maximum} · system secure randomness"),
            })
        }
        RandomRequest::Choose(options) => {
            let selected = options[secure_index(options.len())?].clone();
            Ok(GeneratedRandomValue {
                id: "choice",
                label: "Random Choice",
                symbol: "checkmark.circle.fill",
                value: selected,
                detail: format!("Selected uniformly from {} options", options.len()),
            })
        }
        RandomRequest::Shuffle(mut options) => {
            fisher_yates_shuffle(&mut options)?;
            Ok(GeneratedRandomValue {
                id: "shuffle",
                label: "Shuffled List",
                symbol: "shuffle",
                value: options.join(" | "),
                detail: format!("Uniform Fisher–Yates shuffle of {} options", options.len()),
            })
        }
    }
}

fn roll_dice(specification: DiceSpec) -> Result<GeneratedRandomValue, String> {
    let mut rolls = Vec::with_capacity(specification.count);
    for _ in 0..specification.count {
        rolls.push(secure_u64_below(specification.sides)? + 1);
    }
    let sum: u128 = rolls.iter().map(|value| u128::from(*value)).sum();
    let total = i128::try_from(sum).expect("bounded dice sum fits i128")
        + i128::from(specification.modifier);
    let notation = format_dice_notation(specification);
    let roll_list = rolls
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(" + ");
    Ok(GeneratedRandomValue {
        id: "dice",
        label: "Dice Roll",
        symbol: "dice.fill",
        value: total.to_string(),
        detail: format!("{notation} · rolls: {roll_list}"),
    })
}

fn format_dice_notation(specification: DiceSpec) -> String {
    let base = format!("{}d{}", specification.count, specification.sides);
    match specification.modifier.cmp(&0) {
        std::cmp::Ordering::Greater => format!("{base}+{}", specification.modifier),
        std::cmp::Ordering::Less => format!("{base}{}", specification.modifier),
        std::cmp::Ordering::Equal => base,
    }
}

fn fisher_yates_shuffle(values: &mut [String]) -> Result<(), String> {
    for index in (1..values.len()).rev() {
        let swap_index = secure_index(index + 1)?;
        values.swap(index, swap_index);
    }
    Ok(())
}

fn random_i64_inclusive(minimum: i64, maximum: i64) -> Result<i64, String> {
    if minimum > maximum {
        return Err("random range minimum exceeds maximum".into());
    }
    let range = (i128::from(maximum) - i128::from(minimum) + 1) as u128;
    let offset = secure_offset(range)?;
    let value = i128::from(minimum) + i128::try_from(offset).expect("u64 offset fits i128");
    i64::try_from(value).map_err(|error| error.to_string())
}

fn secure_index(upper_bound: usize) -> Result<usize, String> {
    if upper_bound == 0 {
        return Err("random selection requires at least one value".into());
    }
    let offset = secure_offset(upper_bound as u128)?;
    usize::try_from(offset).map_err(|error| error.to_string())
}

fn secure_u64_below(upper_bound: u64) -> Result<u64, String> {
    if upper_bound == 0 {
        return Err("random selection requires a positive bound".into());
    }
    let offset = secure_offset(u128::from(upper_bound))?;
    u64::try_from(offset).map_err(|error| error.to_string())
}

fn secure_offset(range: u128) -> Result<u128, String> {
    if range == 0 || range > U64_RANGE_SIZE {
        return Err("random range is out of bounds".into());
    }
    loop {
        let mut bytes = [0_u8; 8];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| format!("secure random generation failed: {error}"))?;
        if let Some(offset) = offset_from_raw(u64::from_le_bytes(bytes), range) {
            return Ok(offset);
        }
    }
}

fn offset_from_raw(raw: u64, range: u128) -> Option<u128> {
    if range == 0 || range > U64_RANGE_SIZE {
        return None;
    }
    let raw = u128::from(raw);
    if range == U64_RANGE_SIZE {
        return Some(raw);
    }
    let zone = (U64_RANGE_SIZE / range) * range;
    (raw < zone).then_some(raw % range)
}

fn randomizer_result(
    query: &str,
    generated: GeneratedRandomValue,
    result_id: String,
) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: query.into(),
        subtitle: format!("{} · {}", generated.label, compact(&generated.value, 80)),
        preview: format!("{}\n{}", generated.value, generated.detail),
        kind: ResultKind::Command,
        score: 3_450.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), generated.symbol.into()),
            ("preview_style".into(), "text".into()),
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
    }
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

    #[test]
    fn parses_dice_notation_and_bounds() {
        assert_eq!(
            parse_request("roll"),
            Some(RandomRequest::Dice(DiceSpec {
                count: 1,
                sides: 6,
                modifier: 0,
            }))
        );
        assert_eq!(
            parse_request("roll 2d20 + 5"),
            Some(RandomRequest::Dice(DiceSpec {
                count: 2,
                sides: 20,
                modifier: 5,
            }))
        );
        assert_eq!(
            parse_request("骰子 d8-2"),
            Some(RandomRequest::Dice(DiceSpec {
                count: 1,
                sides: 8,
                modifier: -2,
            }))
        );
        assert_eq!(parse_request("roll 0d6"), None);
        assert_eq!(parse_request("roll 101d6"), None);
        assert_eq!(parse_request("roll 1d1"), None);
    }

    #[test]
    fn parses_coin_and_integer_ranges() {
        assert_eq!(parse_request("coin"), Some(RandomRequest::Coin));
        assert_eq!(parse_request("coin now"), None);
        assert_eq!(
            parse_request("random"),
            Some(RandomRequest::Integer {
                minimum: 1,
                maximum: 100,
            })
        );
        assert_eq!(
            parse_request("random -10 10"),
            Some(RandomRequest::Integer {
                minimum: -10,
                maximum: 10,
            })
        );
        assert_eq!(parse_request("random 10 1"), None);
        assert_eq!(parse_request("ordinary text"), None);
    }

    #[test]
    fn parses_choice_and_shuffle_without_losing_unicode_or_case() {
        assert_eq!(
            parse_request("choose Red | 蓝 | Green"),
            Some(RandomRequest::Choose(vec![
                "Red".into(),
                "蓝".into(),
                "Green".into(),
            ]))
        );
        assert_eq!(
            parse_request("打乱 甲,乙,丙"),
            Some(RandomRequest::Shuffle(vec![
                "甲".into(),
                "乙".into(),
                "丙".into(),
            ]))
        );
        assert_eq!(parse_request("choose only one"), None);
        assert_eq!(parse_request("choose a | "), None);
    }

    #[test]
    fn rejection_sampling_discards_biased_tail_and_handles_full_width() {
        assert_eq!(offset_from_raw(0, 10), Some(0));
        assert_eq!(offset_from_raw(u64::MAX - 6, 10), Some(9));
        assert_eq!(offset_from_raw(u64::MAX, 10), None);
        assert_eq!(
            offset_from_raw(u64::MAX, U64_RANGE_SIZE),
            Some(u128::from(u64::MAX))
        );
        assert_eq!(offset_from_raw(0, 0), None);
    }

    #[test]
    fn full_i64_range_mapping_stays_in_bounds() {
        assert_eq!(
            i128::from(i64::MIN)
                + i128::try_from(offset_from_raw(0, U64_RANGE_SIZE).unwrap()).unwrap(),
            i128::from(i64::MIN)
        );
        assert_eq!(
            i128::from(i64::MIN)
                + i128::try_from(offset_from_raw(u64::MAX, U64_RANGE_SIZE).unwrap()).unwrap(),
            i128::from(i64::MAX)
        );
    }

    #[test]
    fn generated_values_obey_requested_shapes() {
        let dice = generate(RandomRequest::Dice(DiceSpec {
            count: 3,
            sides: 6,
            modifier: 2,
        }))
        .unwrap();
        assert!((5..=20).contains(&dice.value.parse::<i128>().unwrap()));
        assert!(dice.detail.starts_with("3d6+2 · rolls: "));

        let integer = generate(RandomRequest::Integer {
            minimum: -3,
            maximum: 3,
        })
        .unwrap();
        assert!((-3..=3).contains(&integer.value.parse::<i64>().unwrap()));
    }

    #[test]
    fn shuffle_preserves_every_option() {
        let original = vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()];
        let generated = generate(RandomRequest::Shuffle(original.clone())).unwrap();
        let mut shuffled: Vec<&str> = generated.value.split(" | ").collect();
        let mut expected: Vec<&str> = original.iter().map(String::as_str).collect();
        shuffled.sort_unstable();
        expected.sort_unstable();
        assert_eq!(shuffled, expected);
    }

    #[test]
    fn search_result_is_exact_rankable_and_invalidates_old_values() {
        let mut plugin = RandomizerPlugin::default();
        let results = plugin.search("random 1 10", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "random 1 10");
        assert!(results[0].subtitle.starts_with("Random Integer · "));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values.len(), 1);

        plugin.search("not a command", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }

    #[test]
    fn compact_is_unicode_safe() {
        assert_eq!(compact("甲乙丙丁", 3), "甲乙丙…");
        assert_eq!(compact("short", 10), "short");
    }
}
