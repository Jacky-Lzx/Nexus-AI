use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

const PLUGIN_ID: &str = "io.clipkeep.kill-process";
const SNAPSHOT_TTL: Duration = Duration::from_millis(750);

fn main() -> io::Result<()> {
    run_plugin(KillProcessPlugin::default())
}

#[derive(Default)]
struct KillProcessPlugin {
    snapshot: Vec<ProcessEntry>,
    refreshed_at: Option<Instant>,
    last_executables: HashMap<u32, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessEntry {
    pid: u32,
    name: String,
    executable: String,
}

impl Plugin for KillProcessPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        self.refresh_snapshot()?;

        let own_pid = std::process::id();
        let mut matches: Vec<_> = self
            .snapshot
            .iter()
            .filter(|process| process.pid != own_pid)
            .filter_map(|process| {
                match_score(process, &query).map(|score| (score, process.clone()))
            })
            .collect();
        matches.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
                .then_with(|| left.1.pid.cmp(&right.1.pid))
        });
        matches.truncate(limit);

        self.last_executables = matches
            .iter()
            .map(|(_, process)| (process.pid, process.executable.clone()))
            .collect();
        Ok(matches
            .into_iter()
            .map(|(score, process)| process_result(process, score))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id != "terminate" {
            return Err(format!("unknown process action: {action_id}"));
        }
        let pid = result_id
            .strip_prefix("process:")
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| "invalid process result".to_owned())?;
        let expected_executable = self
            .last_executables
            .get(&pid)
            .ok_or_else(|| "process result is no longer current".to_owned())?;
        let current = read_processes()?
            .into_iter()
            .find(|process| process.pid == pid)
            .ok_or_else(|| "process is no longer running".to_owned())?;
        if &current.executable != expected_executable {
            return Err("PID now belongs to a different process; nothing was terminated".into());
        }

        let status = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("could not terminate {} (PID {pid})", current.name));
        }
        self.last_executables.remove(&pid);
        self.refreshed_at = None;
        Ok(Some(format!("Termination requested for {}", current.name)))
    }
}

impl KillProcessPlugin {
    fn refresh_snapshot(&mut self) -> Result<(), String> {
        if self
            .refreshed_at
            .is_some_and(|updated| updated.elapsed() < SNAPSHOT_TTL)
        {
            return Ok(());
        }
        self.snapshot = read_processes()?;
        self.refreshed_at = Some(Instant::now());
        Ok(())
    }
}

fn read_processes() -> Result<Vec<ProcessEntry>, String> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .map_err(|error| format!("could not list processes: {error}"))?;
    if !output.status.success() {
        return Err(format!("process listing failed with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_process_line)
        .collect())
}

fn parse_process_line(line: &str) -> Option<ProcessEntry> {
    let line = line.trim();
    let separator = line.find(char::is_whitespace)?;
    let pid = line[..separator].parse().ok()?;
    let executable = line[separator..].trim().to_owned();
    if executable.is_empty() {
        return None;
    }
    Some(ProcessEntry {
        pid,
        name: display_name(&executable),
        executable,
    })
}

fn display_name(executable: &str) -> String {
    if let Some(app_end) = executable.rfind(".app/Contents/MacOS/") {
        return executable[..app_end]
            .rsplit('/')
            .next()
            .unwrap_or(executable)
            .to_owned();
    }
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(executable)
        .to_owned()
}

fn match_score(process: &ProcessEntry, query: &str) -> Option<u32> {
    let name = process.name.to_lowercase();
    if name == query {
        Some(2_500)
    } else if name.starts_with(query) {
        Some(2_400)
    } else if name.contains(query) {
        Some(2_300)
    } else {
        None
    }
}

fn process_result(process: ProcessEntry, score: u32) -> SearchResult {
    let subtitle = format!("PID {} · {}", process.pid, process.executable);
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: format!("process:{}", process.pid),
        title: process.name.clone(),
        subtitle: subtitle.clone(),
        preview: subtitle,
        kind: ResultKind::Command,
        score: score as f32,
        metadata: BTreeMap::from([
            ("pid".into(), process.pid.to_string()),
            ("executable".into(), process.executable),
            ("symbol_name".into(), "xmark.circle".into()),
            (
                "confirmation_title".into(),
                format!("Terminate {}?", process.name),
            ),
            (
                "confirmation_message".into(),
                format!(
                    "Process {} will receive SIGTERM. Unsaved work may be lost.",
                    process.pid
                ),
            ),
        ]),
        actions: vec![ResultAction {
            host_action: None,
            id: "terminate".into(),
            title: "Terminate Process".into(),
            destructive: true,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_process_rows_and_app_names() {
        let process = parse_process_line(
            "  123 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        )
        .unwrap();
        assert_eq!(process.pid, 123);
        assert_eq!(process.name, "Google Chrome");
    }

    #[test]
    fn does_not_label_nested_plugin_executables_as_the_host_app() {
        let path = "/Applications/Nexus.app/Contents/PlugIns/example.clipkeepplugin/Contents/MacOS/example";
        assert_eq!(display_name(path), "example");
    }

    #[test]
    fn matches_process_names_without_parent_path_false_positives() {
        let process = ProcessEntry {
            pid: 42,
            name: "Safari".into(),
            executable: "/Applications/Safari.app/Contents/MacOS/Safari".into(),
        };
        assert_eq!(match_score(&process, "safari"), Some(2_500));
        assert_eq!(match_score(&process, "applications"), None);
        assert_eq!(match_score(&process, "music"), None);
    }
}
