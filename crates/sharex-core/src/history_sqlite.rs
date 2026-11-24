use crate::domain::{CaptureId, CoreError, ImageFormat};
use crate::history::{
    HistoryEntry, HistoryQuery, HistoryRepository, JsonlHistoryStore, unique_tags,
};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use rusqlite::functions::FunctionFlags;
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, Row, Transaction, TransactionBehavior, params, params_from_iter};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS captures (
        id TEXT PRIMARY KEY NOT NULL,
        timestamp TEXT NOT NULL,
        source_json TEXT NOT NULL,
        width INTEGER NOT NULL,
        height INTEGER NOT NULL,
        format TEXT NOT NULL,
        file_path TEXT,
        upload_url TEXT,
        note TEXT
    );
    CREATE TABLE IF NOT EXISTS capture_tags (
        capture_id TEXT NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
        tag TEXT NOT NULL,
        PRIMARY KEY (capture_id, tag)
    );
    CREATE INDEX IF NOT EXISTS captures_timestamp ON captures(CAST(timestamp AS INTEGER), timestamp);
    CREATE INDEX IF NOT EXISTS captures_format_timestamp
        ON captures(format, CAST(timestamp AS INTEGER), timestamp);
    CREATE INDEX IF NOT EXISTS capture_tags_tag ON capture_tags(tag, capture_id);
";

/// Capture history backed by a SQLite database.
#[derive(Debug)]
pub struct SqliteHistoryStore {
    connection: Mutex<Connection>,
}

impl SqliteHistoryStore {
    /// Opens or creates a history database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch(SCHEMA)?;
        connection.create_scalar_function(
            "history_lower",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(context
                    .get::<Option<String>>(0)?
                    .map(|text| text.to_lowercase()))
            },
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Imports a JSON Lines snapshot and returns the number of newly inserted captures.
    pub fn migrate_from_jsonl(&self, jsonl_path: &Path) -> Result<usize, CoreError> {
        let entries = JsonlHistoryStore::new(jsonl_path).read_entries()?;
        if entries.is_empty() {
            return Ok(0);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut imported = 0;
        for entry in entries {
            imported += usize::from(insert_capture(&transaction, &entry)?);
        }
        transaction.commit()?;
        Ok(imported)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, CoreError> {
        self.connection
            .lock()
            .map_err(|_| CoreError::HistoryError("SQLite history connection lock poisoned".into()))
    }
}

impl HistoryRepository for SqliteHistoryStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !insert_capture(&transaction, entry)? {
            return Err(CoreError::HistoryError(format!(
                "capture {} already exists",
                entry.id
            )));
        }
        transaction.commit()?;
        Ok(())
    }

    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError> {
        self.search(&HistoryQuery {
            limit,
            ..HistoryQuery::default()
        })
    }

    fn clear(&self) -> Result<(), CoreError> {
        self.connection()?.execute("DELETE FROM captures", [])?;
        Ok(())
    }

    fn search(&self, query: &HistoryQuery) -> Result<Vec<HistoryEntry>, CoreError> {
        let mut sql = String::from(
            "SELECT id, timestamp, source_json, width, height, format, file_path, upload_url, note,
                (SELECT json_group_array(tag ORDER BY rowid) FROM capture_tags WHERE capture_id = captures.id)
            FROM captures WHERE 1 = 1",
        );
        let mut values: Vec<Value> = Vec::new();
        if let Some(format) = query.format {
            sql.push_str(" AND format = ?");
            values.push(format.extension().to_owned().into());
        }
        if let Some(tag) = &query.tag {
            sql.push_str(" AND id IN (SELECT capture_id FROM capture_tags WHERE tag = ?)");
            values.push(tag.clone().into());
        }
        if let Some(since) = query.since {
            sql.push_str(" AND (CAST(timestamp AS INTEGER), timestamp) >= (?, ?)");
            values.extend([since.year().into(), format_timestamp(since).into()]);
        }
        if let Some(until) = query.until {
            sql.push_str(" AND (CAST(timestamp AS INTEGER), timestamp) <= (?, ?)");
            values.extend([until.year().into(), format_timestamp(until).into()]);
        }
        if let Some(text) = query.text.as_deref().filter(|text| !text.is_empty()) {
            let parameter = values.len() + 1;
            sql.push_str(&format!(
                " AND (instr(history_lower(id), ?{parameter})
                    OR instr(history_lower(file_path), ?{parameter})
                    OR instr(history_lower(upload_url), ?{parameter})
                    OR instr(history_lower(note), ?{parameter})
                    OR EXISTS (SELECT 1 FROM capture_tags
                        WHERE capture_id = captures.id AND instr(history_lower(tag), ?{parameter})))"
            ));
            values.push(text.to_lowercase().into());
        }
        sql.push_str(
            " ORDER BY CAST(timestamp AS INTEGER) DESC, timestamp DESC, rowid DESC LIMIT ?",
        );
        let limit = query
            .limit
            .and_then(|limit| i64::try_from(limit).ok())
            .unwrap_or(-1);
        values.push(limit.into());

        let connection = self.connection()?;
        let mut statement = connection.prepare_cached(&sql)?;
        let rows = statement.query_map(params_from_iter(values), read_capture)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn delete(&self, id: &CaptureId) -> Result<bool, CoreError> {
        let deleted = self
            .connection()?
            .execute("DELETE FROM captures WHERE id = ?", [id.as_str()])?;
        Ok(deleted != 0)
    }

    fn set_tags(&self, id: &CaptureId, tags: &[String]) -> Result<bool, CoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM captures WHERE id = ?)",
            [id.as_str()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }
        transaction.execute(
            "DELETE FROM capture_tags WHERE capture_id = ?",
            [id.as_str()],
        )?;
        insert_tags(&transaction, id, tags)?;
        transaction.commit()?;
        Ok(true)
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::HistoryError(error.to_string())
    }
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn insert_capture(transaction: &Transaction<'_>, entry: &HistoryEntry) -> Result<bool, CoreError> {
    let source_json = serde_json::to_string(&entry.source)
        .map_err(|error| CoreError::HistoryError(error.to_string()))?;
    let file_path = match &entry.file_path {
        Some(path) => Some(path.to_str().ok_or_else(|| {
            CoreError::HistoryError(format!("capture {} has a non-Unicode file path", entry.id))
        })?),
        None => None,
    };
    let inserted = transaction
        .prepare_cached(
            "INSERT OR IGNORE INTO captures
            (id, timestamp, source_json, width, height, format, file_path, upload_url, note)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?
        .execute(params![
            entry.id.as_str(),
            format_timestamp(entry.timestamp),
            source_json,
            entry.width,
            entry.height,
            entry.format.extension(),
            file_path,
            entry.upload_url,
            entry.note,
        ])?;
    if inserted != 0 {
        insert_tags(transaction, &entry.id, &entry.tags)?;
    }
    Ok(inserted != 0)
}

fn insert_tags(
    transaction: &Transaction<'_>,
    id: &CaptureId,
    tags: &[String],
) -> Result<(), CoreError> {
    let mut statement =
        transaction.prepare_cached("INSERT INTO capture_tags (capture_id, tag) VALUES (?, ?)")?;
    for tag in unique_tags(tags) {
        statement.execute(params![id.as_str(), tag])?;
    }
    Ok(())
}

fn read_capture(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: CaptureId::from_raw(row.get::<_, String>(0)?),
        timestamp: decode_text(row, 1, str::parse::<DateTime<Utc>>)?,
        source: decode_text(row, 2, |value| serde_json::from_str(value))?,
        width: row.get(3)?,
        height: row.get(4)?,
        format: decode_text(row, 5, str::parse::<ImageFormat>)?,
        file_path: row.get::<_, Option<String>>(6)?.map(Into::into),
        upload_url: row.get(7)?,
        note: row.get(8)?,
        tags: decode_text(row, 9, |value| serde_json::from_str(value))?,
    })
}

fn decode_text<T, E: Error + Send + Sync + 'static>(
    row: &Row<'_>,
    column: usize,
    decode: impl FnOnce(&str) -> Result<T, E>,
) -> rusqlite::Result<T> {
    decode(&row.get::<_, String>(column)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CaptureSource, Region};
    use tempfile::tempdir;

    fn capture(id: &str, timestamp: &str, format: ImageFormat) -> HistoryEntry {
        HistoryEntry {
            id: CaptureId::from_raw(id),
            timestamp: timestamp.parse().expect("timestamp"),
            source: CaptureSource::Region {
                region: Region::new(-10, 20, 100, 80).unwrap(),
            },
            width: 100,
            height: 80,
            format,
            file_path: Some(format!("shots/{id}.{}", format.extension()).into()),
            upload_url: None,
            tags: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn sqlite_persists_entries_and_combines_search_filters_before_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("history.sqlite");
        let oldest = capture("cap-old", "2026-10-12T10:00:00Z", ImageFormat::Png);
        let mut screenshot = capture(
            "cap_100%",
            "2026-10-12T10:00:00.000000001Z",
            ImageFormat::Jpeg,
        );
        screenshot.note = Some("Zrzut ekranu ŻÓŁTY".into());
        screenshot.upload_url = Some("https://example.com/files/a.jpg".into());
        screenshot.tags = vec!["Work".into(), "album's best".into()];
        let newest = capture("capX100Z", "2026-10-12T10:00:01Z", ImageFormat::Webp);

        let store = SqliteHistoryStore::open(&path).unwrap();
        assert!(store.list(None).unwrap().is_empty());
        store.append(&newest).unwrap();
        store.append(&oldest).unwrap();
        store.append(&screenshot).unwrap();
        assert!(store.append(&screenshot).is_err());
        drop(store);

        let store = SqliteHistoryStore::open(path).unwrap();
        assert_eq!(store.list(Some(1)).unwrap(), vec![newest.clone()]);
        assert!(store.list(Some(0)).unwrap().is_empty());
        assert_eq!(
            store.list(None).unwrap(),
            vec![newest, screenshot.clone(), oldest.clone()]
        );

        for text in [
            "SHOTS/CAP_100%.JPG",
            "example.COM/files",
            "żółty",
            "ALBUM'S BEST",
            "_100%",
        ] {
            let query = HistoryQuery {
                text: Some(text.into()),
                ..HistoryQuery::default()
            };
            assert_eq!(store.search(&query).unwrap(), vec![screenshot.clone()]);
        }
        let mut query = HistoryQuery {
            text: Some("cap".into()),
            format: Some(ImageFormat::Jpeg),
            tag: Some("Work".into()),
            since: Some(screenshot.timestamp),
            until: Some(screenshot.timestamp),
            limit: Some(1),
        };
        assert_eq!(store.search(&query).unwrap(), vec![screenshot.clone()]);
        query.tag = Some("work".into());
        assert!(store.search(&query).unwrap().is_empty());
        query.tag = None;
        query.format = Some(ImageFormat::Png);
        assert!(store.search(&query).unwrap().is_empty());
        query.format = None;
        query.until = Some(oldest.timestamp);
        assert!(store.search(&query).unwrap().is_empty());
        query.since = None;
        assert_eq!(store.search(&query).unwrap(), vec![oldest]);
    }

    #[test]
    fn sqlite_orders_and_filters_timestamps_across_signed_years() {
        let store = SqliteHistoryStore::open(":memory:").unwrap();
        let oldest = capture("oldest", "-0020-01-01T00:00:00Z", ImageFormat::Png);
        let middle = capture("middle", "-0002-01-01T00:00:00Z", ImageFormat::Png);
        let newest = capture("newest", "+10000-01-01T00:00:00Z", ImageFormat::Png);
        store.append(&middle).unwrap();
        store.append(&newest).unwrap();
        store.append(&oldest).unwrap();
        assert_eq!(
            store.list(None).unwrap(),
            vec![newest, middle.clone(), oldest]
        );
        let query = HistoryQuery {
            since: Some(middle.timestamp),
            until: Some("0000-01-01T00:00:00Z".parse().unwrap()),
            ..HistoryQuery::default()
        };
        assert_eq!(store.search(&query).unwrap(), vec![middle]);
    }

    #[test]
    fn sqlite_replaces_tags_and_cascades_capture_deletions() {
        let store = SqliteHistoryStore::open(":memory:").unwrap();
        let mut first = capture("cap-1", "2026-10-12T10:00:00Z", ImageFormat::Png);
        first.tags = vec!["before".into(), "before".into()];
        let mut second = capture("cap-2", "2026-10-12T10:00:00Z", ImageFormat::Png);
        second.tags = vec!["untouched".into()];
        store.append(&first).unwrap();
        store.append(&second).unwrap();
        first.tags.pop();
        assert_eq!(
            store.list(None).unwrap(),
            vec![second.clone(), first.clone()]
        );

        let tags = ["z".into(), "a".into(), "z".into()];
        assert!(store.set_tags(&first.id, &tags).unwrap());
        first.tags = tags[..2].to_vec();
        assert_eq!(
            store.list(None).unwrap(),
            vec![second.clone(), first.clone()]
        );
        assert!(store.set_tags(&second.id, &[]).unwrap());
        second.tags.clear();
        assert_eq!(
            store.list(None).unwrap(),
            vec![second.clone(), first.clone()]
        );

        assert!(store.delete(&first.id).unwrap());
        assert!(!store.delete(&first.id).unwrap());
        assert!(!store.set_tags(&first.id, &tags).unwrap());
        assert_eq!(store.list(None).unwrap(), vec![second.clone()]);
        store.append(&first).unwrap();
        assert_eq!(store.list(None).unwrap(), vec![first, second]);
        store.clear().unwrap();
        assert!(store.list(None).unwrap().is_empty());
        let tag_count: i64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM capture_tags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tag_count, 0);
    }

    #[test]
    fn sqlite_migrates_jsonl_idempotently_without_overwriting_existing_captures() {
        let dir = tempdir().unwrap();
        let source = JsonlHistoryStore::new(dir.path().join("history.jsonl"));
        let store = SqliteHistoryStore::open(dir.path().join("history.sqlite")).unwrap();
        assert_eq!(store.migrate_from_jsonl(source.path()).unwrap(), 0);

        let mut legacy = capture("cap-legacy", "2026-10-12T10:00:00Z", ImageFormat::Png);
        let mut value = serde_json::to_value(&legacy).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("tags");
        fields.remove("note");
        fs::write(source.path(), format!("{value}\n")).unwrap();
        let mut tagged = capture("cap-tagged", "2026-10-12T10:00:01Z", ImageFormat::Webp);
        tagged.tags = vec!["imported".into()];
        tagged.note = Some("migrated note".into());
        source.append(&tagged).unwrap();
        let mut duplicate = tagged.clone();
        duplicate.tags = vec!["duplicate tag".into()];
        source.append(&duplicate).unwrap();

        assert_eq!(store.migrate_from_jsonl(source.path()).unwrap(), 2);
        assert_eq!(
            store.list(None).unwrap(),
            vec![tagged.clone(), legacy.clone()]
        );
        legacy.tags = vec!["edited after migration".into()];
        assert!(store.set_tags(&legacy.id, &legacy.tags).unwrap());
        assert_eq!(store.migrate_from_jsonl(source.path()).unwrap(), 0);
        assert_eq!(store.list(None).unwrap(), vec![tagged, legacy]);
        assert_eq!(source.list(None).unwrap().len(), 3);
    }

    #[test]
    fn sqlite_rejects_malformed_jsonl_without_partial_import() {
        let dir = tempdir().unwrap();
        let source = JsonlHistoryStore::new(dir.path().join("history.jsonl"));
        source
            .append(&capture("cap-1", "2026-10-12T10:00:00Z", ImageFormat::Png))
            .unwrap();
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(source.path())
            .unwrap();
        use std::io::Write;
        writeln!(file, "malformed").unwrap();
        drop(file);
        let store = SqliteHistoryStore::open(":memory:").unwrap();
        assert!(
            matches!(store.migrate_from_jsonl(source.path()), Err(CoreError::HistoryError(error)) if error.starts_with("line 2:"))
        );
        assert!(store.list(None).unwrap().is_empty());
    }
}
