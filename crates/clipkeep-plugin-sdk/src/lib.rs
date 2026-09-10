use clipkeep_protocol::{
    ConfigurationItem, HostRequest, PROTOCOL_VERSION, PluginEvent, PluginNotification, SearchResult,
};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

pub trait Plugin {
    fn id(&self) -> &'static str;

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String>;

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

    fn notifications(&mut self) -> Result<Vec<PluginNotification>, String> {
        Ok(Vec::new())
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

    while running {
        while let Ok(message) = requests.try_recv() {
            match message {
                ReaderMessage::Request(request) => {
                    running = handle_request(&mut plugin, request, &mut output)?;
                    if !running {
                        break;
                    }
                }
                ReaderMessage::Invalid(message) => {
                    send(&mut output, &PluginEvent::Error { message })?;
                }
                ReaderMessage::Closed => return Ok(()),
            }
        }

        if running {
            match plugin.tick() {
                Ok(true) => send(&mut output, &PluginEvent::CatalogChanged)?,
                Ok(false) => {}
                Err(message) => send(&mut output, &PluginEvent::Error { message })?,
            }
            match plugin.notifications() {
                Ok(notifications) => {
                    for notification in notifications {
                        send(&mut output, &PluginEvent::Notification { notification })?;
                    }
                }
                Err(message) => send(&mut output, &PluginEvent::Error { message })?,
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

fn handle_request(
    plugin: &mut impl Plugin,
    request: HostRequest,
    output: &mut impl Write,
) -> io::Result<bool> {
    match request {
        HostRequest::Initialize { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                send(
                    output,
                    &PluginEvent::Error {
                        message: format!(
                            "unsupported protocol version {protocol_version}; expected {PROTOCOL_VERSION}"
                        ),
                    },
                )?;
            } else {
                send(
                    output,
                    &PluginEvent::Initialized {
                        plugin_id: plugin.id().to_owned(),
                        protocol_version: PROTOCOL_VERSION,
                    },
                )?;
            }
        }
        HostRequest::Configure { settings } => match plugin.configure(&settings) {
            Ok(true) => send(output, &PluginEvent::CatalogChanged)?,
            Ok(false) => {}
            Err(message) => send(output, &PluginEvent::Error { message })?,
        },
        HostRequest::Search {
            query_id,
            query,
            limit,
        } => match plugin.search(&query, limit) {
            Ok(items) => send(output, &PluginEvent::SearchResults { query_id, items })?,
            Err(message) => send(output, &PluginEvent::Error { message })?,
        },
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
        HostRequest::Shutdown => return Ok(false),
    }
    Ok(true)
}

fn send(output: &mut impl Write, event: &PluginEvent) -> io::Result<()> {
    serde_json::to_writer(&mut *output, event).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

enum ReaderMessage {
    Request(HostRequest),
    Invalid(String),
    Closed,
}

fn request_reader() -> Receiver<ReaderMessage> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in BufReader::new(stdin.lock()).lines() {
            let message = match line {
                Ok(line) => match serde_json::from_str(&line) {
                    Ok(request) => ReaderMessage::Request(request),
                    Err(error) => ReaderMessage::Invalid(format!("invalid request: {error}")),
                },
                Err(error) => ReaderMessage::Invalid(format!("could not read request: {error}")),
            };
            if sender.send(message).is_err() {
                return;
            }
        }
        let _ = sender.send(ReaderMessage::Closed);
    });
    receiver
}
