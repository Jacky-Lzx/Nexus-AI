mod command;
pub use command::command_output;
mod scheduling;
mod transport;
use clipkeep_protocol::{
    ConfigurationField, ConfigurationItem, HostRequest, PROTOCOL_VERSION, PluginEvent,
    PluginNotification, PluginPresentationItem, SearchResult,
};
pub use scheduling::SearchContext;
use scheduling::{ReaderMessage, RequestQueue};
use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Write},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub const SDK_FEATURES: &[&str] = &[
    "search.cancel",
    "action.arguments",
    "configuration.values",
    "host_action.object",
    "status.snapshots",
];

#[derive(Default)]
struct Session {
    initialized: bool,
    host_features: Vec<String>,
    last_status: Option<Vec<PluginPresentationItem>>,
}

pub trait Plugin {
    fn required_features(&self) -> Vec<String> {
        Vec::new()
    }
    fn required_services(&self) -> BTreeMap<String, u32> {
        BTreeMap::new()
    }

    fn id(&self) -> &'static str;

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String>;

    fn search_with_context(
        &mut self,
        query: &str,
        limit: usize,
        context: &SearchContext,
    ) -> Result<Vec<SearchResult>, String> {
        context.check()?;
        self.search(query, limit)
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String>;

    fn execute_with_input(
        &mut self,
        result_id: &str,
        action_id: &str,
        _input: Option<&str>,
    ) -> Result<Option<String>, String> {
        self.execute(result_id, action_id)
    }

    fn execute_with_arguments(
        &mut self,
        result_id: &str,
        action_id: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<String>, String> {
        let input = serde_json::to_string(arguments).map_err(|e| e.to_string())?;
        self.execute_with_input(result_id, action_id, Some(&input))
    }

    fn configuration_schema(&self) -> Vec<ConfigurationField> {
        Vec::new()
    }

    fn configure_values(
        &mut self,
        values: &BTreeMap<String, serde_json::Value>,
    ) -> Result<bool, String> {
        let settings = values
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                )
            })
            .collect();
        self.configure(&settings)
    }

    fn configure(&mut self, _settings: &BTreeMap<String, String>) -> Result<bool, String> {
        Ok(false)
    }

    fn configuration_items(&self) -> Vec<ConfigurationItem> {
        Vec::new()
    }

    fn presentation_items(&self) -> Vec<PluginPresentationItem> {
        Vec::new()
    }

    fn notifications(&mut self) -> Result<Vec<PluginNotification>, String> {
        Ok(Vec::new())
    }

    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    fn tick(&mut self) -> Result<bool, String> {
        Ok(false)
    }
}

pub fn run_plugin(mut plugin: impl Plugin) -> io::Result<()> {
    let requests = request_reader();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut running = true;
    let mut session = Session::default();
    let mut poll_interval = None;
    let mut next_poll = None;

    while running {
        let message = requests.receive(next_poll);

        if let Some(message) = message {
            match message {
                ReaderMessage::Request(request, context) => {
                    running = handle_request(
                        &mut plugin,
                        request,
                        context.as_ref(),
                        &mut session,
                        &mut output,
                    )?;
                }
                ReaderMessage::Invalid(message) => {
                    send(&mut output, &PluginEvent::error(message))?;
                }
                ReaderMessage::Closed => return Ok(()),
            }
            let updated_interval = if session.initialized {
                plugin.poll_interval()
            } else {
                None
            };
            if updated_interval != poll_interval {
                poll_interval = updated_interval;
                next_poll = poll_interval.map(|interval| Instant::now() + interval);
            }
        } else if running {
            match plugin.tick() {
                Ok(true) => send(&mut output, &PluginEvent::CatalogChanged)?,
                Ok(false) => {}
                Err(message) => send(&mut output, &PluginEvent::error(message))?,
            }
            match plugin.notifications() {
                Ok(notifications) => {
                    for notification in notifications {
                        send(&mut output, &PluginEvent::Notification { notification })?;
                    }
                }
                Err(message) => send(&mut output, &PluginEvent::error(message))?,
            }
            poll_interval = plugin.poll_interval();
            next_poll = poll_interval.map(|interval| Instant::now() + interval);
        }
        if running && session.initialized {
            emit_status_if_changed(&plugin, &mut session, &mut output)?;
        }
    }
    Ok(())
}

fn emit_status_if_changed(
    plugin: &impl Plugin,
    session: &mut Session,
    output: &mut impl Write,
) -> io::Result<()> {
    if !session
        .host_features
        .iter()
        .any(|feature| feature == "status.snapshots")
    {
        return Ok(());
    }
    let items: Vec<_> = plugin.presentation_items().into_iter().take(64).collect();
    if session.last_status.as_ref() != Some(&items) {
        send(
            output,
            &PluginEvent::StatusChanged {
                presentation_items: items.clone(),
            },
        )?;
        session.last_status = Some(items);
    }
    Ok(())
}

fn handle_request(
    plugin: &mut impl Plugin,
    request: HostRequest,
    context: Option<&SearchContext>,
    session: &mut Session,
    output: &mut impl Write,
) -> io::Result<bool> {
    if !session.initialized
        && !matches!(
            request,
            HostRequest::Initialize { .. } | HostRequest::Shutdown
        )
    {
        send(
            output,
            &PluginEvent::error("initialize must be the first request"),
        )?;
        return Ok(false);
    }
    match request {
        HostRequest::Initialize {
            protocol_version,
            host_features,
            host_services,
        } => {
            if session.initialized {
                send(output, &PluginEvent::error("duplicate initialize"))?;
                return Ok(false);
            }
            if plugin
                .required_features()
                .iter()
                .any(|feature| !host_features.contains(feature))
                || plugin
                    .required_services()
                    .iter()
                    .any(|(service, version)| host_services.get(service) != Some(version))
            {
                send(
                    output,
                    &PluginEvent::error(
                        "host does not support this plugin's required features or service versions",
                    ),
                )?;
                return Ok(false);
            }
            if protocol_version != PROTOCOL_VERSION {
                send(
                    output,
                    &PluginEvent::error(format!(
                        "unsupported protocol version {protocol_version}; expected {PROTOCOL_VERSION}"
                    )),
                )?;
                return Ok(false);
            } else {
                session.initialized = true;
                session.host_features = host_features;
                send(
                    output,
                    &PluginEvent::Initialized {
                        plugin_id: plugin.id().to_owned(),
                        protocol_version: PROTOCOL_VERSION,
                        features: SDK_FEATURES.iter().map(|s| (*s).into()).collect(),
                        required_features: plugin.required_features(),
                        required_services: plugin.required_services(),
                        configuration_schema: plugin.configuration_schema(),
                    },
                )?;
            }
        }
        HostRequest::Configure { settings, values } => {
            match configure_plugin(plugin, &settings, values.as_ref()) {
                Ok(true) => send(output, &PluginEvent::CatalogChanged)?,
                Ok(false) => {}
                Err(message) => send(output, &PluginEvent::error(message))?,
            }
        }
        HostRequest::Search {
            query_id,
            query,
            limit,
            timeout_ms,
        } => {
            let fallback = SearchContext::new(query_id, timeout_ms);
            let context = context.unwrap_or(&fallback);
            let limit = limit.min(100);
            let result = plugin.search_with_context(&query, limit, context);
            let result = context.check().and(result).and_then(|mut items| {
                if !session
                    .host_features
                    .iter()
                    .any(|f| f == "host_action.object")
                {
                    adapt_legacy_host_actions(&mut items)?;
                }
                items.truncate(limit);
                Ok(items)
            });
            match result {
                Ok(items) => send(
                    output,
                    &PluginEvent::SearchResults {
                        query_id,
                        items,
                        presentation_items: plugin.presentation_items(),
                    },
                )?,
                Err(message) => send(
                    output,
                    &PluginEvent::Error {
                        message,
                        query_id: Some(query_id),
                        request_id: None,
                        code: Some("search_failed".into()),
                        retryable: false,
                    },
                )?,
            }
        }
        HostRequest::ListConfigurationItems { request_id } => send(
            output,
            &PluginEvent::ConfigurationItems {
                request_id,
                configuration_items: plugin.configuration_items(),
            },
        )?,
        HostRequest::Execute {
            request_id,
            result_id,
            action_id,
            input,
            arguments,
        } => {
            let outcome = match arguments.as_ref() {
                Some(values) => plugin.execute_with_arguments(&result_id, &action_id, values),
                None => plugin.execute_with_input(&result_id, &action_id, input.as_deref()),
            };
            let (success, message) = match outcome {
                Ok(message) => (true, message),
                Err(message) => (false, Some(message)),
            };
            send(
                output,
                &PluginEvent::ActionFinished {
                    request_id,
                    success,
                    message,
                },
            )?;
        }
        HostRequest::CancelSearch { .. } => {}
        HostRequest::Shutdown => return Ok(false),
    }
    Ok(true)
}

fn adapt_legacy_host_actions(items: &mut [SearchResult]) -> Result<(), String> {
    for item in items {
        let mut parameters = BTreeMap::new();
        for action in &mut item.actions {
            if let Some(call) = action.host_action.take() {
                item.metadata
                    .insert(format!("host_action.{}", action.id), call.service);
                for (key, value) in call.parameters {
                    let value = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    if parameters.get(&key).is_some_and(|old| old != &value) {
                        return Err("This result needs a host supporting host_action.object".into());
                    }
                    parameters.insert(key, value);
                }
            }
        }
        item.metadata.extend(parameters);
    }
    Ok(())
}

fn configure_plugin(
    plugin: &mut impl Plugin,
    settings: &BTreeMap<String, String>,
    values: Option<&BTreeMap<String, serde_json::Value>>,
) -> Result<bool, String> {
    let schema = plugin.configuration_schema();
    if schema.is_empty() && values.is_none() {
        return plugin.configure(settings);
    }
    let mut normalized = values.cloned().unwrap_or_default();
    for field in &schema {
        if normalized.contains_key(&field.id) {
            continue;
        }
        let value = match settings.get(&field.id) {
            Some(text) => match field.kind {
                clipkeep_protocol::ConfigurationKind::Text
                | clipkeep_protocol::ConfigurationKind::Choice => {
                    serde_json::Value::String(text.clone())
                }
                _ => serde_json::from_str(text)
                    .map_err(|_| format!("Invalid setting: {}", field.label))?,
            },
            None => field.default.clone(),
        };
        normalized.insert(field.id.clone(), value);
    }
    let values = &normalized;
    for field in &schema {
        if let Some(value) = values.get(&field.id)
            && !field.accepts(value)
        {
            return Err(format!("Invalid setting: {}", field.label));
        }
    }
    let mut merged: BTreeMap<_, _> = settings
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    for field in schema {
        merged.entry(field.id).or_insert(field.default);
    }
    merged.extend(values.clone());
    plugin.configure_values(&merged)
}

fn send(output: &mut impl Write, event: &PluginEvent) -> io::Result<()> {
    let mut frame = transport::LimitedFrame {
        bytes: Vec::new(),
        limit: transport::MAX_MESSAGE_BYTES,
    };
    serde_json::to_writer(&mut frame, event).map_err(io::Error::other)?;
    output.write_all(&frame.bytes)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn request_reader() -> Arc<RequestQueue> {
    let queue = Arc::new(RequestQueue::default());
    let reader_queue = queue.clone();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        loop {
            match transport::read_frame(&mut reader, transport::MAX_MESSAGE_BYTES) {
                Ok(Some(line)) if line.is_empty() => continue,
                Ok(Some(line)) => match serde_json::from_slice(&line) {
                    Ok(request) => {
                        if !reader_queue.push_sized(request, line.len()) {
                            return;
                        }
                    }
                    Err(error) => {
                        reader_queue.invalid(format!(
                            "invalid request at line {} column {}",
                            error.line(),
                            error.column()
                        ));
                        break;
                    }
                },
                Ok(None) => break,
                Err(error) => {
                    reader_queue.invalid(error.to_string());
                    break;
                }
            }
        }
        reader_queue.close();
    });
    queue
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipkeep_protocol::PresentationStyle;
    struct Failing;
    impl Plugin for Failing {
        fn id(&self) -> &'static str {
            "test"
        }
        fn search(&mut self, _: &str, _: usize) -> Result<Vec<SearchResult>, String> {
            Err("failed".into())
        }
        fn execute(&mut self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
    }
    #[test]
    fn search_failure_keeps_query_identity() {
        let mut output = Vec::new();
        handle_request(
            &mut Failing,
            HostRequest::Search {
                query_id: 42,
                query: "x".into(),
                limit: 1,
                timeout_ms: None,
            },
            None,
            &mut Session {
                initialized: true,
                host_features: Vec::new(),
                last_status: None,
            },
            &mut output,
        )
        .unwrap();
        let event: PluginEvent = serde_json::from_slice(&output).unwrap();
        assert!(matches!(
            event,
            PluginEvent::Error {
                query_id: Some(42),
                ..
            }
        ));
    }
    #[test]
    fn legacy_error_still_decodes() {
        let event: PluginEvent =
            serde_json::from_str(r#"{"type":"error","message":"old"}"#).unwrap();
        assert!(matches!(
            event,
            PluginEvent::Error {
                query_id: None,
                request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn status_snapshots_emit_only_when_negotiated_and_changed() {
        struct StatusPlugin(bool);
        impl Plugin for StatusPlugin {
            fn id(&self) -> &'static str {
                "test"
            }
            fn search(&mut self, _: &str, _: usize) -> Result<Vec<SearchResult>, String> {
                Ok(vec![])
            }
            fn execute(&mut self, _: &str, _: &str) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn presentation_items(&self) -> Vec<PluginPresentationItem> {
                if !self.0 {
                    return vec![];
                }
                vec![PluginPresentationItem {
                    id: "active".into(),
                    title: "Active".into(),
                    value: "Running".into(),
                    compact_value: None,
                    style: PresentationStyle::Success,
                    metadata: BTreeMap::new(),
                }]
            }
        }
        let mut output = Vec::new();
        let mut session = Session {
            initialized: true,
            ..Session::default()
        };
        let mut plugin = StatusPlugin(true);
        emit_status_if_changed(&plugin, &mut session, &mut output).unwrap();
        assert!(output.is_empty());
        session.host_features.push("status.snapshots".into());
        emit_status_if_changed(&plugin, &mut session, &mut output).unwrap();
        emit_status_if_changed(&plugin, &mut session, &mut output).unwrap();
        plugin.0 = false;
        emit_status_if_changed(&plugin, &mut session, &mut output).unwrap();
        let events: Vec<PluginEvent> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], PluginEvent::StatusChanged { presentation_items } if presentation_items.len() == 1)
        );
        assert!(
            matches!(&events[1], PluginEvent::StatusChanged { presentation_items } if presentation_items.is_empty())
        );
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    #[derive(Default)]
    struct Configurable {
        applied: usize,
        input: Option<String>,
    }
    impl Plugin for Configurable {
        fn id(&self) -> &'static str {
            "test"
        }
        fn search(&mut self, _: &str, _: usize) -> Result<Vec<SearchResult>, String> {
            Ok(vec![])
        }
        fn execute(&mut self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn execute_with_input(
            &mut self,
            _: &str,
            _: &str,
            input: Option<&str>,
        ) -> Result<Option<String>, String> {
            self.input = input.map(str::to_owned);
            Ok(None)
        }
        fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
            self.applied = settings["count"].parse().unwrap();
            Ok(false)
        }
        fn configuration_schema(&self) -> Vec<ConfigurationField> {
            vec![ConfigurationField {
                id: "count".into(),
                label: "Count".into(),
                kind: clipkeep_protocol::ConfigurationKind::Integer,
                default: 2.into(),
                description: String::new(),
                options: vec![],
                minimum: Some(1),
                maximum: Some(10),
            }]
        }
    }
    #[test]
    fn validates_before_configuration_side_effects_and_adapts_old_strings() {
        let mut plugin = Configurable::default();
        configure_plugin(&mut plugin, &BTreeMap::new(), Some(&BTreeMap::new())).unwrap();
        assert_eq!(plugin.applied, 2);
        let invalid = BTreeMap::from([("count".into(), 11.into())]);
        assert!(configure_plugin(&mut plugin, &BTreeMap::new(), Some(&invalid)).is_err());
        assert_eq!(plugin.applied, 2);
        configure_plugin(
            &mut plugin,
            &BTreeMap::from([("count".into(), "3".into())]),
            None,
        )
        .unwrap();
        assert_eq!(plugin.applied, 3);
    }
    #[test]
    fn typed_arguments_adapt_to_existing_inline_handlers() {
        let mut plugin = Configurable::default();
        let args = BTreeMap::from([("count".into(), 3.into()), ("flag".into(), true.into())]);
        plugin
            .execute_with_arguments("result", "action", &args)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(plugin.input.as_ref().unwrap()).unwrap(),
            serde_json::json!({"count":3,"flag":true})
        );
    }
}

#[cfg(test)]
mod negotiation_tests {
    use super::*;
    struct Modern;
    impl Plugin for Modern {
        fn id(&self) -> &'static str {
            "modern"
        }
        fn required_features(&self) -> Vec<String> {
            vec!["action.arguments".into()]
        }
        fn search(&mut self, _: &str, _: usize) -> Result<Vec<SearchResult>, String> {
            Ok(vec![])
        }
        fn execute(&mut self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
    }
    #[test]
    fn refuses_missing_required_features_before_ready() {
        let mut output = Vec::new();
        let mut session = Session::default();
        let keep_running = handle_request(
            &mut Modern,
            HostRequest::Initialize {
                protocol_version: 1,
                host_features: vec![],
                host_services: BTreeMap::new(),
            },
            None,
            &mut session,
            &mut output,
        )
        .unwrap();
        assert!(!keep_running);
        assert!(!session.initialized);
        assert!(matches!(
            serde_json::from_slice::<PluginEvent>(&output).unwrap(),
            PluginEvent::Error { .. }
        ));
    }
    #[test]
    fn typed_host_action_has_legacy_wire_adapter() {
        let fixture: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../protocol/fixtures/v1.json")).unwrap();
        let case = fixture
            .iter()
            .find(|v| v["name"] == "typed_results")
            .unwrap();
        let PluginEvent::SearchResults { mut items, .. } =
            serde_json::from_value(case["message"].clone()).unwrap()
        else {
            panic!()
        };
        adapt_legacy_host_actions(&mut items).unwrap();
        assert_eq!(items[0].metadata["host_action.run"], "system.media");
        assert_eq!(items[0].metadata["command"], "mute");
        assert!(items[0].actions[0].host_action.is_none());
    }
}
