use crate::model::ClipItem;
use directories::ProjectDirs;
use std::{fs, io, path::PathBuf};

const APP_QUALIFIER: &str = "io";
const APP_ORG: &str = "clipkeep";
const APP_NAME: &str = "ClipKeep";

pub fn history_path() -> Option<PathBuf> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .map(|dirs| dirs.data_local_dir().join("history.json"))
}

pub fn load() -> Vec<ClipItem> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(items: &[ClipItem]) -> io::Result<()> {
    let path = history_path()
        .ok_or_else(|| io::Error::other("could not determine application data directory"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(items).map_err(io::Error::other)?;
    fs::write(&temp_path, bytes)?;
    fs::rename(temp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trip_format() {
        let items = vec![ClipItem::new(7, "hello".into())];
        let bytes = serde_json::to_vec(&items).unwrap();
        let restored: Vec<ClipItem> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(items, restored);
    }
}
