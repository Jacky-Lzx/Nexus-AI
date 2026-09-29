use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostRequest {
    Initialize {
        protocol_version: u32,
        #[serde(default)]
        host_features: Vec<String>,
        #[serde(default)]
        host_services: BTreeMap<String, u32>,
    },
    Configure {
        #[serde(default)]
        settings: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        values: Option<BTreeMap<String, serde_json::Value>>,
    },
    Search {
        query_id: u64,
        query: String,
        limit: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    ListConfigurationItems {
        request_id: u64,
    },
    Execute {
        request_id: u64,
        result_id: String,
        action_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<BTreeMap<String, serde_json::Value>>,
    },
    CancelSearch {
        query_id: u64,
    },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginEvent {
    Initialized {
        plugin_id: String,
        protocol_version: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        features: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        configuration_schema: Vec<ConfigurationField>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_features: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        required_services: BTreeMap<String, u32>,
    },
    SearchResults {
        query_id: u64,
        items: Vec<SearchResult>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        presentation_items: Vec<PluginPresentationItem>,
    },
    StatusChanged {
        presentation_items: Vec<PluginPresentationItem>,
    },
    ConfigurationItems {
        request_id: u64,
        configuration_items: Vec<ConfigurationItem>,
    },
    Notification {
        notification: PluginNotification,
    },
    CatalogChanged,
    ActionFinished {
        request_id: u64,
        success: bool,
        message: Option<String>,
    },
    Error {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default)]
        retryable: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginPresentationItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_value: Option<String>,
    #[serde(default)]
    pub style: PresentationStyle,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStyle {
    #[default]
    Info,
    Success,
    Warning,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginNotification {
    pub id: String,
    pub title: String,
    pub body: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_fields: Vec<ActionInputField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_action: Option<HostServiceCall>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionInputField {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub kind: ActionInputKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionInputKind {
    #[default]
    Text,
    Integer,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostServiceCall {
    pub service: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationField {
    pub id: String,
    pub label: String,
    pub kind: ConfigurationKind,
    pub default: serde_json::Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub minimum: Option<i64>,
    #[serde(default)]
    pub maximum: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationKind {
    Text,
    Integer,
    Boolean,
    Choice,
}

impl ConfigurationField {
    pub fn accepts(&self, value: &serde_json::Value) -> bool {
        match self.kind {
            ConfigurationKind::Text => value.is_string(),
            ConfigurationKind::Boolean => value.is_boolean(),
            ConfigurationKind::Choice => value
                .as_str()
                .is_some_and(|s| self.options.iter().any(|o| o == s)),
            ConfigurationKind::Integer => value.as_i64().is_some_and(|n| {
                self.minimum.is_none_or(|min| n >= min) && self.maximum.is_none_or(|max| n <= max)
            }),
        }
    }
}

impl PluginEvent {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            query_id: None,
            request_id: None,
            code: None,
            retryable: false,
        }
    }
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
            timeout_ms: None,
        };
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["type"], "search");
        assert_eq!(json["query_id"], 7);
    }

    #[test]
    fn older_search_results_default_to_no_presentation_items() {
        let event: PluginEvent =
            serde_json::from_str(r#"{"type":"search_results","query_id":1,"items":[]}"#).unwrap();
        assert_eq!(
            event,
            PluginEvent::SearchResults {
                query_id: 1,
                items: Vec::new(),
                presentation_items: Vec::new(),
            }
        );
    }

    #[test]
    fn status_changed_keeps_an_explicit_empty_snapshot() {
        let event = PluginEvent::StatusChanged {
            presentation_items: Vec::new(),
        };
        let wire = serde_json::to_string(&event).unwrap();
        assert_eq!(wire, r#"{"type":"status_changed","presentation_items":[]}"#);
        assert_eq!(serde_json::from_str::<PluginEvent>(&wire).unwrap(), event);
    }

    #[test]
    fn compact_status_is_optional_for_existing_plugins() {
        let old = r#"{"id":"active","title":"Active","value":"Running"}"#;
        let item: PluginPresentationItem = serde_json::from_str(old).unwrap();
        assert_eq!(item.compact_value, None);
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"id":"active","title":"Active","value":"Running","style":"info","metadata":{}}"#
        );
    }

    #[test]
    fn older_result_actions_get_safe_behavior_defaults() {
        let action: ResultAction =
            serde_json::from_str(r#"{"id":"open","title":"Open","destructive":false}"#).unwrap();
        assert!(!action.default);
        assert!(!action.closes_window);
        assert!(!action.paste_to_previous);
        assert!(action.input_fields.is_empty());
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
                values: None,
            }
        );
    }

    #[test]
    fn configuration_items_have_a_stable_wire_shape() {
        let event = PluginEvent::ConfigurationItems {
            request_id: 11,
            configuration_items: vec![ConfigurationItem {
                id: "lock".into(),
                title: "Lock Screen".into(),
                subtitle: String::new(),
                metadata: BTreeMap::new(),
            }],
        };
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "configuration_items");
        assert_eq!(json["request_id"], 11);
        assert_eq!(json["configuration_items"][0]["id"], "lock");
    }

    #[test]
    fn plugin_notifications_have_a_stable_wire_shape() {
        let event = PluginEvent::Notification {
            notification: PluginNotification {
                id: "reminder:1".into(),
                title: "Reminder".into(),
                body: "Call Alice".into(),
            },
        };
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "notification");
        assert_eq!(json["notification"]["body"], "Call Alice");
    }

    #[test]
    fn execute_request_accepts_optional_action_input() {
        let request: HostRequest = serde_json::from_str(
            r#"{"type":"execute","request_id":9,"result_id":"clipboard:1","action_id":"apply_edit","input":"changed"}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            HostRequest::Execute {
                request_id: 9,
                result_id: "clipboard:1".into(),
                action_id: "apply_edit".into(),
                input: Some("changed".into()),
                arguments: None,
            }
        );
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    #[test]
    fn shared_fixtures_roundtrip_without_losing_present_fields() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../protocol/fixtures/v1.json")).unwrap();
        fn contains(actual: &serde_json::Value, expected: &serde_json::Value) {
            match expected {
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        contains(&actual[key], value);
                    }
                }
                serde_json::Value::Array(array) => {
                    assert_eq!(actual.as_array().unwrap().len(), array.len());
                    for (a, b) in actual.as_array().unwrap().iter().zip(array) {
                        contains(a, b);
                    }
                }
                _ => assert_eq!(actual, expected),
            }
        }
        for case in cases {
            let value = case["message"].clone();
            let actual = if case["direction"] == "request" {
                serde_json::to_value(serde_json::from_value::<HostRequest>(value.clone()).unwrap())
                    .unwrap()
            } else {
                serde_json::to_value(serde_json::from_value::<PluginEvent>(value.clone()).unwrap())
                    .unwrap()
            };
            contains(&actual, &value);
        }
    }
}
