use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

#[derive(Default)]
struct Hello { greeting: String }
impl Plugin for Hello {
    fn id(&self) -> &'static str {
        "com.example.nexus.hello"
    }

    fn configuration_schema(&self) -> Vec<clipkeep_protocol::ConfigurationField> {
        vec![clipkeep_protocol::ConfigurationField {
            id: "greeting".into(), label: "Greeting".into(),
            kind: clipkeep_protocol::ConfigurationKind::Text,
            default: "Hello".into(), description: "Text shown in search results".into(),
            options: Vec::new(), minimum: None, maximum: None,
        }]
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let greeting = settings.get("greeting").cloned().unwrap_or_else(|| "Hello".into());
        let changed = greeting != self.greeting;
        self.greeting = greeting;
        Ok(changed)
    }

    fn search(&mut self, query: &str, _limit: usize) -> Result<Vec<SearchResult>, String> {
        if !"hello plugin".contains(&query.to_lowercase()) {
            return Ok(Vec::new());
        }
        Ok(vec![SearchResult {
            plugin_id: self.id().into(),
            id: "hello".into(),
            title: format!("{} from a plugin", if self.greeting.is_empty() { "Hello" } else { &self.greeting }),
            subtitle: "Installed without rebuilding Nexus".into(),
            preview: String::new(),
            kind: ResultKind::Command,
            score: 4000.0,
            metadata: BTreeMap::from([("symbol_name".into(), "hand.wave".into())]),
            actions: vec![ResultAction {
                host_action: None,
                id: "hello".into(),
                title: "Say Hello".into(),
                destructive: false,
                default: true,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            }],
        }])
    }

    fn execute(&mut self, _result_id: &str, _action_id: &str) -> Result<Option<String>, String> {
        Ok(Some("Hello! This plugin runs in its own process.".into()))
    }
}

fn main() -> io::Result<()> {
    run_plugin(Hello::default())
}
