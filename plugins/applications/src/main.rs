use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use directories::UserDirs;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PLUGIN_ID: &str = "io.clipkeep.applications";

fn main() -> io::Result<()> {
    run_plugin(ApplicationsPlugin::new())
}

struct ApplicationsPlugin {
    applications: Vec<Application>,
    last_scan: Instant,
}

impl ApplicationsPlugin {
    fn new() -> Self {
        Self {
            applications: scan_applications(),
            last_scan: Instant::now(),
        }
    }
}

impl Plugin for ApplicationsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        let mut matches: Vec<_> = self
            .applications
            .iter()
            .filter_map(|application| {
                fuzzy_score(&application.name, &query).map(|score| (score, application))
            })
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(score, application)| application.as_result(score))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id != "open" {
            return Err(format!("unknown application action: {action_id}"));
        }
        let path = result_id
            .strip_prefix("application:")
            .ok_or_else(|| "invalid application result id".to_owned())?;
        open_application(Path::new(path))?;
        Ok(Some("Application opened".to_owned()))
    }

    fn tick(&mut self) -> Result<bool, String> {
        if self.last_scan.elapsed() < Duration::from_secs(60) {
            return Ok(false);
        }
        self.last_scan = Instant::now();
        let applications = scan_applications();
        let changed = applications != self.applications;
        self.applications = applications;
        Ok(changed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Application {
    name: String,
    path: PathBuf,
}

impl Application {
    fn as_result(&self, score: f32) -> SearchResult {
        let path = self.path.to_string_lossy().into_owned();
        let mut metadata = BTreeMap::new();
        metadata.insert("path".into(), path.clone());
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("application:{path}"),
            title: self.name.clone(),
            subtitle: path,
            preview: format!("Open {}", self.name),
            kind: ResultKind::Application,
            score,
            metadata,
            actions: vec![ResultAction {
                id: "open".into(),
                title: "Open".into(),
                destructive: false,
            }],
        }
    }
}

fn fuzzy_score(name: &str, query: &str) -> Option<f32> {
    if query.is_empty() {
        return Some(2_000.0);
    }
    let name = name.to_lowercase();
    if name == query {
        return Some(3_000.0);
    }
    if name.starts_with(query) {
        return Some(2_800.0 - name.len() as f32);
    }
    if let Some(position) = name.find(query) {
        return Some(2_600.0 - position as f32 * 10.0 - name.len() as f32);
    }

    let mut query_chars = query.chars();
    let mut wanted = query_chars.next()?;
    let mut gaps = 0usize;
    let mut matched = 0usize;
    for character in name.chars() {
        if character == wanted {
            matched += 1;
            if let Some(next) = query_chars.next() {
                wanted = next;
            } else {
                return Some(2_200.0 - gaps as f32 * 5.0 - name.len() as f32);
            }
        } else if matched > 0 {
            gaps += 1;
        }
    }
    None
}

fn scan_applications() -> Vec<Application> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        if let Some(user_dirs) = UserDirs::new() {
            roots.push(user_dirs.home_dir().join("Applications"));
        }
        let mut applications = Vec::new();
        for root in roots {
            scan_macos_directory(&root, 3, &mut applications);
        }
        applications.sort_by(|left, right| left.name.cmp(&right.name));
        applications.dedup_by(|left, right| left.path == right.path);
        applications
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn scan_macos_directory(directory: &Path, depth: usize, output: &mut Vec<Application>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "app") {
            let name = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !name.is_empty() {
                output.push(Application { name, path });
            }
        } else if depth > 0 && path.is_dir() {
            scan_macos_directory(&path, depth - 1, output);
        }
    }
}

fn open_application(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");

    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_search_supports_subsequences() {
        assert!(fuzzy_score("Visual Studio Code", "vsc").is_some());
        assert!(fuzzy_score("Safari", "xyz").is_none());
        assert!(fuzzy_score("Safari", "saf").unwrap() > fuzzy_score("Safari", "sfr").unwrap());
    }
}
