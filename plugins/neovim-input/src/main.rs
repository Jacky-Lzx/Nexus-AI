use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{
    ConfigurationField, ConfigurationKind, HostServiceCall, ResultAction, ResultKind, SearchResult,
};
use serde_json::json;
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.neovim-input";

fn main() -> io::Result<()> {
    run_plugin(NeovimInputPlugin::default())
}

#[derive(Default)]
struct NeovimInputPlugin {
    terminal_path: String,
    editor_path: String,
}

impl Plugin for NeovimInputPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn required_services(&self) -> BTreeMap<String, u32> {
        BTreeMap::from([("text.compose".into(), 1)])
    }

    fn configuration_schema(&self) -> Vec<ConfigurationField> {
        [
            ("terminal_path", "Kitty executable"),
            ("editor_path", "Neovim executable"),
        ]
        .into_iter()
        .map(|(id, label)| ConfigurationField {
            id: id.into(),
            label: label.into(),
            kind: ConfigurationKind::Text,
            default: json!(""),
            description: "Leave empty to auto-detect the executable".into(),
            options: vec![],
            minimum: None,
            maximum: None,
        })
        .collect()
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let terminal = settings.get("terminal_path").cloned().unwrap_or_default();
        let editor = settings.get("editor_path").cloned().unwrap_or_default();
        let changed = terminal != self.terminal_path || editor != self.editor_path;
        self.terminal_path = terminal;
        self.editor_path = editor;
        Ok(changed)
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        let keywords = "neovim nvim vim input write compose kitty 输入 写作 编辑";
        if limit == 0 || !query.split_whitespace().all(|word| keywords.contains(word)) {
            return Ok(vec![]);
        }
        Ok(vec![SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: "compose".into(),
            title: "Write in Neovim".into(),
            subtitle: "Save and quit to paste into the original app".into(),
            preview: "Write in a Kitty window at the bottom of the screen. :wq saves and pastes; :q! cancels.".into(),
            kind: ResultKind::Command,
            score: 1000.0,
            metadata: BTreeMap::from([("symbol_name".into(), "square.and.pencil".into())]),
            actions: vec![ResultAction {
                id: "compose".into(),
                title: "Write in Neovim".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: false,
                input_fields: vec![],
                host_action: Some(HostServiceCall {
                    service: "text.compose".into(),
                    parameters: BTreeMap::from([
                        ("terminal_path".into(), json!(self.terminal_path)),
                        ("editor_path".into(), json!(self.editor_path)),
                    ]),
                }),
            }],
        }])
    }

    fn execute(&mut self, _result_id: &str, _action_id: &str) -> Result<Option<String>, String> {
        Err("Neovim Input requires the Nexus text.compose host service".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_declares_composition_service_and_configured_paths() {
        let mut plugin = NeovimInputPlugin::default();
        plugin
            .configure(&BTreeMap::from([
                (
                    "terminal_path".into(),
                    "/Applications/kitty.app/Contents/MacOS/kitty".into(),
                ),
                ("editor_path".into(), "/opt/homebrew/bin/nvim".into()),
            ]))
            .unwrap();
        let results = plugin.search("nvim input", 10).unwrap();
        assert_eq!(results.len(), 1);
        let action = results[0].actions[0].host_action.as_ref().unwrap();
        assert_eq!(action.service, "text.compose");
        assert_eq!(
            action.parameters["editor_path"],
            json!("/opt/homebrew/bin/nvim")
        );
        assert_eq!(plugin.required_services()["text.compose"], 1);
    }

    #[test]
    fn empty_plugin_page_and_chinese_queries_offer_the_command() {
        let mut plugin = NeovimInputPlugin::default();
        assert_eq!(plugin.search("", 10).unwrap().len(), 1);
        assert_eq!(plugin.search("输入", 10).unwrap().len(), 1);
        assert!(plugin.search("calculator", 10).unwrap().is_empty());
        assert!(plugin.search("nvim", 0).unwrap().is_empty());
    }
}
