use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf, time::SystemTime};

const PLUGIN_ID: &str = "io.clipkeep.clipboard";
const MAX_HISTORY: usize = 500;

fn main() -> io::Result<()> {
    run_plugin(ClipboardPlugin::new()?)
}

struct ClipboardPlugin {
    clipboard: arboard::Clipboard,
    items: Vec<ClipItem>,
    next_id: u64,
    last_text: Option<String>,
    history_path: PathBuf,
}

impl ClipboardPlugin {
    fn new() -> io::Result<Self> {
        let history_path = history_path()
            .ok_or_else(|| io::Error::other("could not determine clipboard data directory"))?;
        let items: Vec<ClipItem> = fs::read(&history_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let next_id = items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        let clipboard = arboard::Clipboard::new().map_err(io::Error::other)?;
        Ok(Self {
            clipboard,
            items,
            next_id,
            last_text: None,
            history_path,
        })
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.history_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temp = self.history_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.items).map_err(|error| error.to_string())?;
        fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        fs::rename(temp, &self.history_path).map_err(|error| error.to_string())
    }

    fn item_id(raw: &str) -> Result<u64, String> {
        raw.strip_prefix("clipboard:")
            .ok_or_else(|| "invalid clipboard result id".to_owned())?
            .parse()
            .map_err(|_| "invalid clipboard result id".to_owned())
    }
}

impl Plugin for ClipboardPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.to_lowercase();
        Ok(self
            .items
            .iter()
            .filter(|item| query.is_empty() || item.text.to_lowercase().contains(&query))
            .take(limit)
            .enumerate()
            .map(|(index, item)| item.as_result(index, &query))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let id = Self::item_id(result_id)?;
        match action_id {
            "copy" => {
                let text = self
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.text.clone())
                    .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
                self.clipboard
                    .set_text(text.clone())
                    .map_err(|error| error.to_string())?;
                self.last_text = Some(text);
                Ok(Some("Copied to clipboard".to_owned()))
            }
            "favorite" => {
                let favorite = {
                    let item = self
                        .items
                        .iter_mut()
                        .find(|item| item.id == id)
                        .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
                    item.favorite = !item.favorite;
                    item.favorite
                };
                self.save()?;
                Ok(Some(
                    if favorite { "Favorited" } else { "Unfavorited" }.to_owned(),
                ))
            }
            "delete" => {
                self.items.retain(|item| item.id != id);
                self.save()?;
                Ok(Some("Deleted".to_owned()))
            }
            _ => Err(format!("unknown clipboard action: {action_id}")),
        }
    }

    fn tick(&mut self) -> Result<bool, String> {
        let Ok(text) = self.clipboard.get_text() else {
            return Ok(false);
        };
        if text.trim().is_empty() || self.last_text.as_ref() == Some(&text) {
            return Ok(false);
        }
        self.last_text = Some(text.clone());
        if let Some(position) = self.items.iter().position(|item| item.text == text) {
            let mut item = self.items.remove(position);
            item.created_at = now_secs();
            self.items.insert(0, item);
        } else {
            self.items.insert(0, ClipItem::new(self.next_id, text));
            self.next_id += 1;
        }
        trim_history(&mut self.items);
        self.save()?;
        Ok(true)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClipItem {
    id: u64,
    text: String,
    created_at: u64,
    #[serde(default)]
    favorite: bool,
}

impl ClipItem {
    fn new(id: u64, text: String) -> Self {
        Self {
            id,
            text,
            created_at: now_secs(),
            favorite: false,
        }
    }

    fn as_result(&self, index: usize, query: &str) -> SearchResult {
        let mut metadata = BTreeMap::new();
        metadata.insert("characters".into(), self.text.chars().count().to_string());
        metadata.insert(
            "words".into(),
            self.text.split_whitespace().count().to_string(),
        );
        metadata.insert("copied".into(), age_label(self.created_at));
        metadata.insert("favorite".into(), self.favorite.to_string());
        let match_bonus = if !query.is_empty() && self.text.to_lowercase().starts_with(query) {
            50.0
        } else {
            0.0
        };
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("clipboard:{}", self.id),
            title: preview(&self.text, 60),
            subtitle: age_label(self.created_at),
            preview: self.text.clone(),
            kind: ResultKind::ClipboardText,
            score: 1_000.0 - index as f32 + match_bonus + if self.favorite { 10.0 } else { 0.0 },
            metadata,
            actions: vec![
                ResultAction {
                    id: "copy".into(),
                    title: "Copy".into(),
                    destructive: false,
                },
                ResultAction {
                    id: "favorite".into(),
                    title: "Favorite".into(),
                    destructive: false,
                },
                ResultAction {
                    id: "delete".into(),
                    title: "Delete".into(),
                    destructive: true,
                },
            ],
        }
    }
}

fn history_path() -> Option<PathBuf> {
    ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .map(|dirs| dirs.data_local_dir().join("history.json"))
}

fn trim_history(items: &mut Vec<ClipItem>) {
    let mut index = items.len();
    while items.len() > MAX_HISTORY && index > 0 {
        index -= 1;
        if !items[index].favorite {
            items.remove(index);
        }
    }
}

fn preview(text: &str, max_chars: usize) -> String {
    let cleaned = text.replace('\r', "").trim().to_owned();
    let mut chars = cleaned.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn age_label(created_at: u64) -> String {
    let elapsed = now_secs().saturating_sub(created_at);
    match elapsed {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_unicode_safe() {
        assert_eq!(preview("你好，剪贴板历史", 4), "你好，剪…");
    }
}
