use crate::domain::{CaptureId, CaptureSource, CoreError, ImageFormat};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Recorded metadata for a completed capture task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: CaptureId,
    pub timestamp: DateTime<Utc>,
    pub source: CaptureSource,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub file_path: Option<PathBuf>,
    pub upload_url: Option<String>,
}

/// Persistence boundary for capture history.
pub trait HistoryRepository: Send + Sync {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError>;
    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError>;
    fn clear(&self) -> Result<(), CoreError>;
}

/// Append-only JSON Lines history store returning entries newest-first.
#[derive(Debug, Clone)]
pub struct JsonlHistoryStore {
    path: PathBuf,
}

impl JsonlHistoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl HistoryRepository for JsonlHistoryStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let line =
            serde_json::to_string(entry).map_err(|err| CoreError::HistoryError(err.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        let mut entries = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: HistoryEntry = serde_json::from_str(trimmed)
                .map_err(|err| CoreError::HistoryError(format!("line {}: {err}", idx + 1)))?;
            entries.push(entry);
        }
        entries.reverse();
        if let Some(max) = limit {
            entries.truncate(max);
        }
        Ok(entries)
    }

    fn clear(&self) -> Result<(), CoreError> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn jsonl_history_appends_lists_newest_first_and_clears() {
        let dir = tempdir().expect("tempdir");
        let store = JsonlHistoryStore::new(dir.path().join("history.jsonl"));
        assert!(store.list(None).expect("empty list").is_empty());

        let t1 = Utc::now();
        let e1 = HistoryEntry {
            id: CaptureId::from_raw("cap-1"),
            timestamp: t1,
            source: CaptureSource::PrimaryMonitor,
            width: 100,
            height: 80,
            format: ImageFormat::Png,
            file_path: Some(PathBuf::from("a.png")),
            upload_url: None,
        };
        let e2 = HistoryEntry {
            id: CaptureId::from_raw("cap-2"),
            timestamp: t1,
            source: CaptureSource::PrimaryMonitor,
            width: 200,
            height: 120,
            format: ImageFormat::Webp,
            file_path: Some(PathBuf::from("b.webp")),
            upload_url: Some("file:///b.webp".into()),
        };

        store.append(&e1).expect("append e1");
        store.append(&e2).expect("append e2");

        let listed = store.list(None).expect("list all");
        assert_eq!(listed, vec![e2.clone(), e1]);

        let limited = store.list(Some(1)).expect("list limit 1");
        assert_eq!(limited, vec![e2]);

        store.clear().expect("clear");
        assert!(store.list(None).expect("after clear").is_empty());
    }
}
