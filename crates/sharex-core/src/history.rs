use crate::domain::{CaptureId, CaptureSource, CoreError, ImageFormat};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

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
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Filters combined with AND, applied before limiting the result count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Case-insensitive literal substring in the ID, file path, URL, note, or any tag.
    pub text: Option<String>,
    pub format: Option<ImageFormat>,
    /// Exact, case-sensitive tag.
    pub tag: Option<String>,
    /// Inclusive UTC lower bound.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive UTC upper bound.
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

/// Persistence boundary for capture history.
pub trait HistoryRepository: Send + Sync {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError>;
    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError>;
    fn clear(&self) -> Result<(), CoreError>;

    fn search(&self, query: &HistoryQuery) -> Result<Vec<HistoryEntry>, CoreError> {
        let text = query.text.as_deref().filter(|value| !value.is_empty());
        let text = text.map(str::to_lowercase);
        Ok(self
            .list(None)?
            .into_iter()
            .filter(|entry| {
                query.format.is_none_or(|format| entry.format == format)
                    && query
                        .tag
                        .as_ref()
                        .is_none_or(|tag| entry.tags.contains(tag))
                    && query.since.is_none_or(|since| entry.timestamp >= since)
                    && query.until.is_none_or(|until| entry.timestamp <= until)
                    && text.as_ref().is_none_or(|text| {
                        [
                            Some(entry.id.as_str()),
                            entry.file_path.as_deref().and_then(Path::to_str),
                            entry.upload_url.as_deref(),
                            entry.note.as_deref(),
                        ]
                        .into_iter()
                        .flatten()
                        .chain(entry.tags.iter().map(String::as_str))
                        .any(|value| value.to_lowercase().contains(text))
                    })
            })
            .take(query.limit.unwrap_or(usize::MAX))
            .collect())
    }

    /// Deletes the identified capture and returns whether it existed.
    fn delete(&self, id: &CaptureId) -> Result<bool, CoreError>;

    /// Replaces the identified capture's tags and returns whether it existed.
    fn set_tags(&self, id: &CaptureId, tags: &[String]) -> Result<bool, CoreError>;
}

pub(crate) fn unique_tags(tags: &[String]) -> impl Iterator<Item = &str> {
    let mut seen = HashSet::new();
    tags.iter()
        .map(String::as_str)
        .filter(move |tag| seen.insert(*tag))
}

/// JSON Lines history store returning the most recently appended entries first.
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

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn lock_path(&self) -> PathBuf {
        let mut path = self.path.as_os_str().to_os_string();
        path.push(".lock");
        path.into()
    }

    fn write_lock(&self) -> Result<File, CoreError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path())?;
        FileExt::lock_exclusive(&lock)?;
        Ok(lock)
    }

    fn read_lock(&self) -> Result<Option<File>, CoreError> {
        match File::open(self.lock_path()) {
            Ok(lock) => {
                FileExt::lock_shared(&lock)?;
                Ok(Some(lock))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn read_entries(&self) -> Result<Vec<HistoryEntry>, CoreError> {
        let lock = self.read_lock()?;
        let content = fs::read_to_string(&self.path);
        if lock.is_none()
            && let Some(_lock) = self.read_lock()?
        {
            return self.read_entries_unlocked();
        }
        parse_entries(content)
    }

    fn read_entries_unlocked(&self) -> Result<Vec<HistoryEntry>, CoreError> {
        parse_entries(fs::read_to_string(&self.path))
    }

    fn rewrite(&self, entries: &[HistoryEntry]) -> Result<(), CoreError> {
        let mut file = NamedTempFile::new_in(self.parent())?;
        for entry in entries {
            file.write_all(&encode_entry(entry)?)?;
        }
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn parse_entries(content: std::io::Result<String>) -> Result<Vec<HistoryEntry>, CoreError> {
    let content = match content {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| (idx, line.trim()))
        .filter(|(_, line)| !line.is_empty())
        .map(|(idx, line)| {
            serde_json::from_str(line)
                .map_err(|err| CoreError::HistoryError(format!("line {}: {err}", idx + 1)))
        })
        .collect()
}

fn encode_entry(entry: &HistoryEntry) -> Result<Vec<u8>, CoreError> {
    let mut line =
        serde_json::to_vec(entry).map_err(|error| CoreError::HistoryError(error.to_string()))?;
    line.push(b'\n');
    Ok(line)
}

impl HistoryRepository for JsonlHistoryStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError> {
        let mut line = encode_entry(entry)?;
        fs::create_dir_all(self.parent())?;
        let _lock = self.write_lock()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.path)?;
        let length = file.metadata()?.len();
        if length != 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last_byte = [0];
            file.read_exact(&mut last_byte)?;
            if last_byte[0] != b'\n' {
                line.insert(0, b'\n');
            }
        }
        if let Err(error) = file.write_all(&line) {
            file.set_len(length)?;
            return Err(error.into());
        }
        Ok(())
    }

    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError> {
        let mut entries = self.read_entries()?;
        entries.reverse();
        if let Some(max) = limit {
            entries.truncate(max);
        }
        Ok(entries)
    }

    fn clear(&self) -> Result<(), CoreError> {
        if !self.path.try_exists()? {
            return Ok(());
        }
        let _lock = self.write_lock()?;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn delete(&self, id: &CaptureId) -> Result<bool, CoreError> {
        if !self.path.try_exists()? {
            return Ok(false);
        }
        let _lock = self.write_lock()?;
        let mut entries = self.read_entries_unlocked()?;
        let original_count = entries.len();
        entries.retain(|entry| &entry.id != id);
        if entries.len() == original_count {
            return Ok(false);
        }
        self.rewrite(&entries)?;
        Ok(true)
    }

    fn set_tags(&self, id: &CaptureId, tags: &[String]) -> Result<bool, CoreError> {
        if !self.path.try_exists()? {
            return Ok(false);
        }
        let _lock = self.write_lock()?;
        let mut entries = self.read_entries_unlocked()?;
        let tags: Vec<_> = unique_tags(tags).map(str::to_owned).collect();
        let mut found = false;
        for entry in entries.iter_mut().filter(|entry| &entry.id == id) {
            entry.tags.clone_from(&tags);
            found = true;
        }
        if found {
            self.rewrite(&entries)?;
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn jsonl_opens_legacy_history_and_persists_tag_updates_and_deletions() {
        let dir = tempdir().expect("tempdir");
        let store = JsonlHistoryStore::new(dir.path().join("history.jsonl"));
        fs::write(
            store.path(),
            r#"{"id":"cap-legacy","timestamp":"2026-10-12T10:00:00Z","source":{"kind":"primary_monitor"},"width":100,"height":80,"format":"png","file_path":"legacy.png","upload_url":null}"#,
        )
        .expect("write old history");
        let [mut entry]: [HistoryEntry; 1] = store.list(None).unwrap().try_into().unwrap();
        assert!(entry.tags.is_empty());
        assert_eq!(entry.note, None);
        let mut recent = entry.clone();
        recent.id = CaptureId::from_raw("cap-recent");
        recent.note = Some("preserve this note".into());
        store.append(&recent).unwrap();
        assert_eq!(
            store.list(None).unwrap(),
            vec![recent.clone(), entry.clone()]
        );

        let query = HistoryQuery {
            limit: Some(1),
            text: Some("LEGACY.PNG".into()),
            format: Some(ImageFormat::Png),
            tag: Some("Żółty".into()),
            since: Some(entry.timestamp),
            until: Some(entry.timestamp),
        };
        assert!(store.search(&query).unwrap().is_empty());
        assert!(
            store
                .set_tags(&entry.id, &["Żółty".into(), "Żółty".into()])
                .unwrap()
        );
        entry.tags = vec!["Żółty".into()];
        let reopened = JsonlHistoryStore::new(store.path());
        assert_eq!(reopened.search(&query).unwrap(), vec![entry.clone()]);
        assert!(reopened.set_tags(&entry.id, &[]).unwrap());
        assert!(store.search(&query).unwrap().is_empty());
        assert!(store.delete(&entry.id).unwrap());
        assert!(!store.delete(&entry.id).unwrap());
        assert!(!store.set_tags(&entry.id, &entry.tags).unwrap());
        assert_eq!(store.list(None).unwrap(), vec![recent]);
    }

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
            tags: Vec::new(),
            note: None,
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
            tags: Vec::new(),
            note: None,
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
