use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ConfigurationItem, ResultAction, ResultKind, SearchResult};
use directories::UserDirs;
use std::{
    collections::{BTreeMap, HashSet},
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
    hidden_items: HashSet<String>,
    last_scan: Instant,
}

impl ApplicationsPlugin {
    fn new() -> Self {
        Self {
            applications: scan_applications(),
            hidden_items: HashSet::new(),
            last_scan: Instant::now(),
        }
    }
}

impl Plugin for ApplicationsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        let mut matches: Vec<_> = self
            .applications
            .iter()
            .filter(|application| {
                !self
                    .hidden_items
                    .contains(application.path.to_string_lossy().as_ref())
            })
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
        let results: Vec<_> = matches
            .into_iter()
            .take(limit)
            .map(|(score, application)| application.as_result(score))
            .collect();
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

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let hidden_items = settings
            .get("hidden_items")
            .map(|value| serde_json::from_str(value))
            .transpose()
            .map_err(|error| format!("invalid hidden application list: {error}"))?
            .unwrap_or_default();
        let changed = hidden_items != self.hidden_items;
        self.hidden_items = hidden_items;
        Ok(changed)
    }

    fn configuration_items(&self) -> Vec<ConfigurationItem> {
        self.applications
            .iter()
            .map(|application| {
                let path = application.path.to_string_lossy().into_owned();
                ConfigurationItem {
                    id: path.clone(),
                    title: application.name.clone(),
                    subtitle: path.clone(),
                    metadata: BTreeMap::from([("path".into(), path)]),
                }
            })
            .collect()
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
    bundle_name: String,
    pinyin: String,
    pinyin_initials: String,
    pinyin_variants: Vec<String>,
    path: PathBuf,
}

impl Application {
    fn search_score(&self, query: &str) -> Option<f32> {
        let base_score = [
            (&self.name, 0.0),
            (&self.bundle_name, -25.0),
            (&self.pinyin, -50.0),
            (&self.pinyin_initials, -75.0),
        ]
        .into_iter()
        .filter(|(alias, _)| !alias.is_empty())
        .filter_map(|(alias, adjustment)| fuzzy_score(alias, query).map(|score| score + adjustment))
        .max_by(f32::total_cmp);
        self.pinyin_variants
            .iter()
            .filter_map(|alias| fuzzy_score(alias, query).map(|score| score - 50.0))
            .chain(base_score)
            .max_by(f32::total_cmp)
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
                host_action: None,
                id: "open".into(),
                title: "Open".into(),
                destructive: false,
                default: true,
                closes_window: true,
                paste_to_previous: false,
                input_fields: Vec::new(),
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
        // The Rust plugin loop does not drain Cocoa autoreleased objects. Release
        // temporary localization objects after every scan, including the first one.
        objc2::rc::autoreleasepool(|_| {
            let user_home = UserDirs::new().map(|user_dirs| user_dirs.home_dir().to_owned());
            let roots = macos_application_roots(user_home.as_deref());
            let mut applications = Vec::new();
            for root in roots {
                scan_macos_directory(&root, 3, &mut applications);
            }
            applications.sort_by(|left, right| left.name.cmp(&right.name));
            applications.dedup_by(|left, right| left.path == right.path);
            applications
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn macos_application_roots(user_home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        // macOS keeps some user-facing utilities here, including Keychain Access.
        PathBuf::from("/System/Library/CoreServices/Applications"),
    ];
    if let Some(user_home) = user_home {
        roots.push(user_home.join("Applications"));
    }
    roots
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
                let (pinyin, pinyin_initials, pinyin_variants) =
                    pinyin_aliases(&name).unwrap_or_default();
                output.push(Application {
                    name,
                    bundle_name,
                    pinyin,
                    pinyin_initials,
                    pinyin_variants,
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

    let locale = preferred_locale();
    if let Some(name) = localized_name_from_loctable(path, &locale)
        .or_else(|| localized_name_from_strings(path, &locale))
    {
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
fn localized_name_from_loctable(path: &Path, locale: &str) -> Option<String> {
    use objc2_foundation::{NSDictionary, NSString};

    let table_path = path.join("Contents/Resources/InfoPlist.loctable");
    if !table_path.is_file() {
        return None;
    }

    let table_path = NSString::from_str(table_path.to_string_lossy().as_ref());
    let table: objc2::rc::Retained<NSDictionary<NSString, NSDictionary<NSString, NSString>>> =
        unsafe { NSDictionary::dictionaryWithContentsOfFile(&table_path) }?;
    for locale in locale_candidates(locale) {
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
#[allow(deprecated)]
fn localized_name_from_strings(path: &Path, locale: &str) -> Option<String> {
    use objc2_foundation::{NSDictionary, NSString};

    let resources = path.join("Contents/Resources");
    for localization in localization_directory_candidates(locale) {
        let strings_path = resources
            .join(format!("{localization}.lproj"))
            .join("InfoPlist.strings");
        if !strings_path.is_file() {
            continue;
        }

        let strings_path = NSString::from_str(strings_path.to_string_lossy().as_ref());
        let values: objc2::rc::Retained<NSDictionary<NSString, NSString>> =
            unsafe { NSDictionary::dictionaryWithContentsOfFile(&strings_path) }?;
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

fn localization_directory_candidates(locale: &str) -> Vec<String> {
    let normalized = locale.replace('_', "-");
    let mut candidates = vec![normalized.clone()];

    let mut shortened = normalized.as_str();
    while let Some((parent, _)) = shortened.rsplit_once('-') {
        candidates.push(parent.to_owned());
        shortened = parent;
    }

    if normalized == "zh-CN" || normalized.starts_with("zh-Hans") {
        push_unique(&mut candidates, "zh-Hans");
    } else if normalized == "zh-HK" || normalized == "zh-MO" {
        push_unique(&mut candidates, "zh-Hant-HK");
        push_unique(&mut candidates, "zh-Hant");
    } else if normalized == "zh-TW" || normalized.starts_with("zh-Hant") {
        push_unique(&mut candidates, "zh-Hant");
    }

    candidates
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

fn clean_application_name(name: String) -> Option<String> {
    let name = name.trim();
    let name = name.strip_suffix(".app").unwrap_or(name).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(target_os = "macos")]
fn pinyin_aliases(name: &str) -> Option<(String, String, Vec<String>)> {
    use objc2_foundation::{
        NSString, NSStringTransformMandarinToLatin, NSStringTransformStripDiacritics,
    };

    if !name
        .chars()
        .any(|character| ('\u{3400}'..='\u{9fff}').contains(&character))
    {
        return None;
    }

    let source_name = name;
    let name = NSString::from_str(source_name);
    let mandarin_to_latin = unsafe { NSStringTransformMandarinToLatin };
    let strip_diacritics = unsafe { NSStringTransformStripDiacritics };
    let latin = name.stringByApplyingTransform_reverse(mandarin_to_latin, false)?;
    let plain = latin
        .stringByApplyingTransform_reverse(strip_diacritics, false)?
        .to_string()
        .to_lowercase();
    let syllables: Vec<_> = plain
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|syllable| !syllable.is_empty())
        .collect();
    let full = syllables.concat();
    let initials = syllables
        .iter()
        .filter_map(|syllable| syllable.chars().next())
        .collect();
    let mut variants = Vec::new();
    for (phrase, system_reading, contextual_reading) in [
        ("音乐", "yinle", "yinyue"),
        ("音樂", "yinle", "yinyue"),
        ("银行", "yinxing", "yinhang"),
        ("銀行", "yinxing", "yinhang"),
        ("重庆", "zhongqing", "chongqing"),
        ("重慶", "zhongqing", "chongqing"),
    ] {
        if source_name.contains(phrase) && full.contains(system_reading) {
            push_unique(
                &mut variants,
                &full.replacen(system_reading, contextual_reading, 1),
            );
        }
    }
    (!full.is_empty()).then_some((full, initials, variants))
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
    fn configured_hidden_applications_are_excluded_from_search() {
        let path = PathBuf::from("/Applications/Hidden.app");
        let mut plugin = ApplicationsPlugin {
            applications: vec![Application {
                name: "Hidden".into(),
                bundle_name: "Hidden".into(),
                pinyin: String::new(),
                pinyin_initials: String::new(),
                pinyin_variants: Vec::new(),
                path: path.clone(),
            }],
            hidden_items: HashSet::new(),
            last_scan: Instant::now(),
        };
        let settings = BTreeMap::from([(
            "hidden_items".into(),
            serde_json::to_string(&[path.to_string_lossy()]).unwrap(),
        )]);

        assert!(plugin.configure(&settings).unwrap());
        assert!(plugin.search("hidden", 10).unwrap().is_empty());
        assert_eq!(plugin.configuration_items().len(), 1);
    }

    #[test]
    fn localized_app_names_keep_the_bundle_name_as_an_alias() {
        let application = Application {
            name: "微信".into(),
            bundle_name: "WeChat".into(),
            pinyin: "weixin".into(),
            pinyin_initials: "wx".into(),
            pinyin_variants: Vec::new(),
            path: PathBuf::from("/Applications/WeChat.app"),
        };
        assert!(application.search_score("微信").is_some());
        assert!(application.search_score("wechat").is_some());
        assert!(application.search_score("weixin").is_some());
        assert!(application.search_score("wx").is_some());
        assert!(application.search_score("telegram").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pinyin_aliases_include_initials_and_contextual_pronunciations() {
        let (full, initials, variants) = pinyin_aliases("网易云音乐").unwrap();
        assert_eq!(full, "wangyiyunyinle");
        assert_eq!(initials, "wyyyl");
        assert!(variants.iter().any(|alias| alias == "wangyiyunyinyue"));
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

    #[test]
    fn localization_directories_fall_back_from_region_to_script() {
        assert_eq!(
            localization_directory_candidates("zh-Hans-CN"),
            ["zh-Hans-CN", "zh-Hans", "zh"]
        );
        assert_eq!(localization_directory_candidates("en_US"), ["en-US", "en"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn application_roots_include_core_services_utilities() {
        let roots = macos_application_roots(Some(Path::new("/Users/example")));

        assert!(roots.contains(&PathBuf::from("/System/Library/CoreServices/Applications")));
        assert!(roots.contains(&PathBuf::from("/Users/example/Applications")));
    }
}
