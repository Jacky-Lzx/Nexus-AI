use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostRequest {
    Initialize {
        protocol_version: u32,
    },
    Configure {
        #[serde(default)]
        settings: BTreeMap<String, String>,
    },
    Search {
        query_id: u64,
        query: String,
        limit: usize,
    },
    Execute {
        request_id: u64,
        result_id: String,
        action_id: String,
    },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginEvent {
    Initialized {
        plugin_id: String,
        protocol_version: u32,
    },
    SearchResults {
        query_id: u64,
        items: Vec<SearchResult>,
    },
    CatalogChanged,
    ActionFinished {
        request_id: u64,
        success: bool,
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub preview: String,
    pub kind: ResultKind,
    #[serde(default)]
    pub score: f32,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub actions: Vec<ResultAction>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultKind {
    Application,
    ClipboardText,
    ClipboardRichText,
    ClipboardImage,
    ClipboardFiles,
    Command,
    File,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResultAction {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub closes_window: bool,
    #[serde(default)]
    pub paste_to_previous: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_stable_tagged_json_shape() {
        let request = HostRequest::Search {
            query_id: 7,
            query: "safari".into(),
            limit: 20,
        };
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["type"], "search");
        assert_eq!(json["query_id"], 7);
    }

    #[test]
    fn older_result_actions_get_safe_behavior_defaults() {
        let action: ResultAction =
            serde_json::from_str(r#"{"id":"open","title":"Open","destructive":false}"#).unwrap();
        assert!(!action.default);
        assert!(!action.closes_window);
        assert!(!action.paste_to_previous);
    }

    #[test]
    fn configure_request_carries_plugin_settings() {
        let request: HostRequest =
            serde_json::from_str(r#"{"type":"configure","settings":{"retention_days":"30"}}"#)
                .unwrap();
        assert_eq!(
            request,
            HostRequest::Configure {
                settings: BTreeMap::from([("retention_days".into(), "30".into())]),
            }
        );
    }
}
