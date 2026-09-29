use clipkeep_plugin_sdk::{Plugin, SearchContext, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use directories::UserDirs;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsStr,
    io,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const PLUGIN_ID: &str = "io.clipkeep.files";
const MIN_QUERY_CHARACTERS: usize = 2;

fn main() -> io::Result<()> {
    run_plugin(FilesPlugin::new())
}

struct FilesPlugin {
    home: PathBuf,
    recent_results: HashSet<PathBuf>,
}

impl FilesPlugin {
    fn new() -> Self {
        let home = UserDirs::new()
            .map(|directories| directories.home_dir().to_owned())
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            home,
            recent_results: HashSet::new(),
        }
    }
}

impl Plugin for FilesPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        self.search_with_context(query, limit, &SearchContext::new(0, None))
    }

    fn search_with_context(
        &mut self,
        query: &str,
        limit: usize,
        context: &SearchContext,
    ) -> Result<Vec<SearchResult>, String> {
        context.check()?;
        let query = query.trim();
        if limit == 0 || query.chars().count() < MIN_QUERY_CHARACTERS {
            self.recent_results.clear();
            return Ok(Vec::new());
        }

        let paths = spotlight_search(&self.home, query, limit, context)?;
        self.recent_results = paths.iter().cloned().collect();
        Ok(paths
            .into_iter()
            .map(|path| file_result(&self.home, &path, query))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        let path = result_id
            .strip_prefix("file:")
            .map(PathBuf::from)
            .ok_or_else(|| "invalid file result id".to_owned())?;
        if !self.recent_results.contains(&path) {
            return Err("file result is no longer active".into());
        }
        if !path.exists() {
            return Err("file no longer exists".into());
        }

        let mut command = Command::new("/usr/bin/open");
        let message = match action_id {
            "open" => {
                command.arg(&path);
                "Opened"
            }
            "reveal" => {
                command.arg("-R").arg(&path);
                "Revealed in Finder"
            }
            _ => return Err(format!("unknown file action: {action_id}")),
        };
        let status = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not launch Finder: {error}"))?;
        if !status.success() {
            return Err(format!("Finder returned {status}"));
        }
        Ok(Some(message.into()))
    }
}

fn spotlight_search(
    home: &Path,
    query: &str,
    limit: usize,
    context: &SearchContext,
) -> Result<Vec<PathBuf>, String> {
    let output = clipkeep_plugin_sdk::command_output(
        Command::new("/usr/bin/mdfind")
            .args([OsStr::new("-0"), OsStr::new("-onlyin")])
            .arg(home)
            .args([OsStr::new("-name"), OsStr::new(query)]),
        context,
        8 * 1024 * 1024,
    )
    .map_err(|error| format!("Spotlight search failed: {error}"))?;
    if !output.status.success() {
        return Err(format!("Spotlight search returned {}", output.status));
    }
    let mut seen = HashSet::new();
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|bytes| std::str::from_utf8(bytes).ok())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| {
            is_visible_user_item(home, path) && path.exists() && seen.insert(path.clone())
        })
        .take(limit)
        .collect())
}

fn is_visible_user_item(home: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(home) else {
        return false;
    };
    let mut components = relative.components();
    if components.next() == Some(Component::Normal(OsStr::new("Library"))) {
        return false;
    }
    relative.components().all(|component| match component {
        Component::Normal(value) => !value.to_string_lossy().starts_with('.'),
        _ => true,
    }) && !path
        .ancestors()
        .any(|ancestor| ancestor.extension() == Some(OsStr::new("app")))
}

fn file_result(home: &Path, path: &Path, query: &str) -> SearchResult {
    let title = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let display_path = path
        .strip_prefix(home)
        .map(|relative| format!("~/{}", relative.display()))
        .unwrap_or_else(|_| path.display().to_string());
    let path_string = path.to_string_lossy().into_owned();
    let mut metadata = BTreeMap::new();
    metadata.insert("path".into(), path_string.clone());

    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: format!("file:{path_string}"),
        title: title.clone(),
        subtitle: display_path.clone(),
        preview: display_path,
        kind: ResultKind::File,
        score: score_for_name(&title, query),
        metadata,
        actions: vec![
            ResultAction {
                host_action: None,
                id: "open".into(),
                title: "Open".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
            ResultAction {
                host_action: None,
                id: "reveal".into(),
                title: "Show in Finder".into(),
                destructive: false,
                default: false,
                closes_window: true,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ],
    }
}

fn score_for_name(name: &str, query: &str) -> f32 {
    let name = name.to_lowercase();
    let query = query.to_lowercase();
    if name == query {
        2_500.0
    } else if name.starts_with(&query) {
        2_300.0 - name.len() as f32
    } else if let Some(position) = name.find(&query) {
        2_100.0 - position as f32 * 5.0 - name.len() as f32
    } else {
        1_500.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_queries_do_not_start_a_broad_search() {
        let mut plugin = FilesPlugin {
            home: PathBuf::from("/Users/test"),
            recent_results: HashSet::new(),
        };
        assert!(plugin.search("a", 20).unwrap().is_empty());
    }

    #[test]
    fn library_hidden_and_application_contents_are_filtered() {
        let home = Path::new("/Users/test");
        assert!(!is_visible_user_item(
            home,
            Path::new("/Users/test/Library/Preferences/example.plist")
        ));
        assert!(!is_visible_user_item(
            home,
            Path::new("/Users/test/.config/example.toml")
        ));
        assert!(!is_visible_user_item(
            home,
            Path::new("/Users/test/Applications/Demo.app/Contents/Info.plist")
        ));
        assert!(is_visible_user_item(
            home,
            Path::new("/Users/test/Documents/report.pdf")
        ));
    }

    #[test]
    fn exact_and_prefix_names_rank_ahead_of_substrings() {
        assert!(score_for_name("report", "report") > score_for_name("report-final", "report"));
        assert!(
            score_for_name("report-final", "report")
                > score_for_name("annual-report-final", "report")
        );
    }

    #[test]
    fn results_include_open_and_reveal_actions() {
        let result = file_result(
            Path::new("/Users/test"),
            Path::new("/Users/test/Documents/report.pdf"),
            "report",
        );
        assert_eq!(result.kind, ResultKind::File);
        assert_eq!(result.title, "report.pdf");
        assert_eq!(result.subtitle, "~/Documents/report.pdf");
        assert_eq!(result.metadata["path"], "/Users/test/Documents/report.pdf");
        assert_eq!(result.actions.len(), 2);
        assert!(result.actions[0].default);
    }
}
