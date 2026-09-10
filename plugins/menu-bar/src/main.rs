use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use serde::Deserialize;
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.menu-bar";

fn main() -> io::Result<()> {
    run_plugin(MenuBarPlugin::default())
}

#[derive(Default)]
struct MenuBarPlugin {
    target_pid: i32,
    items: Vec<MenuBarItem>,
}

#[derive(Clone, Debug, Deserialize)]
struct MenuBarItem {
    id: String,
    title: String,
    subtitle: String,
    path: String,
    path_components: Vec<String>,
}

impl Plugin for MenuBarPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        let mut matches = self
            .items
            .iter()
            .filter_map(|item| {
                match_score(&item.title, &query)
                    .or_else(|| match_score(&item.path, &query).map(|score| score - 100.0))
                    .map(|score| (score, item))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| right.0.total_cmp(&left.0));
        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(score, item)| item.as_result(self.target_pid, score))
            .collect())
    }

    fn execute(&mut self, _result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id == "press" {
            Ok(Some("Menu bar item selected".into()))
        } else {
            Err(format!("unknown menu bar action: {action_id}"))
        }
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        self.target_pid = settings
            .get("target_pid")
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        self.items = settings
            .get("items")
            .map(|items| serde_json::from_str(items))
            .transpose()
            .map_err(|error| format!("invalid menu bar items: {error}"))?
            .unwrap_or_default();
        Ok(false)
    }
}

impl MenuBarItem {
    fn as_result(&self, target_pid: i32, score: f32) -> SearchResult {
        let parent_path = self.path.rsplit_once(" › ").map(|(parent, _)| parent);
        let subtitle = parent_path
            .map(|parent| format!("{} · {}", self.subtitle, parent))
            .unwrap_or_else(|| self.subtitle.clone());
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("menu-bar:{}", self.id),
            title: self.title.clone(),
            subtitle,
            preview: self.path.clone(),
            kind: ResultKind::Command,
            score,
            metadata: BTreeMap::from([
                ("menu_item_id".into(), self.id.clone()),
                (
                    "menu_path".into(),
                    serde_json::to_string(&self.path_components).unwrap_or_default(),
                ),
                ("symbol_name".into(), "menubar.rectangle".into()),
                ("target_pid".into(), target_pid.to_string()),
            ]),
            actions: vec![ResultAction {
                id: "press".into(),
                title: "Run Menu Command".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: false,
                input_fields: Vec::new(),
            }],
        }
    }
}

fn match_score(title: &str, query: &str) -> Option<f32> {
    if query.is_empty() {
        return Some(2_000.0);
    }
    let title = title.to_lowercase();
    if title == query {
        Some(3_000.0)
    } else if title.starts_with(query) {
        Some(2_800.0 - title.len() as f32)
    } else {
        title
            .find(query)
            .map(|position| 2_500.0 - position as f32 * 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_menu_items_are_searchable() {
        let mut plugin = MenuBarPlugin::default();
        let settings = BTreeMap::from([
            ("target_pid".into(), "42".into()),
            (
                "items".into(),
                r#"[{"id":"42:1","title":"Open…","subtitle":"TextEdit","path":"File › Open…","path_components":["File","Open…"]}]"#
                    .into(),
            ),
        ]);
        plugin.configure(&settings).unwrap();
        let results = plugin.search("file", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata["target_pid"], "42");
        assert_eq!(results[0].metadata["menu_item_id"], "42:1");
        assert_eq!(results[0].metadata["menu_path"], r#"["File","Open…"]"#);
        assert_eq!(results[0].title, "Open…");
    }
}
