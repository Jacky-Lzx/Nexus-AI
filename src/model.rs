use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipItem {
    pub id: u64,
    pub text: String,
    pub created_at: u64,
    #[serde(default)]
    pub favorite: bool,
}

impl ClipItem {
    pub fn new(id: u64, text: String) -> Self {
        Self {
            id,
            text,
            created_at: now_secs(),
            favorite: false,
        }
    }

    pub fn preview(&self, max_chars: usize) -> String {
        let cleaned = self.text.replace('\r', "").trim().to_owned();
        let mut chars = cleaned.chars();
        let preview: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{preview}…")
        } else {
            preview
        }
    }

    pub fn age_label(&self) -> String {
        let elapsed = now_secs().saturating_sub(self.created_at);
        match elapsed {
            0..=59 => "just now".to_owned(),
            60..=3_599 => format!("{}m ago", elapsed / 60),
            3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
            _ => format!("{}d ago", elapsed / 86_400),
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_unicode_safe() {
        let item = ClipItem::new(1, "你好，剪贴板历史".into());
        assert_eq!(item.preview(4), "你好，剪…");
    }
}
