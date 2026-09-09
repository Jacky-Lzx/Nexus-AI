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
                application
                    .search_score(&query)
                    .map(|score| (score, application))
            })
            .collect();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        let mut results: Vec<_> = matches
            .into_iter()
            .take(limit)
            .map(|(score, application)| application.as_result(score))
            .collect();
        if let Some(command) = clipboard_history_command(&query) {
            results.push(command);
            results.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.title.cmp(&right.title))
            });
            results.truncate(limit);
        }
        Ok(results)
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

fn clipboard_history_command(query: &str) -> Option<SearchResult> {
    if query.is_empty() {
        return None;
    }
    let score = fuzzy_score("Clipboard History", query)? + 500.0;
    let mut metadata = BTreeMap::new();
    metadata.insert("route".into(), "clipboard_history".into());
    Some(SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: "command:clipboard-history".into(),
        title: "Clipboard History".into(),
        subtitle: "Search copied text, images, and files".into(),
        preview: "Open Clipboard History".into(),
        kind: ResultKind::Command,
        score,
        metadata,
        actions: vec![ResultAction {
            id: "open".into(),
            title: "Open".into(),
            destructive: false,
        }],
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Application {
    name: String,
    bundle_name: String,
    path: PathBuf,
}

impl Application {
    fn search_score(&self, query: &str) -> Option<f32> {
        let display_score = fuzzy_score(&self.name, query);
        let bundle_score = (self.bundle_name != self.name)
            .then(|| fuzzy_score(&self.bundle_name, query).map(|score| score - 25.0))
            .flatten();
        match (display_score, bundle_score) {
            (Some(display), Some(bundle)) => Some(display.max(bundle)),
            (Some(score), None) | (None, Some(score)) => Some(score),
            (None, None) => None,
        }
    }

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
            let bundle_name = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !bundle_name.is_empty() {
                let name = localized_application_name(&path, &bundle_name)
                    .unwrap_or_else(|| bundle_name.clone());
                output.push(Application {
                    name,
                    bundle_name,
                    path,
                });
            }
        } else if depth > 0 && path.is_dir() {
            scan_macos_directory(&path, depth - 1, output);
        }
    }
}

#[cfg(target_os = "macos")]
fn localized_application_name(path: &Path, bundle_name: &str) -> Option<String> {
    use objc2_foundation::{NSBundle, NSFileManager, NSString};

    if let Some(name) = localized_name_from_loctable(path) {
        return Some(name);
    }

    let bundle_path = NSString::from_str(path.to_string_lossy().as_ref());
    let file_manager_name = clean_application_name(
        NSFileManager::defaultManager()
            .displayNameAtPath(&bundle_path)
            .to_string(),
    );

    // Finder's display-name lookup is what localizes system applications (for
    // example, "System Settings.app" becomes "系统设置" on a Chinese Mac).
    if file_manager_name.as_deref() != Some(bundle_name) {
        return file_manager_name;
    }

    // Third-party bundles commonly provide their localized name directly in
    // InfoPlist.strings, even when Finder keeps showing the bundle directory name.
    if let Some(bundle) = NSBundle::bundleWithPath(&bundle_path) {
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            let key = NSString::from_str(key);
            if let Some(value) = bundle.objectForInfoDictionaryKey(&key)
                && let Ok(value) = value.downcast::<NSString>()
                && let Some(value) = clean_application_name(value.to_string())
            {
                return Some(value);
            }
        }
    }

    file_manager_name
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn localized_name_from_loctable(path: &Path) -> Option<String> {
    use objc2_foundation::{NSDictionary, NSString};

    let table_path = path.join("Contents/Resources/InfoPlist.loctable");
    if !table_path.is_file() {
        return None;
    }

    let table_path = NSString::from_str(table_path.to_string_lossy().as_ref());
    let table: objc2::rc::Retained<NSDictionary<NSString, NSDictionary<NSString, NSString>>> =
        unsafe { NSDictionary::dictionaryWithContentsOfFile(&table_path) }?;
    let locale = preferred_locale();

    for locale in locale_candidates(&locale) {
        let locale = NSString::from_str(&locale);
        let Some(values) = table.objectForKey(&locale) else {
            continue;
        };
        for key in ["CFBundleDisplayName", "CFBundleName"] {
            let key = NSString::from_str(key);
            if let Some(value) = values.objectForKey(&key)
                && let Some(value) = clean_application_name(value.to_string())
            {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn preferred_locale() -> String {
    use objc2_foundation::{NSLocale, NSString, NSUserDefaults};

    let languages_key = NSString::from_str("AppleLanguages");
    NSUserDefaults::standardUserDefaults()
        .stringArrayForKey(&languages_key)
        .and_then(|languages| languages.firstObject())
        .map(|language| language.to_string())
        .unwrap_or_else(|| NSLocale::currentLocale().localeIdentifier().to_string())
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let normalized = locale.replace('-', "_");
    let mut candidates = vec![normalized.clone()];
    if normalized.starts_with("zh_Hans") {
        candidates.push("zh_CN".into());
    } else if normalized.starts_with("zh_Hant_HK") {
        candidates.push("zh_HK".into());
    } else if normalized.starts_with("zh_Hant") {
        candidates.push("zh_TW".into());
    }
    if let Some((language, _)) = normalized.split_once('_') {
        candidates.push(language.into());
    }
    candidates.dedup();
    candidates
}

fn clean_application_name(name: String) -> Option<String> {
    let name = name.trim();
    let name = name.strip_suffix(".app").unwrap_or(name).trim();
    (!name.is_empty()).then(|| name.to_owned())
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

    #[test]
    fn clipboard_history_is_exposed_as_a_searchable_command() {
        let command = clipboard_history_command("clipboard history").unwrap();
        assert_eq!(command.kind, ResultKind::Command);
        assert_eq!(command.metadata["route"], "clipboard_history");
        assert!(clipboard_history_command("clip hist").is_some());
        assert!(clipboard_history_command("").is_none());
    }

    #[test]
    fn localized_app_names_keep_the_bundle_name_as_an_alias() {
        let application = Application {
            name: "微信".into(),
            bundle_name: "WeChat".into(),
            path: PathBuf::from("/Applications/WeChat.app"),
        };
        assert!(application.search_score("微信").is_some());
        assert!(application.search_score("wechat").is_some());
        assert!(application.search_score("telegram").is_none());
    }

    #[test]
    fn application_extension_is_removed_from_display_names() {
        assert_eq!(
            clean_application_name("  系统设置.app  ".into()).as_deref(),
            Some("系统设置")
        );
        assert_eq!(clean_application_name("  ".into()), None);
    }

    #[test]
    fn locale_candidates_cover_modern_chinese_identifiers() {
        assert_eq!(
            locale_candidates("zh-Hans-CN"),
            ["zh_Hans_CN", "zh_CN", "zh"]
        );
        assert_eq!(
            locale_candidates("zh-Hant-HK"),
            ["zh_Hant_HK", "zh_HK", "zh"]
        );
    }
}
