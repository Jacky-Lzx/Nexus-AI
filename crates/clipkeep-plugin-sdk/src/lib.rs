mod command;
pub use command::command_output;
mod scheduling;
use clipkeep_protocol::{
    ConfigurationItem, HostRequest, PROTOCOL_VERSION, PluginEvent, PluginNotification,
    PluginPresentationItem, SearchResult,
};
pub use scheduling::SearchContext;
use scheduling::{ReaderMessage, RequestQueue};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub trait Plugin {
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
    let mut poll_interval = plugin.poll_interval();
    let mut next_poll = poll_interval.map(|interval| Instant::now() + interval);

    while running {
        let message = requests.receive(next_poll);

        if let Some(message) = message {
            match message {
                ReaderMessage::Request(request, context) => {
                    running = handle_request(&mut plugin, request, context.as_ref(), &mut output)?;
                }
                ReaderMessage::Invalid(message) => {
                    send(&mut output, &PluginEvent::error(message))?;
                }
                ReaderMessage::Closed => return Ok(()),
            }
            let updated_interval = plugin.poll_interval();
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
    }
    Ok(())
}

fn handle_request(
    plugin: &mut impl Plugin,
    request: HostRequest,
    context: Option<&SearchContext>,
    output: &mut impl Write,
) -> io::Result<bool> {
    match request {
        HostRequest::Initialize { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                send(
                    output,
                    &PluginEvent::error(format!(
                        "unsupported protocol version {protocol_version}; expected {PROTOCOL_VERSION}"
                    )),
                )?;
            } else {
                send(
                    output,
                    &PluginEvent::Initialized {
                        plugin_id: plugin.id().to_owned(),
                        protocol_version: PROTOCOL_VERSION,
                        features: vec!["search.cancel".into()],
                    },
                )?;
            }
        }
        HostRequest::Configure { settings } => match plugin.configure(&settings) {
            Ok(true) => send(output, &PluginEvent::CatalogChanged)?,
            Ok(false) => {}
            Err(message) => send(output, &PluginEvent::error(message))?,
        },
        HostRequest::Search {
            query_id,
            query,
            limit,
            timeout_ms,
        } => {
            let fallback = SearchContext::new(query_id, timeout_ms);
            let context = context.unwrap_or(&fallback);
            let result = plugin.search_with_context(&query, limit, context);
            let result = context.check().and(result);
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
        } => {
            let (success, message) =
                match plugin.execute_with_input(&result_id, &action_id, input.as_deref()) {
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

fn send(output: &mut impl Write, event: &PluginEvent) -> io::Result<()> {
    serde_json::to_writer(&mut *output, event).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn request_reader() -> Arc<RequestQueue> {
    let queue = Arc::new(RequestQueue::default());
    let reader_queue = queue.clone();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in BufReader::new(stdin.lock()).lines() {
            match line {
                Ok(line) => match serde_json::from_str(&line) {
                    Ok(request) => {
                        if !reader_queue.push(request) {
                            return;
                        }
                    }
                    Err(error) => reader_queue.invalid(format!("invalid request: {error}")),
                },
                Err(error) => {
                    reader_queue.invalid(format!("could not read request: {error}"));
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
}
