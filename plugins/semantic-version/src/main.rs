use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{cmp::Ordering, collections::BTreeMap, fmt, io};

const PLUGIN_ID: &str = "io.clipkeep.semantic-version";
const MAX_QUERY_BYTES: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(SemanticVersionPlugin::default())
}

#[derive(Default)]
struct SemanticVersionPlugin {
    action_values: BTreeMap<(String, String), String>,
}

impl Plugin for SemanticVersionPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let command = match parse_request(query) {
            Ok(Some(command)) => command,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let output = match evaluate(command) {
            Ok(output) => output,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        for action_id in ["paste_value", "copy_value"] {
            self.action_values
                .insert((output.id.into(), action_id.into()), output.primary.clone());
        }
        self.action_values.insert(
            (output.id.into(), "copy_report".into()),
            output.report.clone(),
        );
        Ok(vec![output.into_result()])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let value = self
            .action_values
            .get(&(result_id.into(), action_id.into()))
            .ok_or_else(|| format!("unknown or inactive Semantic Version action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            if action_id == "copy_report" {
                "Semantic Version report copied"
            } else {
                "Semantic Version result copied"
            }
            .into(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Identifier {
    Numeric(String),
    AlphaNumeric(String),
}

impl fmt::Display for Identifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Numeric(value) => write!(formatter, "{value}"),
            Self::AlphaNumeric(value) => formatter.write_str(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    major: String,
    minor: String,
    patch: String,
    prerelease: Vec<Identifier>,
    build: Vec<String>,
}

impl Version {
    fn parse(input: &str) -> Result<Self, String> {
        let input = input
            .strip_prefix('v')
            .or_else(|| input.strip_prefix('V'))
            .unwrap_or(input);
        if input.is_empty() {
            return Err("version cannot be empty".into());
        }
        if !input.is_ascii() {
            return Err(
                "Semantic Version identifiers must use ASCII letters, digits, and hyphens".into(),
            );
        }
        let (without_build, build_text) = split_once_unique(input, '+', "build metadata")?;
        let (core, prerelease_text) = split_once(without_build, '-', "prerelease identifiers")?;
        let mut core_parts = core.split('.');
        let major = parse_core_number(core_parts.next(), "major")?;
        let minor = parse_core_number(core_parts.next(), "minor")?;
        let patch = parse_core_number(core_parts.next(), "patch")?;
        if core_parts.next().is_some() {
            return Err("version core must contain exactly major.minor.patch".into());
        }
        let prerelease = prerelease_text
            .map(parse_prerelease)
            .transpose()?
            .unwrap_or_default();
        let build = build_text.map(parse_build).transpose()?.unwrap_or_default();
        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
            build,
        })
    }

    fn precedence_cmp(&self, other: &Self) -> Ordering {
        compare_decimal(&self.major, &other.major)
            .then_with(|| compare_decimal(&self.minor, &other.minor))
            .then_with(|| compare_decimal(&self.patch, &other.patch))
            .then_with(|| compare_prerelease(&self.prerelease, &other.prerelease))
    }

    fn core(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    fn channel(&self) -> &'static str {
        if self.prerelease.is_empty() {
            "Stable release"
        } else {
            "Prerelease"
        }
    }

    fn bumped(&self, kind: BumpKind) -> Result<Self, String> {
        let (major, minor, patch) = match kind {
            BumpKind::Major => (increment_decimal(&self.major), "0".into(), "0".into()),
            BumpKind::Minor => (
                self.major.clone(),
                increment_decimal(&self.minor),
                "0".into(),
            ),
            BumpKind::Patch => (
                self.major.clone(),
                self.minor.clone(),
                increment_decimal(&self.patch),
            ),
            BumpKind::Release => {
                if self.prerelease.is_empty() && self.build.is_empty() {
                    return Err("release bump requires prerelease or build metadata".into());
                }
                (self.major.clone(), self.minor.clone(), self.patch.clone())
            }
        };
        Ok(Self {
            major,
            minor,
            patch,
            prerelease: Vec::new(),
            build: Vec::new(),
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.prerelease.is_empty() {
            formatter.write_str("-")?;
            join_display(formatter, &self.prerelease)?;
        }
        if !self.build.is_empty() {
            write!(formatter, "+{}", self.build.join("."))?;
        }
        Ok(())
    }
}

fn join_display<T: fmt::Display>(formatter: &mut fmt::Formatter<'_>, values: &[T]) -> fmt::Result {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            formatter.write_str(".")?;
        }
        write!(formatter, "{value}")?;
    }
    Ok(())
}

fn split_once_unique<'a>(
    input: &'a str,
    delimiter: char,
    label: &str,
) -> Result<(&'a str, Option<&'a str>), String> {
    let Some((left, right)) = input.split_once(delimiter) else {
        return Ok((input, None));
    };
    if right.contains(delimiter) {
        return Err(format!(
            "{label} may contain only one '{delimiter}' separator"
        ));
    }
    if right.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    Ok((left, Some(right)))
}

fn split_once<'a>(
    input: &'a str,
    delimiter: char,
    label: &str,
) -> Result<(&'a str, Option<&'a str>), String> {
    let Some((left, right)) = input.split_once(delimiter) else {
        return Ok((input, None));
    };
    if right.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    Ok((left, Some(right)))
}

fn parse_core_number(value: Option<&str>, label: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| "version core must contain major.minor.patch".to_owned())?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{label} version must be a non-negative integer"));
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(format!("{label} version cannot contain leading zeros"));
    }
    Ok(value.into())
}

fn compare_decimal(left: &str, right: &str) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn increment_decimal(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    for byte in bytes.iter_mut().rev() {
        if *byte == b'9' {
            *byte = b'0';
        } else {
            *byte += 1;
            return bytes.into_iter().map(char::from).collect();
        }
    }
    let mut result = String::with_capacity(bytes.len() + 1);
    result.push('1');
    result.extend(bytes.into_iter().map(char::from));
    result
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn parse_prerelease(input: &str) -> Result<Vec<Identifier>, String> {
    input
        .split('.')
        .map(|value| {
            if !valid_identifier(value) {
                return Err(format!("invalid prerelease identifier: {value:?}"));
            }
            if value.bytes().all(|byte| byte.is_ascii_digit()) {
                if value.len() > 1 && value.starts_with('0') {
                    return Err(format!(
                        "numeric prerelease identifier cannot contain leading zeros: {value}"
                    ));
                }
                Ok(Identifier::Numeric(value.into()))
            } else {
                Ok(Identifier::AlphaNumeric(value.into()))
            }
        })
        .collect()
}

fn parse_build(input: &str) -> Result<Vec<String>, String> {
    input
        .split('.')
        .map(|value| {
            if valid_identifier(value) {
                Ok(value.into())
            } else {
                Err(format!("invalid build identifier: {value:?}"))
            }
        })
        .collect()
}

fn compare_prerelease(left: &[Identifier], right: &[Identifier]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    for (left, right) in left.iter().zip(right) {
        let ordering = match (left, right) {
            (Identifier::Numeric(left), Identifier::Numeric(right)) => compare_decimal(left, right),
            (Identifier::Numeric(_), Identifier::AlphaNumeric(_)) => Ordering::Less,
            (Identifier::AlphaNumeric(_), Identifier::Numeric(_)) => Ordering::Greater,
            (Identifier::AlphaNumeric(left), Identifier::AlphaNumeric(right)) => left.cmp(right),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BumpKind {
    Major,
    Minor,
    Patch,
    Release,
}

impl BumpKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            "patch" => Ok(Self::Patch),
            "release" | "stable" => Ok(Self::Release),
            _ => Err("bump type must be major, minor, patch, or release".into()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::Patch => "Patch",
            Self::Release => "Stable release",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Inspect(Version),
    Compare(Version, Version),
    Bump(BumpKind, Version),
}

fn parse_request(query: &str) -> Result<Option<Command>, String> {
    let query = query.trim();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    if !matches!(
        command.to_ascii_lowercase().as_str(),
        "semver" | "versioninfo"
    ) && command != "版本"
    {
        return Ok(None);
    }
    if query.len() > MAX_QUERY_BYTES {
        return Err("Semantic Version query exceeds 4 KiB".into());
    }
    let remainder = &query[command_end..];
    let input = remainder.strip_prefix(':').unwrap_or(remainder).trim();
    if input.is_empty() {
        return Err("expected a version, comparison, or bump command".into());
    }
    let parts = input.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [operation, left, right] if operation.eq_ignore_ascii_case("compare") => Ok(Some(
            Command::Compare(Version::parse(left)?, Version::parse(right)?),
        )),
        [left, operation, right] if operation.eq_ignore_ascii_case("vs") => Ok(Some(
            Command::Compare(Version::parse(left)?, Version::parse(right)?),
        )),
        [operation, kind, version] if operation.eq_ignore_ascii_case("bump") => Ok(Some(
            Command::Bump(BumpKind::parse(kind)?, Version::parse(version)?),
        )),
        [version] => Ok(Some(Command::Inspect(Version::parse(version)?))),
        _ => Err(
            "use 'semver VERSION', 'semver compare A B', 'semver A vs B', or 'semver bump TYPE VERSION'"
                .into(),
        ),
    }
}

struct Output {
    id: &'static str,
    title: String,
    subtitle: String,
    report: String,
    primary: String,
    value_label: &'static str,
}

impl Output {
    fn into_result(self) -> SearchResult {
        let actions = vec![
            action(
                "paste_value",
                &format!("Paste {}", self.value_label),
                true,
                true,
                true,
            ),
            action(
                "copy_value",
                &format!("Copy {}", self.value_label),
                false,
                false,
                false,
            ),
            action("copy_report", "Copy Report", false, false, false),
        ];
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: self.id.into(),
            title: self.title,
            subtitle: self.subtitle,
            preview: self.report,
            kind: ResultKind::Command,
            score: 3_440.0,
            metadata: BTreeMap::from([
                ("symbol_name".into(), "number.square".into()),
                ("preview_style".into(), "text".into()),
            ]),
            actions,
        }
    }
}

fn evaluate(command: Command) -> Result<Output, String> {
    match command {
        Command::Inspect(version) => Ok(inspect_output(version)),
        Command::Compare(left, right) => Ok(compare_output(left, right)),
        Command::Bump(kind, version) => bump_output(kind, version),
    }
}

fn inspect_output(version: Version) -> Output {
    let normalized = version.to_string();
    let prerelease = if version.prerelease.is_empty() {
        "None".into()
    } else {
        version
            .prerelease
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(".")
    };
    let build = if version.build.is_empty() {
        "None".into()
    } else {
        version.build.join(".")
    };
    let report = [
        "Semantic Version inspection".into(),
        format!("Normalized: {normalized}"),
        format!("Release channel: {}", version.channel()),
        format!("Major: {}", version.major),
        format!("Minor: {}", version.minor),
        format!("Patch: {}", version.patch),
        format!("Prerelease: {prerelease}"),
        format!("Build metadata: {build}"),
        "Build metadata does not affect SemVer precedence.".into(),
    ]
    .join("\n");
    Output {
        id: "semantic-version:inspect",
        title: format!("SemVer {normalized}"),
        subtitle: format!(
            "{} · major {} · minor {} · patch {}",
            version.channel(),
            version.major,
            version.minor,
            version.patch
        ),
        report,
        primary: normalized,
        value_label: "Version",
    }
}

fn compare_output(left: Version, right: Version) -> Output {
    let ordering = left.precedence_cmp(&right);
    let symbol = match ordering {
        Ordering::Less => "<",
        Ordering::Equal => "=",
        Ordering::Greater => ">",
    };
    let expression = format!("{left} {symbol} {right}");
    let explanation = match ordering {
        Ordering::Less => format!("{left} has lower precedence than {right}"),
        Ordering::Greater => format!("{left} has higher precedence than {right}"),
        Ordering::Equal if left == right => "The versions are identical.".into(),
        Ordering::Equal => {
            "The versions have equal precedence; build metadata is ignored by SemVer comparison."
                .into()
        }
    };
    let report = [
        "Semantic Version comparison".into(),
        format!("Result: {expression}"),
        explanation.clone(),
        format!("Left core: {}", left.core()),
        format!("Right core: {}", right.core()),
    ]
    .join("\n");
    Output {
        id: "semantic-version:compare",
        title: expression.clone(),
        subtitle: explanation,
        report,
        primary: expression,
        value_label: "Comparison",
    }
}

fn bump_output(kind: BumpKind, version: Version) -> Result<Output, String> {
    let bumped = version.bumped(kind)?;
    let primary = bumped.to_string();
    let report = [
        "Semantic Version bump".into(),
        format!("Input: {version}"),
        format!("Operation: {}", kind.label()),
        format!("Result: {primary}"),
        "Prerelease and build metadata are removed from the result.".into(),
    ]
    .join("\n");
    Ok(Output {
        id: "semantic-version:bump",
        title: primary.clone(),
        subtitle: format!("{} bump from {version}", kind.label()),
        report,
        primary,
        value_label: "Version",
    })
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "semantic-version:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Semantic Version unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Semantic Version unavailable\n{}", safe_text(message)),
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

    #[test]
    fn parses_and_normalizes_complete_versions() {
        let version = Version::parse("v1.2.3-alpha-one.1+build.007").unwrap();
        assert_eq!(version.to_string(), "1.2.3-alpha-one.1+build.007");
        assert_eq!(version.major, "1");
        assert_eq!(version.prerelease[1], Identifier::Numeric("1".into()));
        assert_eq!(version.build, ["build", "007"]);
    }

    #[test]
    fn rejects_invalid_core_and_identifiers() {
        for input in [
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-",
            "1.2.3-alpha..1",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3+build..x",
            "1.2.3+one+two",
            "1.2.3-α",
        ] {
            assert!(Version::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn follows_the_official_prerelease_precedence_sequence() {
        let sequence = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ]
        .map(|value| Version::parse(value).unwrap());
        for versions in sequence.windows(2) {
            assert_eq!(versions[0].precedence_cmp(&versions[1]), Ordering::Less);
        }
    }

    #[test]
    fn compares_core_numbers_and_ignores_build_metadata() {
        let older = Version::parse("1.9.9").unwrap();
        let newer = Version::parse("2.0.0").unwrap();
        assert_eq!(older.precedence_cmp(&newer), Ordering::Less);
        let left = Version::parse("1.0.0+abc").unwrap();
        let right = Version::parse("1.0.0+xyz").unwrap();
        assert_eq!(left.precedence_cmp(&right), Ordering::Equal);
        assert_ne!(left, right);
    }

    #[test]
    fn parses_only_explicit_supported_commands() {
        assert!(parse_request("ordinary 1.2.3").unwrap().is_none());
        assert!(matches!(
            parse_request("semver: 1.2.3").unwrap(),
            Some(Command::Inspect(_))
        ));
        assert!(matches!(
            parse_request("版本 1.2.3 vs 2.0.0").unwrap(),
            Some(Command::Compare(_, _))
        ));
        assert!(matches!(
            parse_request("versioninfo bump patch 1.2.3").unwrap(),
            Some(Command::Bump(BumpKind::Patch, _))
        ));
    }

    #[test]
    fn rejects_malformed_commands_and_bounded_queries() {
        assert!(parse_request("semver").is_err());
        assert!(parse_request("semver compare 1.0.0").is_err());
        assert!(parse_request("semver bump banana 1.0.0").is_err());
        assert!(parse_request(&format!("semver {}", "1".repeat(MAX_QUERY_BYTES))).is_err());
    }

    #[test]
    fn bumps_each_core_component_and_releases_prereleases() {
        let version = Version::parse("1.2.3-beta+7").unwrap();
        assert_eq!(
            version.bumped(BumpKind::Major).unwrap().to_string(),
            "2.0.0"
        );
        assert_eq!(
            version.bumped(BumpKind::Minor).unwrap().to_string(),
            "1.3.0"
        );
        assert_eq!(
            version.bumped(BumpKind::Patch).unwrap().to_string(),
            "1.2.4"
        );
        assert_eq!(
            version.bumped(BumpKind::Release).unwrap().to_string(),
            "1.2.3"
        );
        assert!(
            Version::parse("1.2.3")
                .unwrap()
                .bumped(BumpKind::Release)
                .is_err()
        );
    }

    #[test]
    fn supports_arbitrarily_large_numeric_identifiers_and_bumps() {
        let large = Version::parse("18446744073709551616.0.0-18446744073709551616").unwrap();
        assert_eq!(
            large.bumped(BumpKind::Major).unwrap().to_string(),
            "18446744073709551617.0.0"
        );
        let lower = Version::parse("1.0.0-99999999999999999999").unwrap();
        let higher = Version::parse("1.0.0-100000000000000000000").unwrap();
        assert_eq!(lower.precedence_cmp(&higher), Ordering::Less);
    }

    #[test]
    fn outputs_explain_inspection_comparison_and_bumps() {
        let inspected = inspect_output(Version::parse("1.2.3-rc.1+5").unwrap());
        assert!(inspected.report.contains("Prerelease: rc.1"));
        let compared = compare_output(
            Version::parse("1.0.0+one").unwrap(),
            Version::parse("1.0.0+two").unwrap(),
        );
        assert_eq!(compared.primary, "1.0.0+one = 1.0.0+two");
        assert!(compared.subtitle.contains("build metadata"));
        let bumped = bump_output(BumpKind::Minor, Version::parse("1.2.3").unwrap()).unwrap();
        assert_eq!(bumped.primary, "1.3.0");
    }

    #[test]
    fn search_results_are_safe_actionable_and_invalidate_stale_values() {
        let mut plugin = SemanticVersionPlugin::default();
        let result = plugin.search("semver 1.2.3", 1).unwrap().remove(0);
        assert_eq!(result.id, "semantic-version:inspect");
        assert_eq!(result.actions.len(), 3);
        assert!(result.actions[0].default);
        assert!(
            plugin
                .action_values
                .contains_key(&(result.id.clone(), "copy_value".into()))
        );
        let invalid = plugin.search("semver 01.2.3", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(plugin.search("semver 1.2.3", 0).unwrap().is_empty());
        let unsafe_query = "semver 1.2.3\u{202E}";
        let unsafe_result = plugin.search(unsafe_query, 1).unwrap().remove(0);
        assert!(!unsafe_result.title.contains('\u{202E}'));
        assert!(unsafe_result.title.contains("\\u{202E}"));
    }
}
