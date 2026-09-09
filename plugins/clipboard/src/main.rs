use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io,
    path::PathBuf,
    time::SystemTime,
};

const PLUGIN_ID: &str = "io.clipkeep.clipboard";
const MAX_HISTORY: usize = 500;
const MAX_ITEM_BYTES: usize = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 250 * 1024 * 1024;

fn main() -> io::Result<()> {
    run_plugin(ClipboardPlugin::new()?)
}

struct ClipboardPlugin {
    clipboard: PlatformClipboard,
    items: Vec<ClipItem>,
    next_id: u64,
    history_path: PathBuf,
}

impl ClipboardPlugin {
    fn new() -> io::Result<Self> {
        let history_path = history_path()
            .ok_or_else(|| io::Error::other("could not determine clipboard data directory"))?;
        let mut items: Vec<ClipItem> = fs::read(&history_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
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
        };
        if added_thumbnails {
            plugin.save().map_err(io::Error::other)?;
        }
        Ok(plugin)
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.history_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temp = self.history_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.items).map_err(|error| error.to_string())?;
        fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        fs::rename(temp, &self.history_path).map_err(|error| error.to_string())
    }

    fn item_id(raw: &str) -> Result<u64, String> {
        raw.strip_prefix("clipboard:")
            .ok_or_else(|| "invalid clipboard result id".to_owned())?
            .parse()
            .map_err(|_| "invalid clipboard result id".to_owned())
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
            .map(|(index, item)| item.as_result(index, &query))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
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
                self.items.retain(|item| item.id != id);
                self.save()?;
                Ok(Some("Deleted".to_owned()))
            }
            _ => Err(format!("unknown clipboard action: {action_id}")),
        }
    }

    fn tick(&mut self) -> Result<bool, String> {
        let Some(content) = self.clipboard.capture()? else {
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
        trim_history(&mut self.items);
        self.save()?;
        Ok(true)
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
        self.representations
            .iter()
            .map(|value| value.data.len())
            .sum()
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
        self.representations
            .iter()
            .map(|value| value.data.len())
            .sum()
    }

    fn search_text(&self) -> String {
        if self.text.is_empty() {
            self.kind.label().to_owned()
        } else {
            self.text.clone()
        }
    }

    fn as_result(&self, index: usize, query: &str) -> SearchResult {
        let mut metadata = BTreeMap::new();
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
            && let Some(thumbnail) = self.thumbnail_data()
        {
            metadata.insert("thumbnail_png".into(), thumbnail);
        }
        if matches!(self.kind, ClipKind::Text | ClipKind::RichText) {
            metadata.insert("characters".into(), self.text.chars().count().to_string());
            metadata.insert(
                "words".into(),
                self.text.split_whitespace().count().to_string(),
            );
        }
        let title = match self.kind {
            ClipKind::Text | ClipKind::RichText if !self.text.is_empty() => preview(&self.text, 60),
            ClipKind::Image => format!("Image · {}", byte_size(self.byte_len())),
            ClipKind::Files if !self.text.is_empty() => preview(&self.text, 60),
            ClipKind::Files => "Files".to_owned(),
            _ => self.kind.label().to_owned(),
        };
        let match_bonus =
            if !query.is_empty() && self.search_text().to_lowercase().starts_with(query) {
                50.0
            } else {
                0.0
            };
        SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("clipboard:{}", self.id),
            title,
            subtitle: format!("{} · {}", self.kind.label(), age_label(self.created_at)),
            preview: if self.text.is_empty() {
                format!("{} clipboard content", self.kind.label())
            } else {
                self.text.clone()
            },
            kind: self.kind.result_kind(),
            score: 1_000.0 - index as f32 + match_bonus + if self.favorite { 10.0 } else { 0.0 },
            metadata,
            actions: vec![
                ResultAction {
                    id: "copy".into(),
                    title: "Paste".into(),
                    destructive: false,
                },
                ResultAction {
                    id: "favorite".into(),
                    title: "Favorite".into(),
                    destructive: false,
                },
                ResultAction {
                    id: "delete".into(),
                    title: "Delete".into(),
                    destructive: true,
                },
            ],
        }
    }

    fn thumbnail_data(&self) -> Option<String> {
        self.thumbnail
            .clone()
            .or_else(|| make_thumbnail(&self.representations))
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

    fn capture(&mut self) -> Result<Option<CapturedContent>, String> {
        let change_count = self.pasteboard.changeCount();
        if change_count == self.last_change_count {
            return Ok(None);
        }
        self.last_change_count = change_count;
        let Some(items) = self.pasteboard.pasteboardItems() else {
            return Ok(None);
        };

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

    fn capture(&mut self) -> Result<Option<CapturedContent>, String> {
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
        .map(|dirs| dirs.data_local_dir().join("history.json"))
}

fn trim_history(items: &mut Vec<ClipItem>) {
    let mut total_bytes: usize = items.iter().map(ClipItem::byte_len).sum();
    let mut index = items.len();
    while (items.len() > MAX_HISTORY || total_bytes > MAX_TOTAL_BYTES) && index > 0 {
        index -= 1;
        if !items[index].favorite {
            total_bytes = total_bytes.saturating_sub(items[index].byte_len());
            items.remove(index);
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
}
