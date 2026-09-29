use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{
    ActionInputField, ActionInputKind, ResultAction, ResultKind, SearchResult,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const PLUGIN_ID: &str = "io.clipkeep.quick-notes";
const STORAGE_VERSION: u32 = 1;
const MAX_NOTE_BYTES: usize = 64 * 1_024;
const MAX_NOTES: usize = 1_000;

fn main() -> io::Result<()> {
    run_plugin(QuickNotesPlugin::new()?)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Note {
    id: u64,
    content: String,
    created_at: u64,
    updated_at: u64,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct NotesFile {
    version: u32,
    next_id: u64,
    notes: Vec<Note>,
}

struct QuickNotesPlugin {
    notes: Vec<Note>,
    next_id: u64,
    storage_path: PathBuf,
    pending_notes: HashMap<String, String>,
}

impl QuickNotesPlugin {
    fn new() -> io::Result<Self> {
        let storage_path = if let Some(path) = std::env::var_os("NEXUS_QUICK_NOTES_PATH") {
            PathBuf::from(path)
        } else {
            ProjectDirs::from("io", "clipkeep", "ClipKeep")
                .ok_or_else(|| io::Error::other("could not determine quick notes data directory"))?
                .data_dir()
                .join("quick-notes.json")
        };
        Self::from_path(storage_path)
    }

    fn from_path(storage_path: PathBuf) -> io::Result<Self> {
        let stored = match fs::read(&storage_path) {
            Ok(data) => {
                let stored: NotesFile = serde_json::from_slice(&data).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("could not read quick notes: {error}"),
                    )
                })?;
                if stored.version != STORAGE_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported quick notes version: {}", stored.version),
                    ));
                }
                stored
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => NotesFile {
                version: STORAGE_VERSION,
                next_id: 1,
                notes: Vec::new(),
            },
            Err(error) => return Err(error),
        };
        let next_id = stored
            .next_id
            .max(stored.notes.iter().map(|note| note.id).max().unwrap_or(0) + 1);
        Ok(Self {
            notes: stored.notes,
            next_id,
            storage_path,
            pending_notes: HashMap::new(),
        })
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = temporary_path(&self.storage_path);
        let data = serde_json::to_vec(&NotesFile {
            version: STORAGE_VERSION,
            next_id: self.next_id,
            notes: self.notes.clone(),
        })
        .map_err(|error| error.to_string())?;
        fs::write(&temporary, data).map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&temporary, &self.storage_path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
        Ok(())
    }

    fn create(&mut self, content: &str) -> Result<Option<String>, String> {
        let content = validate_content(content)?;
        if self.notes.len() >= MAX_NOTES {
            return Err(format!("quick notes is limited to {MAX_NOTES} entries"));
        }
        let now = now_seconds();
        self.notes.push(Note {
            id: self.next_id,
            content: content.into(),
            created_at: now,
            updated_at: now,
            pinned: false,
        });
        self.next_id += 1;
        self.save()?;
        Ok(Some("Quick note saved".into()))
    }

    fn note_id(result_id: &str) -> Result<u64, String> {
        result_id
            .strip_prefix("quick-note:")
            .ok_or_else(|| "invalid quick note result".to_owned())?
            .parse()
            .map_err(|_| "invalid quick note result".to_owned())
    }

    fn note_position(&self, result_id: &str) -> Result<usize, String> {
        let id = Self::note_id(result_id)?;
        self.notes
            .iter()
            .position(|note| note.id == id)
            .ok_or_else(|| "quick note no longer exists".to_owned())
    }

    fn write_clipboard(&self, result_id: &str) -> Result<Option<String>, String> {
        let note = &self.notes[self.note_position(result_id)?];
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(&note.content))
            .map_err(|error| error.to_string())?;
        Ok(Some("Quick note copied".into()))
    }

    fn toggle_pin(&mut self, result_id: &str) -> Result<Option<String>, String> {
        let position = self.note_position(result_id)?;
        self.notes[position].pinned = !self.notes[position].pinned;
        self.notes[position].updated_at = now_seconds();
        let pinned = self.notes[position].pinned;
        self.save()?;
        Ok(Some(if pinned {
            "Quick note pinned".into()
        } else {
            "Quick note unpinned".into()
        }))
    }

    fn delete(&mut self, result_id: &str) -> Result<Option<String>, String> {
        let position = self.note_position(result_id)?;
        self.notes.remove(position);
        self.save()?;
        Ok(Some("Quick note deleted".into()))
    }
}

impl Plugin for QuickNotesPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.pending_notes.clear();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let original_query = query.trim();
        let Some(request) = parse_request(original_query) else {
            return Ok(Vec::new());
        };
        if let NoteRequest::Save(content) = request {
            if validate_content(content).is_err() {
                return Ok(Vec::new());
            }
            let result_id = "quick-note:save".to_owned();
            self.pending_notes
                .insert(result_id.clone(), content.trim().into());
            return Ok(vec![save_result(original_query, content.trim(), result_id)]);
        }

        let terms = match request {
            NoteRequest::Home => "",
            NoteRequest::Search(terms) => terms,
            NoteRequest::Save(_) => unreachable!(),
        };
        let normalized_terms = terms.to_lowercase();
        let mut matching: Vec<_> = self
            .notes
            .iter()
            .filter(|note| {
                normalized_terms.is_empty()
                    || note.content.to_lowercase().contains(&normalized_terms)
            })
            .collect();
        matching.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| right.id.cmp(&left.id))
        });

        let mut results = Vec::new();
        if matches!(request, NoteRequest::Home) {
            results.push(new_note_result());
        }
        let title_query = matches!(request, NoteRequest::Search(_)).then_some(original_query);
        let remaining = limit.saturating_sub(results.len());
        results.extend(
            matching
                .into_iter()
                .take(remaining)
                .enumerate()
                .map(|(index, note)| note_result(note, index, title_query)),
        );
        Ok(results)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        match (result_id, action_id) {
            ("quick-note:new", "save") => Err("quick note content is required".into()),
            ("quick-note:save", "save") => {
                let content = self
                    .pending_notes
                    .get(result_id)
                    .cloned()
                    .ok_or_else(|| "quick note draft is no longer active".to_owned())?;
                self.create(&content)
            }
            (_, "copy" | "paste") => self.write_clipboard(result_id),
            (_, "pin") => self.toggle_pin(result_id),
            (_, "delete") => self.delete(result_id),
            (_, action) => Err(format!("unknown quick note action: {action}")),
        }
    }

    fn execute_with_input(
        &mut self,
        result_id: &str,
        action_id: &str,
        input: Option<&str>,
    ) -> Result<Option<String>, String> {
        if result_id == "quick-note:new" && action_id == "save" {
            let input: NewNoteInput = serde_json::from_str(
                input.ok_or_else(|| "quick note content is required".to_owned())?,
            )
            .map_err(|error| format!("invalid quick note: {error}"))?;
            return self.create(&input.content);
        }
        self.execute(result_id, action_id)
    }
}

#[derive(Deserialize)]
struct NewNoteInput {
    content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteRequest<'a> {
    Home,
    Save(&'a str),
    Search(&'a str),
}

fn parse_request(query: &str) -> Option<NoteRequest<'_>> {
    let query = query.trim();
    if query.is_empty()
        || query.eq_ignore_ascii_case("note")
        || query.eq_ignore_ascii_case("notes")
        || matches!(query, "笔记" | "便签")
    {
        return Some(NoteRequest::Home);
    }
    for prefix in ["note:", "notes:", "笔记:", "笔记：", "便签:", "便签："] {
        if query
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
        {
            let content = query[prefix.len()..].trim();
            return (!content.is_empty()).then_some(NoteRequest::Save(content));
        }
    }
    for prefix in ["note ", "notes ", "笔记 ", "便签 "] {
        if query
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
        {
            let terms = query[prefix.len()..].trim();
            return (!terms.is_empty()).then_some(NoteRequest::Search(terms));
        }
    }
    None
}

fn validate_content(content: &str) -> Result<&str, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("quick note content cannot be empty".into());
    }
    if content.len() > MAX_NOTE_BYTES {
        return Err(format!(
            "quick note content cannot exceed {MAX_NOTE_BYTES} bytes"
        ));
    }
    Ok(content)
}

fn new_note_result() -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "quick-note:new".into(),
        title: "Note".into(),
        subtitle: "Add a quick note and save it locally".into(),
        preview: "Create a new local quick note".into(),
        kind: ResultKind::Command,
        score: 3_400.0,
        metadata: BTreeMap::from([("symbol_name".into(), "note.text.badge.plus".into())]),
        actions: vec![ResultAction {
            host_action: None,
            id: "save".into(),
            title: "Save Note".into(),
            destructive: false,
            default: true,
            closes_window: false,
            paste_to_previous: false,
            input_fields: vec![ActionInputField {
                id: "content".into(),
                label: "note".into(),
                placeholder: "Write a quick note".into(),
                kind: ActionInputKind::Text,
                required: true,
                minimum: None,
                maximum: None,
            }],
        }],
    }
}

fn save_result(query: &str, content: &str, result_id: String) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title: query.into(),
        subtitle: format!("Save Quick Note — {}", one_line_preview(content, 72)),
        preview: content.into(),
        kind: ResultKind::Command,
        score: 3_500.0,
        metadata: BTreeMap::from([
            ("preview_style".into(), "text".into()),
            ("symbol_name".into(), "square.and.arrow.down".into()),
        ]),
        actions: vec![ResultAction {
            host_action: None,
            id: "save".into(),
            title: "Save Note".into(),
            destructive: false,
            default: true,
            closes_window: false,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

fn note_result(note: &Note, index: usize, title_query: Option<&str>) -> SearchResult {
    let result_id = format!("quick-note:{}", note.id);
    let note_title = one_line_preview(&note.content, 96);
    let title = title_query
        .map(|query| format!("{query} — {note_title}"))
        .unwrap_or(note_title);
    let age = age_label(note.updated_at);
    let subtitle = if note.pinned {
        format!("Pinned · {age}")
    } else {
        age
    };
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: result_id,
        title,
        subtitle,
        preview: note.content.clone(),
        kind: ResultKind::Other,
        score: if note.pinned { 3_200.0 } else { 3_000.0 } - index as f32,
        metadata: BTreeMap::from([
            ("preview_style".into(), "text".into()),
            (
                "symbol_name".into(),
                if note.pinned { "pin.fill" } else { "note.text" }.into(),
            ),
            ("confirmation_title".into(), "Delete Quick Note?".into()),
            (
                "confirmation_message".into(),
                "This note will be permanently removed from local storage.".into(),
            ),
        ]),
        actions: vec![
            ResultAction {
                host_action: None,
                id: "paste".into(),
                title: "Paste".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "copy".into(),
                title: "Copy".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "pin".into(),
                title: if note.pinned { "Unpin" } else { "Pin" }.into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "delete".into(),
                title: "Delete".into(),
                destructive: true,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

fn one_line_preview(content: &str, maximum: usize) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = compact.chars();
    let preview: String = characters.by_ref().take(maximum).collect();
    if characters.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn age_label(timestamp: u64) -> String {
    let elapsed = now_seconds().saturating_sub(timestamp);
    match elapsed {
        0..60 => "Just now".into(),
        60..3_600 => format!("{}m ago", elapsed / 60),
        3_600..86_400 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_else(|| "quick-notes.json".into());
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nexus-quick-notes-{}-{}.json",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn test_plugin() -> QuickNotesPlugin {
        let path = test_path();
        let _ = fs::remove_file(&path);
        QuickNotesPlugin::from_path(path).unwrap()
    }

    fn cleanup(plugin: &QuickNotesPlugin) {
        let _ = fs::remove_file(&plugin.storage_path);
        let _ = fs::remove_file(temporary_path(&plugin.storage_path));
    }

    #[test]
    fn parses_explicit_home_search_and_save_queries() {
        assert_eq!(parse_request("note"), Some(NoteRequest::Home));
        assert_eq!(
            parse_request("notes release"),
            Some(NoteRequest::Search("release"))
        );
        assert_eq!(
            parse_request("note: ship Friday"),
            Some(NoteRequest::Save("ship Friday"))
        );
        assert_eq!(parse_request("ordinary query"), None);
    }

    #[test]
    fn creates_persists_and_reloads_notes() {
        let mut plugin = test_plugin();
        plugin.create("Call Alice").unwrap();
        let reloaded = QuickNotesPlugin::from_path(plugin.storage_path.clone()).unwrap();
        assert_eq!(reloaded.notes.len(), 1);
        assert_eq!(reloaded.notes[0].content, "Call Alice");
        assert_eq!(reloaded.next_id, 2);
        cleanup(&plugin);
    }

    #[test]
    fn save_command_is_invalidated_by_the_next_search() {
        let mut plugin = test_plugin();
        let result = plugin.search("note: remember this", 20).unwrap().remove(0);
        assert_eq!(result.id, "quick-note:save");
        plugin.execute(&result.id, "save").unwrap();
        assert_eq!(plugin.notes[0].content, "remember this");

        plugin.search("notes", 20).unwrap();
        assert!(plugin.execute("quick-note:save", "save").is_err());
        cleanup(&plugin);
    }

    #[test]
    fn searches_only_after_a_note_prefix() {
        let mut plugin = test_plugin();
        plugin.create("Release checklist").unwrap();
        assert!(plugin.search("release", 20).unwrap().is_empty());
        let results = plugin.search("notes release", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "notes release — Release checklist");
        cleanup(&plugin);
    }

    #[test]
    fn pinned_notes_sort_first_and_can_be_deleted() {
        let mut plugin = test_plugin();
        plugin.create("First").unwrap();
        plugin.create("Second").unwrap();
        plugin.toggle_pin("quick-note:1").unwrap();
        let results = plugin.search("notes", 20).unwrap();
        assert_eq!(results[1].title, "First");
        assert!(results[1].subtitle.starts_with("Pinned"));
        plugin.delete("quick-note:1").unwrap();
        assert_eq!(plugin.notes.len(), 1);
        cleanup(&plugin);
    }

    #[test]
    fn add_and_note_results_declare_safe_actions() {
        let add = new_note_result();
        assert_eq!(add.actions[0].input_fields[0].id, "content");
        assert!(add.actions[0].input_fields[0].required);

        let result = note_result(
            &Note {
                id: 1,
                content: "A local note".into(),
                created_at: 1,
                updated_at: 1,
                pinned: false,
            },
            0,
            None,
        );
        assert_eq!(result.actions[0].id, "paste");
        assert!(result.actions[0].paste_to_previous);
        assert!(result.actions.last().unwrap().destructive);
    }

    #[test]
    fn previews_are_unicode_safe_and_content_is_bounded() {
        assert_eq!(one_line_preview("你好\n世界", 3), "你好 …");
        assert!(validate_content("  ").is_err());
        assert!(validate_content(&"x".repeat(MAX_NOTE_BYTES + 1)).is_err());
    }
}
