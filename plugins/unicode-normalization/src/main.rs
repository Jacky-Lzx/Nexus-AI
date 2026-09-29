use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.unicode-normalization";
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_VISIBLE_CHARACTERS: usize = 4 * 1024;

fn main() -> io::Result<()> {
    run_plugin(UnicodeNormalizationPlugin::default())
}

#[derive(Default)]
struct UnicodeNormalizationPlugin {
    active_result_id: Option<String>,
    action_values: BTreeMap<String, String>,
}

impl Plugin for UnicodeNormalizationPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.active_result_id = None;
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let evaluation = match evaluate(&request) {
            Ok(evaluation) => evaluation,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let result_id = format!("unicode-normalization:{}", request.operation.id());
        self.active_result_id = Some(result_id.clone());
        self.action_values = evaluation.action_values.clone();
        Ok(vec![normalization_result(result_id, evaluation)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if self.active_result_id.as_deref() != Some(result_id) {
            return Err("Unicode normalization result is no longer active".into());
        }
        let value = self.action_values.get(action_id).ok_or_else(|| {
            format!("unknown or inactive Unicode Normalization action: {action_id}")
        })?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            if action_id == "paste_result" {
                "Normalized text copied for pasting"
            } else {
                "Unicode normalization value copied"
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NormalizationForm {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

impl NormalizationForm {
    const ALL: [Self; 4] = [Self::Nfc, Self::Nfd, Self::Nfkc, Self::Nfkd];

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "nfc" => Some(Self::Nfc),
            "nfd" => Some(Self::Nfd),
            "nfkc" => Some(Self::Nfkc),
            "nfkd" => Some(Self::Nfkd),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Nfc => "nfc",
            Self::Nfd => "nfd",
            Self::Nfkc => "nfkc",
            Self::Nfkd => "nfkd",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Nfc => "NFC",
            Self::Nfd => "NFD",
            Self::Nfkc => "NFKC",
            Self::Nfkd => "NFKD",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Nfc => "canonical decomposition followed by composition",
            Self::Nfd => "canonical decomposition",
            Self::Nfkc => "compatibility decomposition followed by composition",
            Self::Nfkd => "compatibility decomposition",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Normalize(NormalizationForm),
    StripDiacritics,
    Inspect,
    Compare,
}

impl Operation {
    fn id(self) -> &'static str {
        match self {
            Self::Normalize(form) => form.id(),
            Self::StripDiacritics => "strip-diacritics",
            Self::Inspect => "inspect",
            Self::Compare => "compare",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Normalize(NormalizationForm::Nfc) => "Normalize Unicode to NFC",
            Self::Normalize(NormalizationForm::Nfd) => "Normalize Unicode to NFD",
            Self::Normalize(NormalizationForm::Nfkc) => "Normalize Unicode to NFKC",
            Self::Normalize(NormalizationForm::Nfkd) => "Normalize Unicode to NFKD",
            Self::StripDiacritics => "Strip Unicode Diacritics",
            Self::Inspect => "Inspect Unicode Normalization",
            Self::Compare => "Compare Unicode Equivalence",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    operation: Operation,
    left: String,
    right: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Evaluation {
    title: String,
    subtitle: String,
    preview: String,
    action_values: BTreeMap<String, String>,
    actions: Vec<ResultAction>,
}

fn parse_request(query: &str) -> Result<Option<Request>, String> {
    let query = query.trim_start();
    let command_end = query
        .find(|character: char| character.is_whitespace() || character == ':')
        .unwrap_or(query.len());
    let command = &query[..command_end];
    let command_lower = command.to_ascii_lowercase();

    let direct_operation = NormalizationForm::parse(&command_lower).map(Operation::Normalize);
    let command_kind = if direct_operation.is_some() {
        "direct"
    } else {
        match command_lower.as_str() {
            "unorm" | "unicode-normalize" | "unicode-normalization" => "general",
            "stripdiacritics" | "unicode-strip-diacritics" => "strip",
            "normcheck" | "unicode-normalization-check" => "inspect",
            "unicodeeq" | "normeq" | "unicode-equivalent" => "compare",
            _ if command == "Unicode规范化" => "general",
            _ if command == "Unicode去音标" => "strip",
            _ if command == "Unicode规范检查" => "inspect",
            _ if command == "Unicode等价" => "compare",
            _ => return Ok(None),
        }
    };
    if query.len() > MAX_TEXT_BYTES * 2 + 1024 {
        return Err("Unicode Normalization query exceeds its bounded input size".into());
    }

    let mut remainder = query[command_end..].trim_start();
    if remainder.starts_with(':') && !remainder.starts_with("::") {
        remainder = remainder[1..].trim_start();
    }
    let (argument, text) = split_arguments(remainder)?;
    let argument = argument.trim();

    let operation = if let Some(operation) = direct_operation {
        if !argument.is_empty() {
            return Err(format!(
                "{} does not accept an option before '::'",
                operation.label()
            ));
        }
        operation
    } else {
        match command_kind {
            "general" => match argument.to_ascii_lowercase().as_str() {
                "strip" | "diacritics" | "strip-diacritics" => Operation::StripDiacritics,
                "check" | "inspect" => Operation::Inspect,
                _ => Operation::Normalize(NormalizationForm::parse(argument).ok_or_else(|| {
                    "expected one of nfc, nfd, nfkc, nfkd, strip, or check before '::'".to_owned()
                })?),
            },
            "strip" => {
                reject_argument(argument, "Strip Unicode Diacritics")?;
                Operation::StripDiacritics
            }
            "inspect" => {
                reject_argument(argument, "Inspect Unicode Normalization")?;
                Operation::Inspect
            }
            "compare" => {
                if argument.is_empty() {
                    return Err("left-hand text is required before '::'".into());
                }
                Operation::Compare
            }
            _ => unreachable!(),
        }
    };

    if operation == Operation::Compare {
        validate_text(argument, "left-hand text")?;
        validate_text(text, "right-hand text")?;
    } else {
        validate_text(text, "Unicode text")?;
    }

    Ok(Some(Request {
        operation,
        left: if operation == Operation::Compare {
            argument.to_owned()
        } else {
            text.to_owned()
        },
        right: if operation == Operation::Compare {
            text.to_owned()
        } else {
            String::new()
        },
    }))
}

fn split_arguments(remainder: &str) -> Result<(&str, &str), String> {
    if remainder == "::" {
        return Ok(("", ""));
    }
    if let Some(text) = remainder.strip_prefix(":: ") {
        return Ok(("", text));
    }
    if let Some(argument) = remainder.strip_suffix(" ::") {
        return Ok((argument, ""));
    }
    remainder
        .split_once(" :: ")
        .ok_or_else(|| "expected '[form or left text] :: <text>'".to_owned())
}

fn reject_argument(argument: &str, label: &str) -> Result<(), String> {
    if argument.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} does not accept an option before '::'"))
    }
}

fn validate_text(text: &str, label: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!("{label} exceeds 64 KiB"));
    }
    Ok(())
}

fn evaluate(request: &Request) -> Result<Evaluation, String> {
    match request.operation {
        Operation::Normalize(form) => {
            let output = normalize_text(&request.left, form)?;
            Ok(evaluate_transformation(
                request.operation,
                &request.left,
                output,
                form.description(),
            ))
        }
        Operation::StripDiacritics => {
            let output = strip_diacritics(&request.left)?;
            Ok(evaluate_transformation(
                request.operation,
                &request.left,
                output,
                "macOS Foundation diacritic-removal transform",
            ))
        }
        Operation::Inspect => evaluate_inspection(&request.left),
        Operation::Compare => evaluate_comparison(&request.left, &request.right),
    }
}

#[cfg(target_os = "macos")]
fn normalize_text(input: &str, form: NormalizationForm) -> Result<String, String> {
    use objc2_foundation::NSString;

    let output = objc2::rc::autoreleasepool(|_| {
        let input = NSString::from_str(input);
        match form {
            NormalizationForm::Nfc => input.precomposedStringWithCanonicalMapping(),
            NormalizationForm::Nfd => input.decomposedStringWithCanonicalMapping(),
            NormalizationForm::Nfkc => input.precomposedStringWithCompatibilityMapping(),
            NormalizationForm::Nfkd => input.decomposedStringWithCompatibilityMapping(),
        }
        .to_string()
    });
    validate_output(output)
}

#[cfg(not(target_os = "macos"))]
fn normalize_text(_: &str, _: NormalizationForm) -> Result<String, String> {
    Err("Unicode normalization currently requires macOS Foundation".into())
}

#[cfg(target_os = "macos")]
fn strip_diacritics(input: &str) -> Result<String, String> {
    use objc2_foundation::{NSString, NSStringTransformStripDiacritics};

    let output = objc2::rc::autoreleasepool(|_| {
        let input = NSString::from_str(input);
        let transform = unsafe { NSStringTransformStripDiacritics };
        input
            .stringByApplyingTransform_reverse(transform, false)
            .map(|value| value.to_string())
    })
    .ok_or_else(|| "macOS could not apply the diacritic-removal transform".to_owned())?;
    validate_output(output)
}

#[cfg(not(target_os = "macos"))]
fn strip_diacritics(_: &str) -> Result<String, String> {
    Err("diacritic removal currently requires macOS Foundation".into())
}

fn validate_output(output: String) -> Result<String, String> {
    if output.len() > MAX_OUTPUT_BYTES {
        Err("normalized output exceeds 256 KiB".into())
    } else {
        Ok(output)
    }
}

fn all_forms(input: &str) -> Result<BTreeMap<&'static str, String>, String> {
    NormalizationForm::ALL
        .into_iter()
        .map(|form| normalize_text(input, form).map(|output| (form.id(), output)))
        .collect()
}

fn evaluate_transformation(
    operation: Operation,
    input: &str,
    output: String,
    detail: &str,
) -> Evaluation {
    let changed = input != output;
    let preview = format!(
        "{}\nMethod: {detail}\nStatus: {}\nInput: {} scalars · {} bytes\nOutput: {} scalars · {} bytes\n\nResult\n{}",
        operation.label(),
        if changed {
            "changed"
        } else {
            "already in requested form"
        },
        input.chars().count(),
        input.len(),
        output.chars().count(),
        output.len(),
        visible_text(&compact(&output, MAX_VISIBLE_CHARACTERS))
    );
    Evaluation {
        title: output.clone(),
        subtitle: format!(
            "{} · {} · local",
            operation.label(),
            if changed { "changed" } else { "unchanged" }
        ),
        preview,
        action_values: BTreeMap::from([
            ("paste_result".into(), output.clone()),
            ("copy_result".into(), output),
        ]),
        actions: vec![
            action("paste_result", "Paste Result", true, true, true),
            action("copy_result", "Copy Result", false, false, false),
        ],
    }
}

fn evaluate_inspection(input: &str) -> Result<Evaluation, String> {
    let forms = all_forms(input)?;
    let mut lines = vec![
        "Unicode normalization inspection".to_owned(),
        format!(
            "Original: {} scalars · {} bytes",
            input.chars().count(),
            input.len()
        ),
    ];
    for form in NormalizationForm::ALL {
        let output = &forms[form.id()];
        lines.push(format!(
            "{}: {} · {} scalars · {} bytes",
            form.label(),
            if output == input {
                "already normalized"
            } else {
                "would change"
            },
            output.chars().count(),
            output.len()
        ));
    }
    lines.extend([
        String::new(),
        "NFC/NFD preserve canonical equivalence; NFKC/NFKD may fold compatibility distinctions."
            .to_owned(),
    ]);
    let report = lines.join("\n");
    let mut action_values = BTreeMap::from([("copy_report".into(), report.clone())]);
    let mut actions = vec![action(
        "copy_report",
        "Copy Normalization Report",
        true,
        false,
        false,
    )];
    for form in NormalizationForm::ALL {
        let action_id = format!("copy_{}", form.id());
        action_values.insert(action_id.clone(), forms[form.id()].clone());
        actions.push(action(
            &action_id,
            &format!("Copy {} Text", form.label()),
            false,
            false,
            false,
        ));
    }
    let normalized_forms = NormalizationForm::ALL
        .into_iter()
        .filter(|form| forms[form.id()] == input)
        .map(NormalizationForm::label)
        .collect::<Vec<_>>();
    Ok(Evaluation {
        title: if normalized_forms.is_empty() {
            "Unicode text is not normalized".into()
        } else {
            format!("Unicode text matches {}", normalized_forms.join(", "))
        },
        subtitle: format!(
            "Normalization inspection · {} scalars · {} bytes",
            input.chars().count(),
            input.len()
        ),
        preview: report,
        action_values,
        actions,
    })
}

fn evaluate_comparison(left: &str, right: &str) -> Result<Evaluation, String> {
    let exact = left == right;
    let canonical = normalize_text(left, NormalizationForm::Nfc)?
        == normalize_text(right, NormalizationForm::Nfc)?;
    let compatibility = normalize_text(left, NormalizationForm::Nfkc)?
        == normalize_text(right, NormalizationForm::Nfkc)?;
    let classification = if exact {
        "Exactly equal"
    } else if canonical {
        "Canonically equivalent"
    } else if compatibility {
        "Compatibility equivalent"
    } else {
        "Not Unicode-normalization equivalent"
    };
    let report = format!(
        "Unicode equivalence comparison\nExact scalar sequence: {}\nCanonical equivalence (NFC): {}\nCompatibility equivalence (NFKC): {}\n\nLeft: {} scalars · {} bytes\n{}\n\nRight: {} scalars · {} bytes\n{}",
        yes_no(exact),
        yes_no(canonical),
        yes_no(compatibility),
        left.chars().count(),
        left.len(),
        visible_text(&compact(left, 512)),
        right.chars().count(),
        right.len(),
        visible_text(&compact(right, 512))
    );
    Ok(Evaluation {
        title: classification.into(),
        subtitle: "Unicode equivalence comparison · local".into(),
        preview: report.clone(),
        action_values: BTreeMap::from([("copy_report".into(), report)]),
        actions: vec![action(
            "copy_report",
            "Copy Equivalence Report",
            true,
            false,
            false,
        )],
    })
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn normalization_result(result_id: String, evaluation: Evaluation) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: compact(&safe_text(&evaluation.title), 200),
        subtitle: evaluation.subtitle,
        preview: compact(&evaluation.preview, MAX_VISIBLE_CHARACTERS),
        kind: ResultKind::Command,
        score: 3_350.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "character.book.closed".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: evaluation.actions,
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "unicode-normalization:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "Unicode normalization unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("Unicode normalization unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_350.0,
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
    visible_text(value)
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn visible_text(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if matches!(character, '\n' | '\t') {
            output.push(character);
        } else if character.is_control() || is_directional_control(character) {
            output.push_str(&format!("\\u{{{:04X}}}", character as u32));
        } else {
            output.push(character);
        }
    }
    output
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
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
    fn parses_direct_general_compare_and_chinese_commands() {
        assert!(parse_request("normal text").unwrap().is_none());
        assert_eq!(
            parse_request("nfc :: e\u{301}").unwrap().unwrap().operation,
            Operation::Normalize(NormalizationForm::Nfc)
        );
        assert_eq!(
            parse_request("unorm nfkd :: ①").unwrap().unwrap().operation,
            Operation::Normalize(NormalizationForm::Nfkd)
        );
        assert_eq!(
            parse_request("Unicode规范化: check :: café")
                .unwrap()
                .unwrap()
                .operation,
            Operation::Inspect
        );
        let compare = parse_request("Unicode等价: é :: e\u{301}")
            .unwrap()
            .unwrap();
        assert_eq!(compare.left, "é");
        assert_eq!(compare.right, "e\u{301}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn canonical_forms_compose_and_decompose() {
        assert_eq!(
            normalize_text("e\u{301}", NormalizationForm::Nfc).unwrap(),
            "é"
        );
        assert_eq!(
            normalize_text("é", NormalizationForm::Nfd).unwrap(),
            "e\u{301}"
        );
        assert_eq!(normalize_text("각", NormalizationForm::Nfd).unwrap(), "각");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compatibility_forms_fold_compatibility_characters() {
        assert_eq!(
            normalize_text("① ﬁ", NormalizationForm::Nfkc).unwrap(),
            "1 fi"
        );
        assert_eq!(normalize_text("①", NormalizationForm::Nfc).unwrap(), "①");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn strips_precomposed_diacritics_with_foundation() {
        assert_eq!(
            strip_diacritics("Crème Brûlée Ångström").unwrap(),
            "Creme Brulee Angstrom"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn inspection_identifies_matching_forms_and_exposes_actions() {
        let evaluation = evaluate_inspection("e\u{301}").unwrap();
        assert!(evaluation.title.contains("NFD"));
        assert_eq!(evaluation.action_values["copy_nfc"], "é");
        assert_eq!(evaluation.action_values["copy_nfd"], "e\u{301}");
        assert_eq!(evaluation.actions.len(), 5);
        assert!(evaluation.preview.contains("NFKC"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn comparison_distinguishes_exact_canonical_compatibility_and_different() {
        assert_eq!(
            evaluate_comparison("same", "same").unwrap().title,
            "Exactly equal"
        );
        assert_eq!(
            evaluate_comparison("é", "e\u{301}").unwrap().title,
            "Canonically equivalent"
        );
        assert_eq!(
            evaluate_comparison("①", "1").unwrap().title,
            "Compatibility equivalent"
        );
        assert_eq!(
            evaluate_comparison("a", "b").unwrap().title,
            "Not Unicode-normalization equivalent"
        );
    }

    #[test]
    fn rejects_missing_bad_and_oversized_requests() {
        assert!(parse_request("nfc text").is_err());
        assert!(parse_request("nfc option :: text").is_err());
        assert!(parse_request("unorm unknown :: text").is_err());
        assert!(parse_request("normcheck option :: text").is_err());
        assert!(parse_request("unicodeeq :: right").is_err());
        assert!(parse_request("unicodeeq left ::").is_err());
        assert!(parse_request("nfd ::").is_err());
        assert!(parse_request(&format!("nfc :: {}", "x".repeat(MAX_TEXT_BYTES + 1))).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn results_are_actionable_and_searches_clear_stale_values() {
        let mut plugin = UnicodeNormalizationPlugin::default();
        let result = plugin.search("nfc :: e\u{301}", 1).unwrap().remove(0);
        assert_eq!(result.id, "unicode-normalization:nfc");
        assert_eq!(plugin.action_values["paste_result"], "é");
        assert!(result.actions[0].default);

        let invalid = plugin.search("unorm bad :: text", 1).unwrap().remove(0);
        assert!(invalid.actions.is_empty());
        assert!(plugin.action_values.is_empty());
        assert!(plugin.active_result_id.is_none());
        assert!(plugin.search("nfc :: value", 0).unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn visible_fields_escape_controls_and_bidi_but_actions_keep_exact_output() {
        let mut plugin = UnicodeNormalizationPlugin::default();
        let result = plugin
            .search("nfc :: line\ntext\u{202e}\u{0001}", 1)
            .unwrap()
            .remove(0);
        assert_eq!(
            plugin.action_values["copy_result"],
            "line\ntext\u{202e}\u{0001}"
        );
        assert!(result.title.contains("\\n"));
        assert!(result.title.contains("\\u{202E}"));
        assert!(!result.preview.contains('\u{202e}'));
        assert!(!result.preview.contains('\u{0001}'));
    }

    #[test]
    fn compaction_is_unicode_safe() {
        assert_eq!(compact("é😀中", 2), "é😀…");
        assert_eq!(compact("é😀", 2), "é😀");
    }
}
