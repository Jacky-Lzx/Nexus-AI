use clipkeep_protocol::{HostRequest, PROTOCOL_VERSION, PluginEvent, SearchResult};
use directories::ProjectDirs;
use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
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
    plugins: Vec<PluginSlot>,
    current_query_id: u64,
    next_request_id: u64,
    current_query: String,
    results_by_plugin: HashMap<String, Vec<SearchResult>>,
    results: Vec<SearchResult>,
    status: Option<String>,
    settings_path: Option<PathBuf>,
}

struct PluginSlot {
    id: String,
    name: String,
    executable: PathBuf,
    client: Option<PluginClient>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginState {
    pub id: String,
    pub name: String,
    pub enabled: bool,
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
            settings_path: None,
        }
    }
}

impl SearchCore {
    pub fn with_builtin_plugins() -> io::Result<Self> {
        let mut core = Self::default();
        let (settings_path, settings) = load_plugin_settings();
        core.settings_path = settings_path;
        for (plugin_id, plugin_name, executable_name) in [
            (
                "io.clipkeep.applications",
                "Applications",
                "clipkeep-plugin-applications",
            ),
            (
                "io.clipkeep.clipboard",
                "Clipboard History",
                "clipkeep-plugin-clipboard",
            ),
        ] {
            let executable = sibling_executable(executable_name)?;
            let enabled = settings.get(plugin_id).copied().unwrap_or(true);
            core.register_plugin(plugin_id, plugin_name, executable, enabled)?;
        }
        core.search("");
        Ok(core)
    }

    pub fn register_plugin(
        &mut self,
        plugin_id: impl Into<String>,
        name: impl Into<String>,
        executable: PathBuf,
        enabled: bool,
    ) -> io::Result<()> {
        let plugin_id = plugin_id.into();
        let client = enabled
            .then(|| PluginClient::spawn(plugin_id.clone(), &executable))
            .transpose()?;
        self.plugins.push(PluginSlot {
            id: plugin_id,
            name: name.into(),
            executable,
            client,
        });
        Ok(())
    }

    pub fn plugin_states(&self) -> Vec<PluginState> {
        self.plugins
            .iter()
            .map(|plugin| PluginState {
                id: plugin.id.clone(),
                name: plugin.name.clone(),
                enabled: plugin.client.is_some(),
            })
            .collect()
    }

    pub fn set_plugin_enabled(&mut self, plugin_id: &str, enabled: bool) -> io::Result<()> {
        let plugin = self
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == plugin_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "plugin is not registered"))?;
        match (enabled, plugin.client.is_some()) {
            (true, false) => {
                plugin.client = Some(PluginClient::spawn(&plugin.id, &plugin.executable)?);
            }
            (false, true) => {
                plugin.client.take();
                self.results_by_plugin.remove(plugin_id);
            }
            _ => return Ok(()),
        }
        self.save_plugin_settings()?;
        self.search(self.current_query.clone());
        Ok(())
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
            timeout_ms: None,
        };
        for plugin in &mut self.plugins {
            let Some(client) = &mut plugin.client else {
                continue;
            };
            if let Err(error) = client.send(&request) {
                self.status = Some(format!("{}: {error}", client.plugin_id()));
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
            input: None,
            arguments: None,
        };
        if let Some(client) = self
            .plugins
            .iter_mut()
            .filter_map(|plugin| plugin.client.as_mut())
            .find(|client| client.plugin_id() == result.plugin_id)
            && let Err(error) = client.send(&request)
        {
            self.status = Some(format!("{}: {error}", client.plugin_id()));
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut refresh = false;
        for plugin in &self.plugins {
            let Some(client) = &plugin.client else {
                continue;
            };
            while let Some(event) = client.try_event() {
                match event {
                    PluginEvent::SearchResults {
                        query_id, items, ..
                    } if query_id == self.current_query_id => {
                        self.results_by_plugin
                            .insert(client.plugin_id().to_owned(), items);
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
                    PluginEvent::Error { message, .. } => self.status = Some(message),
                    PluginEvent::Initialized { .. }
                    | PluginEvent::SearchResults { .. }
                    | PluginEvent::ConfigurationItems { .. }
                    | PluginEvent::Notification { .. } => {}
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

    fn save_plugin_settings(&self) -> io::Result<()> {
        let Some(path) = &self.settings_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let settings: HashMap<_, _> = self
            .plugins
            .iter()
            .map(|plugin| (plugin.id.clone(), plugin.client.is_some()))
            .collect();
        let bytes = serde_json::to_vec_pretty(&settings).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, path)
    }
}

fn load_plugin_settings() -> (Option<PathBuf>, HashMap<String, bool>) {
    let path = ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .map(|directories| directories.config_dir().join("plugins.json"));
    let settings = path
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    (path, settings)
}

fn sibling_executable(name: &str) -> io::Result<std::path::PathBuf> {
    let executable_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let current = std::env::current_exe()?;
    let path = current
        .parent()
        .ok_or_else(|| io::Error::other("application executable has no parent directory"))?
        .join(executable_name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("built-in plugin not found: {}", path.display()),
        ))
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
