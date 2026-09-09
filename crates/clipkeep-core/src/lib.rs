use clipkeep_protocol::{HostRequest, PROTOCOL_VERSION, PluginEvent, SearchResult};
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
};

pub struct PluginClient {
    plugin_id: String,
    child: Child,
    input: BufWriter<ChildStdin>,
    events: Receiver<PluginEvent>,
}

impl PluginClient {
    pub fn spawn(plugin_id: impl Into<String>, executable: &Path) -> io::Result<Self> {
        let plugin_id = plugin_id.into();
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("plugin stdin was not available"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("plugin stdout was not available"))?;
        let events = event_reader(output);

        let mut client = Self {
            plugin_id,
            child,
            input: BufWriter::new(input),
            events,
        };
        client.send(&HostRequest::Initialize {
            protocol_version: PROTOCOL_VERSION,
        })?;
        Ok(client)
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn send(&mut self, request: &HostRequest) -> io::Result<()> {
        serde_json::to_writer(&mut self.input, request).map_err(io::Error::other)?;
        self.input.write_all(b"\n")?;
        self.input.flush()
    }

    pub fn try_event(&self) -> Option<PluginEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for PluginClient {
    fn drop(&mut self) {
        let _ = self.send(&HostRequest::Shutdown);
        let _ = self.child.wait();
    }
}

pub struct SearchCore {
    plugins: Vec<PluginClient>,
    current_query_id: u64,
    next_request_id: u64,
    current_query: String,
    results_by_plugin: HashMap<String, Vec<SearchResult>>,
    results: Vec<SearchResult>,
    status: Option<String>,
}

impl Default for SearchCore {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            current_query_id: 0,
            next_request_id: 1,
            current_query: String::new(),
            results_by_plugin: HashMap::new(),
            results: Vec::new(),
            status: None,
        }
    }
}

impl SearchCore {
    pub fn add_plugin(&mut self, client: PluginClient) {
        self.plugins.push(client);
    }

    pub fn search(&mut self, query: impl Into<String>) {
        self.current_query = query.into();
        self.current_query_id += 1;
        self.results_by_plugin.clear();
        self.results.clear();
        let request = HostRequest::Search {
            query_id: self.current_query_id,
            query: self.current_query.clone(),
            limit: 100,
        };
        for plugin in &mut self.plugins {
            if let Err(error) = plugin.send(&request) {
                self.status = Some(format!("{}: {error}", plugin.plugin_id()));
            }
        }
    }

    pub fn execute(&mut self, result: &SearchResult, action_id: impl Into<String>) {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let request = HostRequest::Execute {
            request_id,
            result_id: result.id.clone(),
            action_id: action_id.into(),
        };
        if let Some(plugin) = self
            .plugins
            .iter_mut()
            .find(|plugin| plugin.plugin_id() == result.plugin_id)
            && let Err(error) = plugin.send(&request)
        {
            self.status = Some(format!("{}: {error}", plugin.plugin_id()));
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut refresh = false;
        for plugin in &self.plugins {
            while let Some(event) = plugin.try_event() {
                match event {
                    PluginEvent::SearchResults { query_id, items }
                        if query_id == self.current_query_id =>
                    {
                        self.results_by_plugin
                            .insert(plugin.plugin_id().to_owned(), items);
                        changed = true;
                    }
                    PluginEvent::CatalogChanged => refresh = true,
                    PluginEvent::ActionFinished {
                        success, message, ..
                    } => {
                        self.status = message.or_else(|| {
                            Some(if success { "Done" } else { "Action failed" }.to_owned())
                        });
                        refresh = true;
                    }
                    PluginEvent::Error { message } => self.status = Some(message),
                    PluginEvent::Initialized { .. } | PluginEvent::SearchResults { .. } => {}
                }
            }
        }
        if refresh {
            self.search(self.current_query.clone());
            return true;
        }
        if changed {
            self.rebuild_results();
        }
        changed
    }

    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    pub fn take_status(&mut self) -> Option<String> {
        self.status.take()
    }

    fn rebuild_results(&mut self) {
        self.results = self.results_by_plugin.values().flatten().cloned().collect();
        self.results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.title.cmp(&right.title))
        });
    }
}

fn event_reader(output: impl io::Read + Send + 'static) -> Receiver<PluginEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if let Ok(event) = serde_json::from_str(&line)
                && sender.send(event).is_err()
            {
                return;
            }
        }
    });
    receiver
}
