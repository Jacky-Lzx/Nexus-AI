use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use directories::ProjectDirs;
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

const PLUGIN_ID: &str = "io.clipkeep.clipboard";
const MAX_HISTORY: usize = 500;
const MAX_ITEM_BYTES: usize = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 250 * 1024 * 1024;
const HISTORY_MAGIC: &[u8; 4] = b"CKH1";

#[derive(Clone, Debug)]
struct ClipboardSettings {
    retention_days: u64,
    ignore_concealed: bool,
    ignored_bundle_ids: Vec<String>,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            retention_days: 30,
            ignore_concealed: true,
            ignored_bundle_ids: Vec::new(),
        }
    }
}

fn main() -> io::Result<()> {
    run_plugin(ClipboardPlugin::new()?)
}

struct ClipboardPlugin {
    clipboard: PlatformClipboard,
    items: Vec<ClipItem>,
    next_id: u64,
    history_path: PathBuf,
    preview_cache_dir: PathBuf,
    encryption_key: [u8; 32],
    settings: ClipboardSettings,
}

impl ClipboardPlugin {
    fn new() -> io::Result<Self> {
        let history_path = history_path()
            .ok_or_else(|| io::Error::other("could not determine clipboard data directory"))?;
        let preview_cache_dir = preview_cache_dir()
            .ok_or_else(|| io::Error::other("could not determine clipboard cache directory"))?;
        let encryption_key = history_encryption_key()?;
        let legacy_path = legacy_history_path();
        let mut items: Vec<ClipItem> =
            load_history(&history_path, legacy_path.as_deref(), &encryption_key)
                .unwrap_or_default();
        let mut added_thumbnails = false;
        for item in &mut items {
            if item.kind == ClipKind::Image && item.thumbnail.is_none() {
                item.thumbnail = make_thumbnail(&item.representations);
                added_thumbnails |= item.thumbnail.is_some();
            }
        }
        let next_id = items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
        let plugin = Self {
            clipboard: PlatformClipboard::new()?,
            items,
            next_id,
            history_path,
            preview_cache_dir,
            encryption_key,
            settings: ClipboardSettings::default(),
        };
        if added_thumbnails || legacy_path.as_ref().is_some_and(|path| path.is_file()) {
            plugin.save().map_err(io::Error::other)?;
            if let Some(path) = legacy_path {
                let _ = fs::remove_file(path);
            }
        }
        Ok(plugin)
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.history_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temp = self.history_path.with_extension("json.tmp");
        let plaintext = serde_json::to_vec(&self.items).map_err(|error| error.to_string())?;
        let bytes = encrypt_history(&plaintext, &self.encryption_key)?;
        fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        fs::rename(temp, &self.history_path).map_err(|error| error.to_string())
    }

    fn item_id(raw: &str) -> Result<u64, String> {
        raw.strip_prefix("clipboard:")
            .ok_or_else(|| "invalid clipboard result id".to_owned())?
            .parse()
            .map_err(|_| "invalid clipboard result id".to_owned())
    }

    fn apply_edited_text(
        &mut self,
        result_id: &str,
        edited: &str,
    ) -> Result<Option<String>, String> {
        if edited.len() > MAX_ITEM_BYTES {
            return Err("edited clipboard text is too large".into());
        }
        let id = Self::item_id(result_id)?;
        let position = self
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
        let item = self.items[position].clone();
        if !matches!(item.kind, ClipKind::Text | ClipKind::RichText) {
            return Err("only text clipboard entries can be edited".into());
        }
        let content = plain_text_content(edited.to_owned());
        self.items[position] = ClipItem::new(id, content);
        self.items[position].favorite = item.favorite;
        let edited_item = self.items[position].clone();
        self.clipboard.restore(&edited_item)?;
        promote_item(&mut self.items, position, now_secs());
        self.save()?;
        Ok(Some("Edited text ready to paste".into()))
    }
}

impl Plugin for ClipboardPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.to_lowercase();
        Ok(self
            .items
            .iter()
            .filter(|item| query.is_empty() || item.search_text().to_lowercase().contains(&query))
            .take(limit)
            .enumerate()
            .map(|(index, item)| item.as_result(index, &query, &self.preview_cache_dir))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id.starts_with("clear_") {
            let cutoff = match action_id {
                "clear_5m" => Some(now_secs().saturating_sub(5 * 60)),
                "clear_15m" => Some(now_secs().saturating_sub(15 * 60)),
                "clear_all" => None,
                _ => return Err(format!("unknown clipboard action: {action_id}")),
            };
            let cache = &self.preview_cache_dir;
            self.items.retain(|item| {
                let remove =
                    !item.favorite && cutoff.is_none_or(|cutoff| item.created_at >= cutoff);
                if remove {
                    item.remove_image_preview_files(cache);
                }
                !remove
            });
            self.save()?;
            return Ok(Some("Clipboard history cleared".into()));
        }
        let id = Self::item_id(result_id)?;
        match action_id {
            "copy" => {
                let position = self
                    .items
                    .iter()
                    .position(|item| item.id == id)
                    .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
                let item = self.items[position].clone();
                self.clipboard.restore(&item)?;
                promote_item(&mut self.items, position, now_secs());
                self.save()?;
                Ok(Some(format!("{} ready to paste", item.kind.label())))
            }
            "favorite" => {
                let favorite = {
                    let item = self
                        .items
                        .iter_mut()
                        .find(|item| item.id == id)
                        .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
                    item.favorite = !item.favorite;
                    item.favorite
                };
                self.save()?;
                Ok(Some(
                    if favorite { "Favorited" } else { "Unfavorited" }.to_owned(),
                ))
            }
            "delete" => {
                let position = self
                    .items
                    .iter()
                    .position(|item| item.id == id)
                    .ok_or_else(|| "clipboard entry no longer exists".to_owned())?;
                let item = self.items.remove(position);
                item.remove_image_preview_files(&self.preview_cache_dir);
                self.save()?;
                Ok(Some("Deleted".to_owned()))
            }
            _ => Err(format!("unknown clipboard action: {action_id}")),
        }
    }

    fn execute_with_input(
        &mut self,
        result_id: &str,
        action_id: &str,
        input: Option<&str>,
    ) -> Result<Option<String>, String> {
        if action_id == "apply_edit" {
            let edited = input.ok_or_else(|| "edited text is missing".to_owned())?;
            self.apply_edited_text(result_id, edited)
        } else {
            self.execute(result_id, action_id)
        }
    }

    fn tick(&mut self) -> Result<bool, String> {
        let Some(content) = self.clipboard.capture(&self.settings)? else {
            return Ok(false);
        };
        if content.byte_len() > MAX_ITEM_BYTES {
            return Ok(false);
        }
        if let Some(position) = self
            .items
            .iter()
            .position(|item| item.fingerprint == content.fingerprint)
        {
            promote_item(&mut self.items, position, now_secs());
        } else {
            self.items.insert(0, ClipItem::new(self.next_id, content));
            self.next_id += 1;
        }
        trim_history(&mut self.items, &self.preview_cache_dir);
        self.save()?;
        Ok(true)
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        self.settings.retention_days = settings
            .get("retention_days")
            .and_then(|value| value.parse().ok())
            .unwrap_or(30);
        self.settings.ignore_concealed = settings
            .get("ignore_concealed")
            .is_none_or(|value| value == "true");
        self.settings.ignored_bundle_ids = settings
            .get("ignored_bundle_ids")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let cutoff = now_secs().saturating_sub(self.settings.retention_days * 86_400);
        let cache = &self.preview_cache_dir;
        let before = self.items.len();
        self.items.retain(|item| {
            let keep = item.favorite || item.created_at >= cutoff;
            if !keep {
                item.remove_image_preview_files(cache);
            }
            keep
        });
        if self.items.len() != before {
            self.save()?;
        }
        Ok(self.items.len() != before)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ClipKind {
    #[default]
    Text,
    RichText,
    Image,
    Files,
}

impl ClipKind {
    fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::RichText => "Rich text",
            Self::Image => "Image",
            Self::Files => "Files",
        }
    }

    fn result_kind(self) -> ResultKind {
        match self {
            Self::Text => ResultKind::ClipboardText,
            Self::RichText => ResultKind::ClipboardRichText,
            Self::Image => ResultKind::ClipboardImage,
            Self::Files => ResultKind::ClipboardFiles,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Representation {
    #[serde(default)]
    item: usize,
    uti: String,
    #[serde(with = "base64_bytes")]
    data: Vec<u8>,
}

#[derive(Clone, Debug)]
struct CapturedContent {
    kind: ClipKind,
    text: String,
    representations: Vec<Representation>,
    fingerprint: u64,
    thumbnail: Option<String>,
}

impl CapturedContent {
    fn byte_len(&self) -> usize {
        let representation_bytes = self
            .representations
            .iter()
            .map(|value| value.data.len())
            .sum();
        if representation_bytes == 0 && matches!(self.kind, ClipKind::Text | ClipKind::RichText) {
            self.text.len()
        } else {
            representation_bytes
        }
    }
}

fn plain_text_content(text: String) -> CapturedContent {
    let uti = "public.utf8-plain-text".to_owned();
    let data = text.as_bytes().to_vec();
    let mut hasher = DefaultHasher::new();
    0usize.hash(&mut hasher);
    uti.hash(&mut hasher);
    data.hash(&mut hasher);
    CapturedContent {
        kind: ClipKind::Text,
        text,
        // Let PlatformClipboard::restore use NSPasteboard.setString so AppKit owns
        // the exact text encoding, including whether a trailing newline exists.
        representations: Vec::new(),
        fingerprint: hasher.finish(),
        thumbnail: None,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ClipItem {
    id: u64,
    #[serde(default)]
    text: String,
    created_at: u64,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    kind: ClipKind,
    #[serde(default)]
    representations: Vec<Representation>,
    #[serde(default)]
    fingerprint: u64,
    #[serde(default)]
    thumbnail: Option<String>,
}

impl ClipItem {
    fn new(id: u64, content: CapturedContent) -> Self {
        Self {
            id,
            text: content.text,
            created_at: now_secs(),
            favorite: false,
            kind: content.kind,
            representations: content.representations,
            fingerprint: content.fingerprint,
            thumbnail: content.thumbnail,
        }
    }

    fn byte_len(&self) -> usize {
        let representation_bytes = self
            .representations
            .iter()
            .map(|value| value.data.len())
            .sum();
        if representation_bytes == 0 && matches!(self.kind, ClipKind::Text | ClipKind::RichText) {
            self.text.len()
        } else {
            representation_bytes
        }
    }

    fn search_text(&self) -> String {
        if self.text.is_empty() {
            self.kind.label().to_owned()
        } else {
            self.text.clone()
        }
    }

    fn subtitle(&self) -> String {
        let age = age_label(self.created_at);
        match self.kind {
            ClipKind::Text => age,
            ClipKind::Image => format!("{} · {age}", byte_size(self.byte_len())),
            ClipKind::RichText | ClipKind::Files => format!("{} · {age}", self.kind.label()),
        }
    }

    fn as_result(&self, index: usize, query: &str, preview_cache_dir: &Path) -> SearchResult {
        let mut metadata = BTreeMap::from([
            ("host_action.edit".into(), "text.edit".into()),
            ("edit_submit_action".into(), "apply_edit".into()),
        ]);
        metadata.insert("format".into(), self.kind.label().into());
        metadata.insert("size".into(), byte_size(self.byte_len()));
        metadata.insert("copied".into(), age_label(self.created_at));
        metadata.insert("favorite".into(), self.favorite.to_string());
        if self.kind == ClipKind::Files {
            let count = self
                .representations
                .iter()
                .map(|value| value.item)
                .max()
                .map_or(0, |index| index + 1);
            metadata.insert("items".into(), count.to_string());
        }
        if self.kind == ClipKind::Image
            && let Some(thumbnail) = self.thumbnail_path(preview_cache_dir)
        {
            metadata.insert(
                "thumbnail_image_path".into(),
                thumbnail.to_string_lossy().into_owned(),
            );
        }
        if self.kind == ClipKind::Image
            && let Some((preview, original)) = self.image_paths(preview_cache_dir)
        {
            metadata.insert(
                "preview_image_path".into(),
                preview.to_string_lossy().into_owned(),
            );
            metadata.insert(
                "original_image_path".into(),
                original.to_string_lossy().into_owned(),
            );
        }
        if matches!(self.kind, ClipKind::Text | ClipKind::RichText) {
            metadata.insert("characters".into(), self.text.chars().count().to_string());
            metadata.insert(
                "words".into(),
                self.text.split_whitespace().count().to_string(),
            );
        }
        let title = match self.kind {
            ClipKind::Text | ClipKind::RichText if !self.text.is_empty() => {
                first_line_preview(&self.text, 60)
            }
            ClipKind::Image => "Image".to_owned(),
            ClipKind::Files if !self.text.is_empty() => first_line_preview(&self.text, 60),
            ClipKind::Files => "Files".to_owned(),
            _ => self.kind.label().to_owned(),
        };
        let match_bonus =
            if !query.is_empty() && self.search_text().to_lowercase().starts_with(query) {
                50.0
            } else {
                0.0
            };
        let mut actions = vec![ResultAction {
            id: "copy".into(),
            title: "Paste".into(),
            destructive: false,
            default: true,
            closes_window: true,
            paste_to_previous: true,
            input_fields: Vec::new(),
        }];
        if matches!(self.kind, ClipKind::Text | ClipKind::RichText) && !self.text.is_empty() {
            actions.push(ResultAction {
                id: "edit".into(),
                title: "Edit in Neovim".into(),
                destructive: false,
                default: false,
                closes_window: true,
                paste_to_previous: true,
                input_fields: Vec::new(),
            });
        }
        actions.extend([
            ResultAction {
                id: "favorite".into(),
                title: "Favorite".into(),
                destructive: false,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
            ResultAction {
                id: "delete".into(),
                title: "Delete".into(),
                destructive: true,
                default: false,
                closes_window: false,
                paste_to_previous: false,
                input_fields: Vec::new(),
            },
        ]);
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("clipboard:{}", self.id),
            title,
            subtitle: self.subtitle(),
            preview: if self.text.is_empty() {
                format!("{} clipboard content", self.kind.label())
            } else {
                self.text.clone()
            },
            kind: self.kind.result_kind(),
            score: 1_000.0 - index as f32 + match_bonus + if self.favorite { 10.0 } else { 0.0 },
            metadata,
            actions,
        }
    }

    fn thumbnail_path(&self, cache_dir: &Path) -> Option<PathBuf> {
        fs::create_dir_all(cache_dir).ok()?;
        let path = cache_dir.join(format!("{}-thumbnail.png", self.preview_cache_stem()));
        if path.is_file() {
            return Some(path);
        }
        let bytes = if let Some(encoded) = self.thumbnail.as_deref() {
            BASE64.decode(encoded).ok()?
        } else {
            BASE64.decode(make_thumbnail(&self.representations)?).ok()?
        };
        fs::write(&path, bytes).ok()?;
        Some(path)
    }

    fn image_paths(&self, cache_dir: &Path) -> Option<(PathBuf, PathBuf)> {
        let source = self
            .representations
            .iter()
            .find(|value| value.uti == "public.png")
            .or_else(|| {
                self.representations
                    .iter()
                    .find(|value| value.uti == "public.tiff")
            })?;
        fs::create_dir_all(cache_dir).ok()?;
        let stem = self.preview_cache_stem();
        let extension = if source.uti == "public.png" {
            "png"
        } else {
            "tiff"
        };
        let original = cache_dir.join(format!("{stem}-original.{extension}"));
        if !original.is_file() {
            fs::write(&original, &source.data).ok()?;
        }

        let preview = cache_dir.join(format!("{stem}-preview.png"));
        Some((preview, original))
    }

    fn preview_cache_stem(&self) -> String {
        format!("{}-{:016x}", self.id, self.fingerprint)
    }

    fn remove_image_preview_files(&self, cache_dir: &Path) {
        let stem = self.preview_cache_stem();
        for suffix in [
            "thumbnail.png",
            "preview.png",
            "original.png",
            "original.tiff",
        ] {
            let _ = fs::remove_file(cache_dir.join(format!("{stem}-{suffix}")));
        }
    }
}

#[cfg(target_os = "macos")]
struct PlatformClipboard {
    pasteboard: objc2::rc::Retained<objc2_app_kit::NSPasteboard>,
    last_change_count: isize,
}

#[cfg(target_os = "macos")]
impl PlatformClipboard {
    fn new() -> io::Result<Self> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        Ok(Self {
            pasteboard,
            last_change_count: -1,
        })
    }

    fn capture(&mut self, settings: &ClipboardSettings) -> Result<Option<CapturedContent>, String> {
        let change_count = self.pasteboard.changeCount();
        if change_count == self.last_change_count {
            return Ok(None);
        }
        self.last_change_count = change_count;
        let Some(items) = self.pasteboard.pasteboardItems() else {
            return Ok(None);
        };

        if settings.ignore_concealed && pasteboard_is_sensitive(&items) {
            return Ok(None);
        }
        if current_application_is_ignored(&settings.ignored_bundle_ids) {
            return Ok(None);
        }

        let mut representations = Vec::new();
        let mut hasher = DefaultHasher::new();
        for item_index in 0..items.count() {
            let item = items.objectAtIndex(item_index);
            let types = item.types();
            for type_index in 0..types.count() {
                let pasteboard_type = types.objectAtIndex(type_index);
                let uti = pasteboard_type.to_string();
                if !supported_type(&uti) {
                    continue;
                }
                let Some(data) = item.dataForType(&pasteboard_type) else {
                    continue;
                };
                let bytes = data.to_vec();
                item_index.hash(&mut hasher);
                uti.hash(&mut hasher);
                bytes.hash(&mut hasher);
                representations.push(Representation {
                    item: item_index,
                    uti,
                    data: bytes,
                });
            }
        }
        if representations.is_empty() {
            return Ok(None);
        }
        let text_type = unsafe { objc2_app_kit::NSPasteboardTypeString };
        let plain_text = self
            .pasteboard
            .stringForType(text_type)
            .map(|value| value.to_string())
            .unwrap_or_default();
        let kind = classify(&representations);
        let thumbnail = (kind == ClipKind::Image)
            .then(|| make_thumbnail(&representations))
            .flatten();
        Ok(Some(CapturedContent {
            kind,
            text: plain_text,
            representations,
            fingerprint: hasher.finish(),
            thumbnail,
        }))
    }

    fn restore(&mut self, item: &ClipItem) -> Result<(), String> {
        use objc2::runtime::ProtocolObject;
        use objc2_app_kit::{NSPasteboardItem, NSPasteboardWriting};
        use objc2_foundation::{NSArray, NSData, NSString};

        if item.representations.is_empty() {
            self.pasteboard.clearContents();
            let text = NSString::from_str(&item.text);
            let text_type = unsafe { objc2_app_kit::NSPasteboardTypeString };
            if !self.pasteboard.setString_forType(&text, text_type) {
                return Err("could not restore text clipboard content".into());
            }
            self.last_change_count = self.pasteboard.changeCount();
            return Ok(());
        }

        let mut grouped = BTreeMap::<usize, Vec<&Representation>>::new();
        for representation in &item.representations {
            grouped
                .entry(representation.item)
                .or_default()
                .push(representation);
        }
        let mut pasteboard_items = Vec::new();
        for representations in grouped.values() {
            let pasteboard_item = NSPasteboardItem::new();
            for representation in representations {
                let pasteboard_type = NSString::from_str(&representation.uti);
                let data = NSData::with_bytes(&representation.data);
                if !pasteboard_item.setData_forType(&data, &pasteboard_type) {
                    return Err(format!(
                        "could not restore {} clipboard data",
                        representation.uti
                    ));
                }
            }
            pasteboard_items.push(ProtocolObject::<dyn NSPasteboardWriting>::from_retained(
                pasteboard_item,
            ));
        }
        let objects = NSArray::from_retained_slice(&pasteboard_items);
        self.pasteboard.clearContents();
        if !self.pasteboard.writeObjects(&objects) {
            return Err("could not restore clipboard items".into());
        }
        self.last_change_count = self.pasteboard.changeCount();
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
struct PlatformClipboard {
    clipboard: arboard::Clipboard,
    last_text: Option<String>,
}

#[cfg(not(target_os = "macos"))]
impl PlatformClipboard {
    fn new() -> io::Result<Self> {
        Ok(Self {
            clipboard: arboard::Clipboard::new().map_err(io::Error::other)?,
            last_text: None,
        })
    }

    fn capture(
        &mut self,
        _settings: &ClipboardSettings,
    ) -> Result<Option<CapturedContent>, String> {
        let text = self
            .clipboard
            .get_text()
            .map_err(|error| error.to_string())?;
        if text.trim().is_empty() || self.last_text.as_ref() == Some(&text) {
            return Ok(None);
        }
        self.last_text = Some(text.clone());
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        Ok(Some(CapturedContent {
            kind: ClipKind::Text,
            text,
            representations: Vec::new(),
            fingerprint: hasher.finish(),
            thumbnail: None,
        }))
    }

    fn restore(&mut self, item: &ClipItem) -> Result<(), String> {
        self.clipboard
            .set_text(item.text.clone())
            .map_err(|error| error.to_string())?;
        self.last_text = Some(item.text.clone());
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn supported_type(uti: &str) -> bool {
    matches!(
        uti,
        "public.utf8-plain-text"
            | "public.utf16-external-plain-text"
            | "public.rtf"
            | "public.html"
            | "com.apple.flat-rtfd"
            | "public.png"
            | "public.tiff"
            | "public.file-url"
            | "public.url"
            | "NSFilenamesPboardType"
    )
}

#[cfg(target_os = "macos")]
fn pasteboard_is_sensitive(
    items: &objc2_foundation::NSArray<objc2_app_kit::NSPasteboardItem>,
) -> bool {
    const SENSITIVE_TYPES: [&str; 4] = [
        "org.nspasteboard.ConcealedType",
        "org.nspasteboard.TransientType",
        "org.nspasteboard.AutoGeneratedType",
        "com.agilebits.onepassword",
    ];
    for item_index in 0..items.count() {
        let types = items.objectAtIndex(item_index).types();
        for type_index in 0..types.count() {
            let uti = types.objectAtIndex(type_index).to_string();
            if SENSITIVE_TYPES.iter().any(|candidate| uti == *candidate) {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn current_application_is_ignored(ignored_bundle_ids: &[String]) -> bool {
    use objc2_app_kit::NSWorkspace;
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(application) = workspace.frontmostApplication() else {
        return false;
    };
    let Some(bundle_id) = application.bundleIdentifier() else {
        return false;
    };
    let bundle_id = bundle_id.to_string();
    ignored_bundle_ids
        .iter()
        .any(|ignored| ignored.eq_ignore_ascii_case(&bundle_id))
}

#[cfg(target_os = "macos")]
fn classify(representations: &[Representation]) -> ClipKind {
    let contains = |needle: &str| representations.iter().any(|value| value.uti == needle);
    if contains("public.file-url") || contains("NSFilenamesPboardType") {
        ClipKind::Files
    } else if contains("public.png") || contains("public.tiff") {
        ClipKind::Image
    } else if contains("public.rtf") || contains("public.html") || contains("com.apple.flat-rtfd") {
        ClipKind::RichText
    } else {
        ClipKind::Text
    }
}

fn make_thumbnail(representations: &[Representation]) -> Option<String> {
    use std::io::Cursor;

    let source = representations
        .iter()
        .find(|value| value.uti == "public.png" || value.uti == "public.tiff")?;
    let image = image::load_from_memory(&source.data).ok()?;
    let thumbnail = image.thumbnail(320, 240);
    let mut output = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut output, image::ImageFormat::Png)
        .ok()?;
    Some(BASE64.encode(output.into_inner()))
}

fn history_path() -> Option<PathBuf> {
    ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .map(|dirs| dirs.data_local_dir().join("history.ckh"))
}

fn legacy_history_path() -> Option<PathBuf> {
    ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .map(|dirs| dirs.data_local_dir().join("history.json"))
}

fn preview_cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .map(|dirs| dirs.cache_dir().join("image-previews"))
}

fn load_history(path: &Path, legacy_path: Option<&Path>, key: &[u8; 32]) -> Option<Vec<ClipItem>> {
    if let Ok(bytes) = fs::read(path) {
        let plaintext = decrypt_history(&bytes, key).ok()?;
        return serde_json::from_slice(&plaintext).ok();
    }
    legacy_path
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn encrypt_history(plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| "could not create encryption nonce")?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "could not initialize history encryption")?;
    let key = aead::LessSafeKey::new(unbound);
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(HISTORY_MAGIC.as_slice()),
        &mut ciphertext,
    )
    .map_err(|_| "could not encrypt clipboard history")?;
    let mut output = Vec::with_capacity(HISTORY_MAGIC.len() + nonce.len() + ciphertext.len());
    output.extend_from_slice(HISTORY_MAGIC);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn decrypt_history(bytes: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    if bytes.len() < HISTORY_MAGIC.len() + 12 || &bytes[..4] != HISTORY_MAGIC {
        return Err("invalid encrypted history".into());
    }
    let nonce: [u8; 12] = bytes[4..16]
        .try_into()
        .map_err(|_| "invalid history nonce")?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "could not initialize history decryption")?;
    let key = aead::LessSafeKey::new(unbound);
    let mut plaintext = bytes[16..].to_vec();
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(HISTORY_MAGIC.as_slice()),
            &mut plaintext,
        )
        .map_err(|_| "clipboard history could not be decrypted")?;
    Ok(plaintext.to_vec())
}

#[cfg(target_os = "macos")]
fn history_encryption_key() -> io::Result<[u8; 32]> {
    const SERVICE: &str = "io.clipkeep.ClipKeep.clipboard-history";
    const ACCOUNT: &str = "history";
    let existing = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
        .output()?;
    if existing.status.success() {
        let encoded = String::from_utf8_lossy(&existing.stdout).trim().to_owned();
        if let Ok(bytes) = BASE64.decode(encoded)
            && let Ok(key) = <[u8; 32]>::try_from(bytes)
        {
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| io::Error::other("could not generate history key"))?;
    let encoded = BASE64.encode(key);
    let status = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            SERVICE,
            "-a",
            ACCOUNT,
            "-w",
            &encoded,
        ])
        .status()?;
    if !status.success() {
        return Err(io::Error::other(
            "could not save clipboard history key in Keychain",
        ));
    }
    Ok(key)
}

#[cfg(not(target_os = "macos"))]
fn history_encryption_key() -> io::Result<[u8; 32]> {
    let path = ProjectDirs::from("io", "clipkeep", "ClipKeep")
        .ok_or_else(|| io::Error::other("could not determine key directory"))?
        .data_local_dir()
        .join("history.key");
    if let Ok(bytes) = fs::read(&path)
        && let Ok(key) = <[u8; 32]>::try_from(bytes)
    {
        return Ok(key);
    }
    let mut key = [0u8; 32];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| io::Error::other("could not generate history key"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, key)?;
    Ok(key)
}

fn trim_history(items: &mut Vec<ClipItem>, preview_cache_dir: &Path) {
    let mut total_bytes: usize = items.iter().map(ClipItem::byte_len).sum();
    let mut index = items.len();
    while (items.len() > MAX_HISTORY || total_bytes > MAX_TOTAL_BYTES) && index > 0 {
        index -= 1;
        if !items[index].favorite {
            total_bytes = total_bytes.saturating_sub(items[index].byte_len());
            let item = items.remove(index);
            item.remove_image_preview_files(preview_cache_dir);
        }
    }
}

fn promote_item(items: &mut Vec<ClipItem>, position: usize, created_at: u64) {
    let mut item = items.remove(position);
    item.created_at = created_at;
    items.insert(0, item);
}

fn preview(text: &str, max_chars: usize) -> String {
    let cleaned = text.replace('\r', "").trim().to_owned();
    let mut chars = cleaned.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn first_line_preview(text: &str, max_chars: usize) -> String {
    let has_more_lines = text.contains(['\r', '\n']);
    let cleaned = text.trim();
    let first_line = cleaned.lines().next().unwrap_or_default().trim();
    let content_limit = if has_more_lines {
        max_chars.saturating_sub(2)
    } else {
        max_chars
    };
    let first_line = preview(first_line, content_limit);
    if has_more_lines {
        format!("{first_line} ↩")
    } else {
        first_line
    }
}

fn byte_size(bytes: usize) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn age_label(created_at: u64) -> String {
    let elapsed = now_secs().saturating_sub(created_at);
    match elapsed {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

mod base64_bytes {
    use super::*;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        BASE64.decode(encoded).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_unicode_safe() {
        assert_eq!(preview("你好，剪贴板历史", 4), "你好，剪…");
    }

    #[test]
    fn list_preview_only_shows_the_first_line() {
        assert_eq!(
            first_line_preview("first line\nsecond line", 60),
            "first line ↩"
        );
        assert_eq!(first_line_preview("one line", 60), "one line");
        assert_eq!(first_line_preview("first\r\nsecond", 60), "first ↩");
        assert_eq!(
            first_line_preview("abcdefghijkl\nsecond", 10),
            "abcdefgh… ↩"
        );
        assert_eq!(first_line_preview("abcdefghijkl", 10), "abcdefghij…");
    }

    #[test]
    fn representation_uses_compact_base64_json() {
        let representation = Representation {
            item: 0,
            uti: "public.png".into(),
            data: vec![0, 1, 2, 255],
        };
        let json = serde_json::to_string(&representation).unwrap();
        let decoded: Representation = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.data, representation.data);
        assert!(json.contains("AAEC/w=="));
    }

    #[test]
    fn encrypted_history_round_trips_and_hides_plaintext() {
        let key = [7u8; 32];
        let plaintext = br#"[{"text":"secret clipboard value"}]"#;
        let encrypted = encrypt_history(plaintext, &key).unwrap();

        assert!(encrypted.starts_with(HISTORY_MAGIC));
        assert!(!encrypted.windows(6).any(|window| window == b"secret"));
        assert_eq!(decrypt_history(&encrypted, &key).unwrap(), plaintext);
    }

    #[test]
    fn promoting_an_existing_item_moves_it_to_the_front() {
        let item = |id, created_at| ClipItem {
            id,
            text: format!("item {id}"),
            created_at,
            favorite: false,
            kind: ClipKind::Text,
            representations: Vec::new(),
            fingerprint: id,
            thumbnail: None,
        };
        let mut items = vec![item(1, 10), item(2, 20), item(3, 30)];

        promote_item(&mut items, 2, 40);

        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            [3, 1, 2]
        );
        assert_eq!(items[0].created_at, 40);
    }

    #[test]
    fn subtitles_only_show_type_when_it_adds_information() {
        let item = |kind| ClipItem {
            id: 1,
            text: "content".into(),
            created_at: now_secs(),
            favorite: false,
            kind,
            representations: Vec::new(),
            fingerprint: 1,
            thumbnail: None,
        };
        let text = item(ClipKind::Text);
        let image = item(ClipKind::Image);
        let rich_text = item(ClipKind::RichText);

        assert_eq!(text.subtitle(), "just now");
        assert_eq!(image.subtitle(), "0 B · just now");
        assert_eq!(rich_text.subtitle(), "Rich text · just now");
    }

    #[test]
    fn edit_action_is_only_available_for_text_entries() {
        let item = |kind, text: &str| ClipItem {
            id: 1,
            text: text.into(),
            created_at: now_secs(),
            favorite: false,
            kind,
            representations: Vec::new(),
            fingerprint: 1,
            thumbnail: None,
        };
        let has_edit = |item: ClipItem| {
            item.as_result(0, "", &std::env::temp_dir())
                .actions
                .iter()
                .any(|action| action.id == "edit")
        };

        assert!(has_edit(item(ClipKind::Text, "plain text")));
        assert!(has_edit(item(ClipKind::RichText, "rich text")));
        assert!(!has_edit(item(ClipKind::Image, "")));
        assert!(!has_edit(item(ClipKind::Text, "")));
    }

    #[test]
    fn edited_text_is_stored_as_plain_text() {
        let content = plain_text_content("edited text".into());

        assert_eq!(content.kind, ClipKind::Text);
        assert_eq!(content.text, "edited text");
        assert_eq!(content.byte_len(), 11);
        assert!(content.representations.is_empty());
    }
}
