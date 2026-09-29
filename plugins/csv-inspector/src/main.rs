use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;

const PLUGIN_ID: &str = "io.clipkeep.csv-inspector";
const RESULT_ID: &str = "csv-inspector:result";
const MAX_INPUT_BYTES: usize = 128 * 1024;
const MAX_ROWS: usize = 1_000;
const MAX_COLUMNS: usize = 100;
const MAX_CELLS: usize = 20_000;
const MAX_FIELD_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_INCONSISTENT_ROWS: usize = 20;
const PREVIEW_ROWS: usize = 6;
const PREVIEW_COLUMNS: usize = 6;

fn main() -> io::Result<()> {
    run_plugin(CsvInspectorPlugin::default())
}

#[derive(Default)]
struct CsvInspectorPlugin {
    action_values: BTreeMap<String, String>,
}

impl Plugin for CsvInspectorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.action_values.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let request = match parse_request(query) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(Vec::new()),
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        let inspection = match inspect(&request) {
            Ok(inspection) => inspection,
            Err(message) => return Ok(vec![invalid_result(query, &message)]),
        };
        self.action_values
            .insert("paste_normalized".into(), inspection.primary_output.clone());
        self.action_values
            .insert("copy_csv".into(), inspection.csv.clone());
        self.action_values
            .insert("copy_tsv".into(), inspection.tsv.clone());
        self.action_values
            .insert("copy_json".into(), inspection.json.clone());
        self.action_values
            .insert("copy_report".into(), inspection.report.clone());
        Ok(vec![inspection_result(&inspection)])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if result_id != RESULT_ID {
            return Err("CSV inspection is no longer active".into());
        }
        let value = self
            .action_values
            .get(action_id)
            .ok_or_else(|| format!("unknown or inactive CSV Inspector action: {action_id}"))?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some(
            match action_id {
                "paste_normalized" => "Normalized table copied",
                "copy_csv" => "CSV copied",
                "copy_tsv" => "TSV copied",
                "copy_json" => "JSON copied",
                "copy_report" => "CSV inspection copied",
                _ => "Table value copied",
            }
            .into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delimiter {
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

impl Delimiter {
    fn character(self) -> char {
        match self {
            Self::Comma => ',',
            Self::Tab => '\t',
            Self::Semicolon => ';',
            Self::Pipe => '|',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Comma => "comma (,)",
            Self::Tab => "tab (\\t)",
            Self::Semicolon => "semicolon (;)",
            Self::Pipe => "pipe (|)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CsvRequest {
    delimiter: Delimiter,
    header: bool,
    prefer_tsv: bool,
    input: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CsvDocument {
    rows: Vec<Vec<String>>,
    quoted_fields: usize,
    embedded_line_breaks: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ValueKind {
    Boolean,
    Integer,
    Decimal,
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ColumnProfile {
    label: String,
    present: usize,
    missing: usize,
    empty: usize,
    maximum_characters: usize,
    kinds: BTreeSet<ValueKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CsvInspection {
    title: String,
    subtitle: String,
    report: String,
    primary_output: String,
    csv: String,
    tsv: String,
    json: String,
}

fn parse_request(query: &str) -> Result<Option<CsvRequest>, String> {
    let query = query.trim();
    let command = query
        .split(|character: char| character.is_whitespace() || character == ':')
        .next()
        .unwrap_or_default();
    let default_delimiter = match command.to_ascii_lowercase().as_str() {
        "csv" => Delimiter::Comma,
        "tsv" => Delimiter::Tab,
        _ if command == "表格" => Delimiter::Comma,
        _ => return Ok(None),
    };
    let (options, input) = query
        .split_once("::")
        .ok_or_else(|| "expected '::' between CSV options and data".to_owned())?;
    let mut tokens = options.split_whitespace();
    let _ = tokens.next();
    let mut delimiter = default_delimiter;
    let mut header = true;
    let mut header_option = None;
    let mut delimiter_option = None;
    for option in tokens {
        match option.to_ascii_lowercase().as_str() {
            "header" => {
                if header_option == Some(false) {
                    return Err("header and no-header options conflict".into());
                }
                header = true;
                header_option = Some(true);
            }
            "no-header" | "noheader" => {
                if header_option == Some(true) {
                    return Err("header and no-header options conflict".into());
                }
                header = false;
                header_option = Some(false);
            }
            "comma" => set_delimiter(&mut delimiter, &mut delimiter_option, Delimiter::Comma)?,
            "tab" => set_delimiter(&mut delimiter, &mut delimiter_option, Delimiter::Tab)?,
            "semicolon" => {
                set_delimiter(&mut delimiter, &mut delimiter_option, Delimiter::Semicolon)?
            }
            "pipe" => set_delimiter(&mut delimiter, &mut delimiter_option, Delimiter::Pipe)?,
            unsupported => return Err(format!("unsupported CSV option '{unsupported}'")),
        }
    }
    let input = input
        .strip_prefix(|character: char| character.is_whitespace())
        .unwrap_or(input);
    if input.is_empty() {
        return Err("CSV input cannot be empty".into());
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err("CSV input exceeds 128 KiB".into());
    }
    Ok(Some(CsvRequest {
        delimiter,
        header,
        prefer_tsv: default_delimiter == Delimiter::Tab,
        input: input.to_owned(),
    }))
}

fn set_delimiter(
    delimiter: &mut Delimiter,
    selected: &mut Option<Delimiter>,
    value: Delimiter,
) -> Result<(), String> {
    if selected.is_some_and(|existing| existing != value) {
        return Err("multiple delimiter options conflict".into());
    }
    *delimiter = value;
    *selected = Some(value);
    Ok(())
}

fn parse_csv(input: &str, delimiter: char) -> Result<CsvDocument, String> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut characters = input.chars().peekable();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut after_quote = false;
    let mut pending_record = false;
    let mut quoted_fields = 0;
    let mut embedded_line_breaks = 0;
    let mut cells = 0;

    while let Some(character) = characters.next() {
        if in_quotes {
            match character {
                '"' if characters.peek() == Some(&'"') => {
                    characters.next();
                    push_field_character(&mut field, '"')?;
                }
                '"' => {
                    in_quotes = false;
                    after_quote = true;
                }
                '\r' => {
                    push_field_character(&mut field, '\r')?;
                    if characters.peek() == Some(&'\n') {
                        characters.next();
                        push_field_character(&mut field, '\n')?;
                    }
                    embedded_line_breaks += 1;
                }
                '\n' => {
                    push_field_character(&mut field, '\n')?;
                    embedded_line_breaks += 1;
                }
                value => push_field_character(&mut field, value)?,
            }
            continue;
        }

        if after_quote {
            if character == delimiter {
                finish_field(&mut row, &mut field, &mut cells)?;
                after_quote = false;
                pending_record = true;
            } else if matches!(character, '\r' | '\n') {
                finish_field(&mut row, &mut field, &mut cells)?;
                finish_row(&mut rows, &mut row)?;
                after_quote = false;
                pending_record = false;
                if character == '\r' && characters.peek() == Some(&'\n') {
                    characters.next();
                }
            } else {
                return Err("unexpected character after a closing CSV quote".into());
            }
            continue;
        }

        if character == delimiter {
            finish_field(&mut row, &mut field, &mut cells)?;
            pending_record = true;
        } else if matches!(character, '\r' | '\n') {
            finish_field(&mut row, &mut field, &mut cells)?;
            finish_row(&mut rows, &mut row)?;
            pending_record = false;
            if character == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
        } else if character == '"' {
            if !field.is_empty() {
                return Err("a quoted CSV field must begin with a quote".into());
            }
            in_quotes = true;
            pending_record = true;
            quoted_fields += 1;
        } else {
            push_field_character(&mut field, character)?;
            pending_record = true;
        }
    }

    if in_quotes {
        return Err("CSV input ends inside a quoted field".into());
    }
    if pending_record || after_quote || !row.is_empty() {
        finish_field(&mut row, &mut field, &mut cells)?;
        finish_row(&mut rows, &mut row)?;
    }
    if rows.is_empty() {
        return Err("CSV input contains no records".into());
    }
    Ok(CsvDocument {
        rows,
        quoted_fields,
        embedded_line_breaks,
    })
}

fn push_field_character(field: &mut String, character: char) -> Result<(), String> {
    if field.len() + character.len_utf8() > MAX_FIELD_BYTES {
        return Err("CSV field exceeds 64 KiB".into());
    }
    field.push(character);
    Ok(())
}

fn finish_field(
    row: &mut Vec<String>,
    field: &mut String,
    cells: &mut usize,
) -> Result<(), String> {
    if row.len() == MAX_COLUMNS {
        return Err("CSV record exceeds 100 columns".into());
    }
    if *cells == MAX_CELLS {
        return Err("CSV input exceeds 20,000 cells".into());
    }
    row.push(std::mem::take(field));
    *cells += 1;
    Ok(())
}

fn finish_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) -> Result<(), String> {
    if rows.len() == MAX_ROWS {
        return Err("CSV input exceeds 1,000 records".into());
    }
    rows.push(std::mem::take(row));
    Ok(())
}

fn inspect(request: &CsvRequest) -> Result<CsvInspection, String> {
    let document = parse_csv(&request.input, request.delimiter.character())?;
    let expected_columns = document.rows[0].len();
    let maximum_columns = document.rows.iter().map(Vec::len).max().unwrap_or(0);
    let inconsistent: Vec<usize> = document
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.len() != expected_columns)
        .map(|(index, _)| index + 1)
        .collect();
    let data_rows = if request.header {
        &document.rows[1..]
    } else {
        &document.rows[..]
    };
    let profiles = column_profiles(&document.rows, request.header, maximum_columns);
    let csv = encode_delimited(&document.rows, ',');
    let tsv = encode_delimited(&document.rows, '\t');
    let json = encode_json(&document.rows, request.header, maximum_columns)?;
    for (label, output) in [("CSV", &csv), ("TSV", &tsv), ("JSON", &json)] {
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(format!("normalized {label} output exceeds 1 MiB"));
        }
    }
    let shape = if inconsistent.is_empty() {
        format!("{} columns, consistent", expected_columns)
    } else {
        format!(
            "{}–{} columns, {} inconsistent record{}",
            document.rows.iter().map(Vec::len).min().unwrap_or(0),
            maximum_columns,
            inconsistent.len(),
            if inconsistent.len() == 1 { "" } else { "s" }
        )
    };
    let report = build_report(
        request,
        &document,
        data_rows.len(),
        &profiles,
        &inconsistent,
    );
    Ok(CsvInspection {
        title: format!(
            "{} · {} row{} × {} column{}",
            if request.prefer_tsv { "TSV" } else { "CSV" },
            data_rows.len(),
            if data_rows.len() == 1 { "" } else { "s" },
            maximum_columns,
            if maximum_columns == 1 { "" } else { "s" }
        ),
        subtitle: format!(
            "{} · {} · {shape}",
            request.delimiter.label().trim(),
            if request.header {
                "header"
            } else {
                "no header"
            }
        ),
        report,
        primary_output: if request.prefer_tsv {
            tsv.clone()
        } else {
            csv.clone()
        },
        csv,
        tsv,
        json,
    })
}

fn column_profiles(
    rows: &[Vec<String>],
    header: bool,
    maximum_columns: usize,
) -> Vec<ColumnProfile> {
    let data_rows = if header { &rows[1..] } else { rows };
    (0..maximum_columns)
        .map(|column| {
            let label = if header {
                rows[0]
                    .get(column)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| compact(&safe_text(value), 80))
                    .unwrap_or_else(|| format!("Column {}", column + 1))
            } else {
                format!("Column {}", column + 1)
            };
            let mut profile = ColumnProfile {
                label,
                present: 0,
                missing: 0,
                empty: 0,
                maximum_characters: 0,
                kinds: BTreeSet::new(),
            };
            for row in data_rows {
                let Some(value) = row.get(column) else {
                    profile.missing += 1;
                    continue;
                };
                profile.present += 1;
                profile.maximum_characters = profile.maximum_characters.max(value.chars().count());
                let value = value.trim();
                if value.is_empty() {
                    profile.empty += 1;
                } else {
                    profile.kinds.insert(value_kind(value));
                }
            }
            profile
        })
        .collect()
}

fn value_kind(value: &str) -> ValueKind {
    if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        ValueKind::Boolean
    } else if value.parse::<i128>().is_ok() {
        ValueKind::Integer
    } else if value.parse::<f64>().is_ok_and(f64::is_finite) {
        ValueKind::Decimal
    } else {
        ValueKind::Text
    }
}

fn profile_kind(kinds: &BTreeSet<ValueKind>) -> &'static str {
    if kinds.is_empty() {
        "Empty"
    } else if kinds.len() == 1 {
        match kinds.first().unwrap() {
            ValueKind::Boolean => "Boolean",
            ValueKind::Integer => "Integer",
            ValueKind::Decimal => "Decimal",
            ValueKind::Text => "Text",
        }
    } else if kinds
        .iter()
        .all(|kind| matches!(kind, ValueKind::Integer | ValueKind::Decimal))
    {
        "Number"
    } else {
        "Mixed"
    }
}

fn build_report(
    request: &CsvRequest,
    document: &CsvDocument,
    data_row_count: usize,
    profiles: &[ColumnProfile],
    inconsistent: &[usize],
) -> String {
    let mut lines = vec![
        "Delimited-data inspection".into(),
        format!("Delimiter: {}", request.delimiter.label().trim()),
        format!("Header row: {}", if request.header { "Yes" } else { "No" }),
        format!("Data rows: {data_row_count}"),
        format!("Total records: {}", document.rows.len()),
        format!("Maximum columns: {}", profiles.len()),
        format!("Quoted fields: {}", document.quoted_fields),
        format!("Embedded line breaks: {}", document.embedded_line_breaks),
    ];
    if inconsistent.is_empty() {
        lines.push("Record widths: consistent".into());
    } else {
        let displayed = inconsistent
            .iter()
            .take(MAX_INCONSISTENT_ROWS)
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "Inconsistent records: {}{}",
            displayed,
            if inconsistent.len() > MAX_INCONSISTENT_ROWS {
                ", …"
            } else {
                ""
            }
        ));
    }
    lines.push(String::new());
    lines.push("Columns".into());
    for (index, profile) in profiles.iter().enumerate() {
        lines.push(format!(
            "{}. {} — {}; present {}, empty {}, missing {}, max {} chars",
            index + 1,
            profile.label,
            profile_kind(&profile.kinds),
            profile.present,
            profile.empty,
            profile.missing,
            profile.maximum_characters
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "Preview (first {} record{})",
        document.rows.len().min(PREVIEW_ROWS),
        if document.rows.len().min(PREVIEW_ROWS) == 1 {
            ""
        } else {
            "s"
        }
    ));
    for row in document.rows.iter().take(PREVIEW_ROWS) {
        let mut values = row
            .iter()
            .take(PREVIEW_COLUMNS)
            .map(|value| compact(&safe_text(value), 40))
            .collect::<Vec<_>>();
        if row.len() > PREVIEW_COLUMNS {
            values.push("…".into());
        }
        lines.push(values.join(" | "));
    }
    lines.join("\n")
}

fn encode_delimited(rows: &[Vec<String>], delimiter: char) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|field| encode_field(field, delimiter))
                .collect::<Vec<_>>()
                .join(&delimiter.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn encode_field(field: &str, delimiter: char) -> String {
    if field.contains(delimiter)
        || field.contains('"')
        || field.contains('\r')
        || field.contains('\n')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

fn encode_json(
    rows: &[Vec<String>],
    header: bool,
    maximum_columns: usize,
) -> Result<String, String> {
    let value = if header {
        let keys = json_header_keys(&rows[0], maximum_columns);
        Value::Array(
            rows[1..]
                .iter()
                .map(|row| {
                    let mut object = Map::new();
                    for (column, key) in keys.iter().enumerate() {
                        object.insert(
                            key.clone(),
                            row.get(column)
                                .map(|value| Value::String(value.clone()))
                                .unwrap_or(Value::Null),
                        );
                    }
                    Value::Object(object)
                })
                .collect(),
        )
    } else {
        Value::Array(
            rows.iter()
                .map(|row| Value::Array(row.iter().cloned().map(Value::String).collect()))
                .collect(),
        )
    };
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("could not encode JSON output: {error}"))
}

fn json_header_keys(header: &[String], maximum_columns: usize) -> Vec<String> {
    let mut used = HashSet::new();
    let mut keys = Vec::with_capacity(maximum_columns);
    for column in 0..maximum_columns {
        let base = header
            .get(column)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("column_{}", column + 1));
        let mut key = base.clone();
        let mut suffix = 2;
        while !used.insert(key.clone()) {
            key = format!("{base}_{suffix}");
            suffix += 1;
        }
        keys.push(key);
    }
    keys
}

fn inspection_result(inspection: &CsvInspection) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: RESULT_ID.into(),
        title: inspection.title.clone(),
        subtitle: inspection.subtitle.clone(),
        preview: inspection.report.clone(),
        kind: ResultKind::Command,
        score: 3_420.0,
        metadata: BTreeMap::from([
            ("symbol_name".into(), "tablecells".into()),
            ("preview_style".into(), "text".into()),
        ]),
        actions: vec![
            action(
                "paste_normalized",
                "Paste Normalized Table",
                true,
                true,
                true,
            ),
            action("copy_csv", "Copy as CSV", false, false, false),
            action("copy_tsv", "Copy as TSV", false, false, false),
            action("copy_json", "Copy as JSON", false, false, false),
            action("copy_report", "Copy Inspection Report", false, false, false),
        ],
    }
}

fn invalid_result(query: &str, message: &str) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "csv-inspector:invalid".into(),
        title: compact(&safe_text(query.trim()), 200),
        subtitle: format!(
            "CSV inspection unavailable · {}",
            compact(&safe_text(message), 120)
        ),
        preview: format!("CSV inspection unavailable\n{}", safe_text(message)),
        kind: ResultKind::Command,
        score: 3_420.0,
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

    fn request(input: &str) -> CsvRequest {
        parse_request(input).unwrap().unwrap()
    }

    #[test]
    fn parses_commands_delimiters_and_header_options() {
        let csv = request("csv :: name,age\nAda,36");
        assert_eq!(csv.delimiter, Delimiter::Comma);
        assert!(csv.header);
        assert!(!csv.prefer_tsv);
        let tsv = request("tsv no-header :: a\tb\n1\t2");
        assert_eq!(tsv.delimiter, Delimiter::Tab);
        assert!(!tsv.header);
        assert!(tsv.prefer_tsv);
        let pipe = request("表格 pipe header :: a|b\n1|2");
        assert_eq!(pipe.delimiter, Delimiter::Pipe);
        assert!(pipe.header);
        assert_eq!(parse_request("ordinary search").unwrap(), None);
    }

    #[test]
    fn rejects_missing_separator_options_empty_and_oversized_input() {
        assert!(parse_request("csv a,b").is_err());
        assert!(parse_request("csv header no-header :: a").is_err());
        assert!(parse_request("csv comma tab :: a").is_err());
        assert!(parse_request("csv mystery :: a").is_err());
        assert!(parse_request("csv ::").is_err());
        assert!(parse_request(&format!("csv :: {}", "a".repeat(MAX_INPUT_BYTES + 1))).is_err());
    }

    #[test]
    fn parses_quotes_escaped_quotes_crlf_and_embedded_lines() {
        let document = parse_csv(
            "name,note\r\n\"Ada, L.\",\"said \"\"hello\"\"\"\r\nBob,\"line 1\r\nline 2\"",
            ',',
        )
        .unwrap();
        assert_eq!(document.rows.len(), 3);
        assert_eq!(document.rows[1], ["Ada, L.", "said \"hello\""]);
        assert_eq!(document.rows[2][1], "line 1\r\nline 2");
        assert_eq!(document.quoted_fields, 3);
        assert_eq!(document.embedded_line_breaks, 1);
    }

    #[test]
    fn handles_empty_trailing_and_blank_records() {
        let document = parse_csv("a,b,\n1,,3\n\n", ',').unwrap();
        assert_eq!(document.rows[0], ["a", "b", ""]);
        assert_eq!(document.rows[1], ["1", "", "3"]);
        assert_eq!(document.rows[2], [""]);
        assert_eq!(document.rows.len(), 3);
    }

    #[test]
    fn rejects_malformed_quotes_and_hard_limits() {
        assert!(parse_csv("a,\"unterminated", ',').is_err());
        assert!(parse_csv("a,\"closed\"tail", ',').is_err());
        assert!(parse_csv("a,b\"ad", ',').is_err());
        assert!(parse_csv(&format!("\"{}\"", "x".repeat(MAX_FIELD_BYTES + 1)), ',').is_err());
        assert!(parse_csv(&vec!["x"; MAX_COLUMNS + 1].join(","), ',').is_err());
    }

    #[test]
    fn reports_profiles_and_inconsistent_rows() {
        let inspection = inspect(&request(
            "csv :: name,age,active,score\nAda,36,true,9.5\nBob,41,false,10\nEve,,true",
        ))
        .unwrap();
        assert!(inspection.title.contains("3 rows × 4 columns"));
        assert!(inspection.subtitle.contains("1 inconsistent record"));
        assert!(inspection.report.contains("2. age — Integer"));
        assert!(inspection.report.contains("3. active — Boolean"));
        assert!(inspection.report.contains("4. score — Number"));
        assert!(inspection.report.contains("missing 1"));
    }

    #[test]
    fn normalized_csv_and_tsv_round_trip_semantics() {
        let rows = vec![
            vec!["name".into(), "note".into()],
            vec!["Ada, L.".into(), "line 1\nline 2".into()],
            vec!["Bob".into(), "said \"hi\"".into()],
        ];
        let csv = encode_delimited(&rows, ',');
        let tsv = encode_delimited(&rows, '\t');
        assert_eq!(parse_csv(&csv, ',').unwrap().rows, rows);
        assert_eq!(parse_csv(&tsv, '\t').unwrap().rows, rows);
        assert!(csv.contains("\"Ada, L.\""));
        assert!(tsv.contains("Ada, L."));
    }

    #[test]
    fn json_uses_unique_headers_nulls_and_array_mode() {
        let rows = vec![
            vec!["name".into(), "name".into(), "".into()],
            vec!["Ada".into(), "Lovelace".into()],
        ];
        let json: Value = serde_json::from_str(&encode_json(&rows, true, 3).unwrap()).unwrap();
        assert_eq!(json[0]["name"], "Ada");
        assert_eq!(json[0]["name_2"], "Lovelace");
        assert!(json[0]["column_3"].is_null());
        let arrays: Value =
            serde_json::from_str(&encode_json(&rows[1..], false, 2).unwrap()).unwrap();
        assert_eq!(arrays[0][0], "Ada");
    }

    #[test]
    fn result_actions_and_primary_format_follow_the_command() {
        let csv = inspect(&request("csv :: a,b\n1,2")).unwrap();
        let tsv = inspect(&request("tsv :: a\tb\n1\t2")).unwrap();
        assert_eq!(csv.primary_output, csv.csv);
        assert_eq!(tsv.primary_output, tsv.tsv);
        let result = inspection_result(&csv);
        assert!(result.actions[0].default);
        assert!(result.actions[0].paste_to_previous);
        assert_eq!(result.actions.len(), 5);
    }

    #[test]
    fn searches_clear_stale_actions_and_escape_preview_controls() {
        let mut plugin = CsvInspectorPlugin::default();
        plugin
            .action_values
            .insert("copy_csv".into(), "stale".into());
        assert!(plugin.search("ordinary search", 10).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        plugin
            .action_values
            .insert("copy_csv".into(), "stale".into());
        assert!(plugin.search("csv :: a,b", 0).unwrap().is_empty());
        assert!(plugin.action_values.is_empty());
        let report = inspect(&request("csv :: value\n\"safe\nspoof\u{202e}\""))
            .unwrap()
            .report;
        assert!(report.contains("safe\\u{000A}spoof\\u{202E}"));
        assert!(!report.contains('\u{202e}'));
    }
}
