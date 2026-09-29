use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

const PLUGIN_ID: &str = "io.clipkeep.password-generator";
const DEFAULT_PASSWORD_LENGTH: usize = 20;
const MIN_PASSWORD_LENGTH: usize = 8;
const MAX_PASSWORD_LENGTH: usize = 128;
const DEFAULT_PIN_LENGTH: usize = 6;
const MIN_PIN_LENGTH: usize = 4;
const MAX_PIN_LENGTH: usize = 32;

const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.?";
const READABLE_LOWERCASE: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
const READABLE_UPPERCASE: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const READABLE_DIGITS: &[u8] = b"23456789";
const READABLE_SYMBOLS: &[u8] = b"-_!@#$%&*+?";

fn main() -> io::Result<()> {
    run_plugin(PasswordGeneratorPlugin::default())
}

#[derive(Default)]
struct PasswordGeneratorPlugin {
    recent_values: HashMap<String, String>,
}

impl Plugin for PasswordGeneratorPlugin {
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
        Ok(generated
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, password)| {
                let result_id = format!("password-generator:{}", password.id);
                self.recent_values
                    .insert(result_id.clone(), password.value.clone());
                password_result(
                    query,
                    password,
                    result_id,
                    index == 0,
                    3_500.0 - index as f32,
                )
            })
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown password generator action: {action_id}"));
        }
        let value = self
            .recent_values
            .get(result_id)
            .ok_or_else(|| "generated password is no longer active".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Generated secret copied".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasswordStyle {
    Strong,
    Readable,
    Alphanumeric,
    Pin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasswordRequest {
    All(usize),
    One(PasswordStyle, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GeneratedPassword {
    id: &'static str,
    label: &'static str,
    detail: String,
    symbol: &'static str,
    value: String,
}

fn parse_request(query: &str) -> Option<PasswordRequest> {
    let normalized = query.trim().to_lowercase();
    if let Some(arguments) = strip_command(&normalized, &["pin", "数字密码"]) {
        let length = parse_single_length(arguments, DEFAULT_PIN_LENGTH)?;
        return valid_pin_length(length)
            .then_some(PasswordRequest::One(PasswordStyle::Pin, length));
    }

    let arguments = strip_command(
        &normalized,
        &["password", "passwords", "pass", "pwd", "密码", "随机密码"],
    )?;
    let mut style = None;
    let mut length = None;
    for argument in arguments.split_whitespace() {
        if let Ok(value) = argument.parse::<usize>() {
            if length.replace(value).is_some() {
                return None;
            }
            continue;
        }
        let parsed_style = match argument {
            "strong" | "secure" | "强" | "强密码" => PasswordStyle::Strong,
            "readable" | "easy" | "易读" => PasswordStyle::Readable,
            "alphanumeric" | "alnum" | "nosymbol" | "无符号" => PasswordStyle::Alphanumeric,
            "pin" | "numeric" | "数字" => PasswordStyle::Pin,
            _ => return None,
        };
        if style.replace(parsed_style).is_some() {
            return None;
        }
    }

    let length = length.unwrap_or_else(|| {
        if style == Some(PasswordStyle::Pin) {
            DEFAULT_PIN_LENGTH
        } else {
            DEFAULT_PASSWORD_LENGTH
        }
    });
    if style == Some(PasswordStyle::Pin) {
        return valid_pin_length(length)
            .then_some(PasswordRequest::One(PasswordStyle::Pin, length));
    }
    if !valid_password_length(length) {
        return None;
    }
    Some(match style {
        Some(style) => PasswordRequest::One(style, length),
        None => PasswordRequest::All(length),
    })
}

fn strip_command<'a>(query: &'a str, commands: &[&str]) -> Option<&'a str> {
    commands.iter().find_map(|command| {
        (query == *command)
            .then_some("")
            .or_else(|| query.strip_prefix(&format!("{command} ")))
    })
}

fn parse_single_length(arguments: &str, default: usize) -> Option<usize> {
    if arguments.is_empty() {
        Some(default)
    } else {
        let mut parts = arguments.split_whitespace();
        let length = parts.next()?.parse::<usize>().ok()?;
        parts.next().is_none().then_some(length)
    }
}

fn valid_password_length(length: usize) -> bool {
    (MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&length)
}

fn valid_pin_length(length: usize) -> bool {
    (MIN_PIN_LENGTH..=MAX_PIN_LENGTH).contains(&length)
}

fn generate(request: PasswordRequest) -> Result<Vec<GeneratedPassword>, String> {
    let specifications: Vec<(PasswordStyle, usize)> = match request {
        PasswordRequest::All(length) => vec![
            (PasswordStyle::Strong, length),
            (PasswordStyle::Readable, length),
            (PasswordStyle::Alphanumeric, length),
        ],
        PasswordRequest::One(style, length) => vec![(style, length)],
    };
    specifications
        .into_iter()
        .map(|(style, length)| generate_password(style, length))
        .collect()
}

fn generate_password(style: PasswordStyle, length: usize) -> Result<GeneratedPassword, String> {
    let (id, label, detail, symbol, classes): (_, _, _, _, &[&[u8]]) = match style {
        PasswordStyle::Strong => (
            "strong",
            "Strong",
            "lowercase, uppercase, digits, and symbols",
            "lock.shield.fill",
            &[LOWERCASE, UPPERCASE, DIGITS, SYMBOLS],
        ),
        PasswordStyle::Readable => (
            "readable",
            "Readable",
            "ambiguous characters removed",
            "textformat.abc",
            &[
                READABLE_LOWERCASE,
                READABLE_UPPERCASE,
                READABLE_DIGITS,
                READABLE_SYMBOLS,
            ],
        ),
        PasswordStyle::Alphanumeric => (
            "alphanumeric",
            "Alphanumeric",
            "lowercase, uppercase, and digits",
            "character.cursor.ibeam",
            &[LOWERCASE, UPPERCASE, DIGITS],
        ),
        PasswordStyle::Pin => (
            "pin",
            "Numeric PIN",
            "digits only",
            "number.square.fill",
            &[DIGITS],
        ),
    };
    let value = generate_from_classes(length, classes, random_index)?;
    Ok(GeneratedPassword {
        id,
        label,
        detail: format!("{length} characters · {detail} · system secure randomness"),
        symbol,
        value,
    })
}

fn generate_from_classes<F>(length: usize, classes: &[&[u8]], mut pick: F) -> Result<String, String>
where
    F: FnMut(usize) -> Result<usize, String>,
{
    if classes.is_empty() || length < classes.len() || classes.iter().any(|class| class.is_empty())
    {
        return Err("invalid password character classes".into());
    }
    let alphabet: Vec<u8> = classes
        .iter()
        .flat_map(|class| class.iter().copied())
        .collect();
    loop {
        let mut password = Vec::with_capacity(length);
        for _ in 0..length {
            password.push(alphabet[pick(alphabet.len())?]);
        }
        if classes
            .iter()
            .all(|class| password.iter().any(|byte| class.contains(byte)))
        {
            return String::from_utf8(password).map_err(|error| error.to_string());
        }
    }
}

fn random_index(upper_bound: usize) -> Result<usize, String> {
    if !(1..=256).contains(&upper_bound) {
        return Err("random selection bound is out of range".into());
    }
    loop {
        let mut byte = [0_u8; 1];
        getrandom::getrandom(&mut byte)
            .map_err(|error| format!("secure random generation failed: {error}"))?;
        if let Some(index) = accepted_index(byte[0], upper_bound) {
            return Ok(index);
        }
    }
}

fn accepted_index(byte: u8, upper_bound: usize) -> Option<usize> {
    if !(1..=256).contains(&upper_bound) {
        return None;
    }
    let zone = (256 / upper_bound) * upper_bound;
    (usize::from(byte) < zone).then_some(usize::from(byte) % upper_bound)
}

fn password_result(
    query: &str,
    password: GeneratedPassword,
    result_id: String,
    primary: bool,
    score: f32,
) -> SearchResult {
    let action_noun = if password.id == "pin" {
        "PIN"
    } else {
        "Password"
    };
    let title = if primary {
        query.into()
    } else {
        format!("{query} — {}", password.label)
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle: format!("{} · {}", password.label, password.value),
        preview: format!("{}\n{}", password.value, password.detail),
        kind: ResultKind::Command,
        score,
        metadata: BTreeMap::from([("symbol_name".into(), password.symbol.into())]),
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: format!("Paste {action_noun}"),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: format!("Copy {action_noun}"),
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
    fn parses_password_styles_lengths_and_aliases() {
        assert_eq!(
            parse_request("password"),
            Some(PasswordRequest::All(DEFAULT_PASSWORD_LENGTH))
        );
        assert_eq!(parse_request("password 32"), Some(PasswordRequest::All(32)));
        assert_eq!(
            parse_request("pwd readable 24"),
            Some(PasswordRequest::One(PasswordStyle::Readable, 24))
        );
        assert_eq!(
            parse_request("密码 无符号 18"),
            Some(PasswordRequest::One(PasswordStyle::Alphanumeric, 18))
        );
        assert_eq!(
            parse_request("pin"),
            Some(PasswordRequest::One(PasswordStyle::Pin, DEFAULT_PIN_LENGTH))
        );
        assert_eq!(
            parse_request("password pin 8"),
            Some(PasswordRequest::One(PasswordStyle::Pin, 8))
        );
        assert_eq!(parse_request("password 7"), None);
        assert_eq!(parse_request("pin 33"), None);
        assert_eq!(parse_request("passphrase"), None);
    }

    #[test]
    fn generated_password_contains_every_requested_character_class() {
        let classes: &[&[u8]] = &[LOWERCASE, UPPERCASE, DIGITS, SYMBOLS];
        let required_indexes = [
            0,
            LOWERCASE.len(),
            LOWERCASE.len() + UPPERCASE.len(),
            LOWERCASE.len() + UPPERCASE.len() + DIGITS.len(),
        ];
        let mut next = 0_usize;
        let password = generate_from_classes(20, classes, |bound| {
            let selected = required_indexes.get(next).copied().unwrap_or(0) % bound;
            next += 1;
            Ok(selected)
        })
        .unwrap();
        assert_eq!(password.len(), 20);
        assert!(password.bytes().any(|byte| LOWERCASE.contains(&byte)));
        assert!(password.bytes().any(|byte| UPPERCASE.contains(&byte)));
        assert!(password.bytes().any(|byte| DIGITS.contains(&byte)));
        assert!(password.bytes().any(|byte| SYMBOLS.contains(&byte)));
    }

    #[test]
    fn readable_password_excludes_ambiguous_characters() {
        let required_indexes = [
            0,
            READABLE_LOWERCASE.len(),
            READABLE_LOWERCASE.len() + READABLE_UPPERCASE.len(),
            READABLE_LOWERCASE.len() + READABLE_UPPERCASE.len() + READABLE_DIGITS.len(),
        ];
        let mut next = 0_usize;
        let password = generate_from_classes(
            32,
            &[
                READABLE_LOWERCASE,
                READABLE_UPPERCASE,
                READABLE_DIGITS,
                READABLE_SYMBOLS,
            ],
            |bound| {
                let selected = required_indexes.get(next).copied().unwrap_or(bound - 1) % bound;
                next += 1;
                Ok(selected)
            },
        )
        .unwrap();
        assert!(
            password
                .bytes()
                .all(|byte| !matches!(byte, b'0' | b'1' | b'I' | b'O' | b'l'))
        );
    }

    #[test]
    fn rejection_sampling_discards_the_biased_tail() {
        assert_eq!(accepted_index(0, 10), Some(0));
        assert_eq!(accepted_index(249, 10), Some(9));
        assert_eq!(accepted_index(250, 10), None);
        assert_eq!(accepted_index(255, 64), Some(63));
        assert_eq!(accepted_index(255, 256), Some(255));
        assert_eq!(accepted_index(0, 0), None);
    }

    #[test]
    fn all_request_generates_three_password_variants() {
        let generated = generate(PasswordRequest::All(24)).unwrap();
        assert_eq!(
            generated.iter().map(|item| item.id).collect::<Vec<_>>(),
            ["strong", "readable", "alphanumeric"]
        );
        assert!(generated.iter().all(|item| item.value.len() == 24));
    }

    #[test]
    fn pin_is_numeric_and_has_the_requested_length() {
        let generated = generate(PasswordRequest::One(PasswordStyle::Pin, 8)).unwrap();
        assert_eq!(generated[0].value.len(), 8);
        assert!(generated[0].value.bytes().all(|byte| byte.is_ascii_digit()));

        let result = password_result("pin 8", generated[0].clone(), "pin".into(), true, 1.0);
        assert_eq!(result.actions[0].title, "Paste PIN");
        assert_eq!(result.actions[1].title, "Copy PIN");
    }

    #[test]
    fn search_results_rank_exact_title_first_and_invalidate_old_values() {
        let mut plugin = PasswordGeneratorPlugin::default();
        let results = plugin.search("password", 20).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "password");
        assert_eq!(results[1].title, "password — Readable");
        assert!(results[0].subtitle.starts_with("Strong · "));
        assert!(results[0].actions[0].default);
        assert!(results[0].actions[0].paste_to_previous);
        assert_eq!(plugin.recent_values.len(), 3);

        plugin.search("not a command", 20).unwrap();
        assert!(plugin.recent_values.is_empty());
    }
}
