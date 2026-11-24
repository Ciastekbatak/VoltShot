pub mod cli;
pub mod clipboard;
pub mod editor_ui;
pub mod effects_ui;
pub mod gui;
pub mod hotkeys;
pub mod i18n;
pub mod pin_window;
pub mod recording_ui;
pub mod region_overlay;
pub mod theme;
pub mod thumb_cache;
pub mod tools_ui;
pub mod tray;
pub mod uploaders_ui;
pub mod win_focus;

use chrono::Utc;
use clap::Parser;
use cli::{Cli, Command, execute_headless};
use clipboard::SystemClipboard;
use sharex_capture::XcapCapturer;
use sharex_core::{
    AppConfig, CaptureId, CoreError, HistoryEntry, HistoryQuery, HistoryRepository,
    JsonlHistoryStore, SqliteHistoryStore,
};
use sharex_uploaders::UploaderRegistry;
use std::path::{Path, PathBuf};

/// Shared history backend used by both the desktop GUI and headless CLI.
pub enum AppHistoryStore {
    Sqlite {
        store: SqliteHistoryStore,
        legacy_jsonl: PathBuf,
    },
    Jsonl(JsonlHistoryStore),
}

impl AppHistoryStore {
    /// Opens the shared SQLite history store alongside `history_path`, migrating legacy JSONL once.
    pub fn open(history_path: &Path) -> (Self, Option<String>) {
        let sqlite_path = history_path.with_extension("sqlite");
        match SqliteHistoryStore::open(&sqlite_path) {
            Ok(store) => {
                let warning = if sqlite_path != history_path {
                    store
                        .migrate_from_jsonl_once(history_path)
                        .err()
                        .map(|err| format!("History import failed: {err}"))
                } else {
                    None
                };
                (
                    Self::Sqlite {
                        store,
                        legacy_jsonl: history_path.to_path_buf(),
                    },
                    warning,
                )
            }
            Err(err) => (
                Self::Jsonl(JsonlHistoryStore::new(history_path)),
                Some(format!("Using JSONL history. SQLite open failed: {err}")),
            ),
        }
    }
}

impl HistoryRepository for AppHistoryStore {
    fn append(&self, entry: &HistoryEntry) -> Result<(), CoreError> {
        match self {
            Self::Sqlite { store, .. } => store.append(entry),
            Self::Jsonl(store) => store.append(entry),
        }
    }

    fn list(&self, limit: Option<usize>) -> Result<Vec<HistoryEntry>, CoreError> {
        match self {
            Self::Sqlite { store, .. } => store.list(limit),
            Self::Jsonl(store) => store.list(limit),
        }
    }

    fn clear(&self) -> Result<(), CoreError> {
        match self {
            Self::Sqlite {
                store,
                legacy_jsonl,
            } => {
                store.clear()?;
                if legacy_jsonl.exists() {
                    let _ = JsonlHistoryStore::new(legacy_jsonl).clear();
                }
                Ok(())
            }
            Self::Jsonl(store) => store.clear(),
        }
    }

    fn search(&self, query: &HistoryQuery) -> Result<Vec<HistoryEntry>, CoreError> {
        match self {
            Self::Sqlite { store, .. } => store.search(query),
            Self::Jsonl(store) => store.search(query),
        }
    }

    fn set_tags(&self, id: &CaptureId, tags: &[String]) -> Result<bool, CoreError> {
        match self {
            Self::Sqlite { store, .. } => store.set_tags(id, tags),
            Self::Jsonl(store) => store.set_tags(id, tags),
        }
    }

    fn delete(&self, id: &CaptureId) -> Result<bool, CoreError> {
        match self {
            Self::Sqlite { store, .. } => store.delete(id),
            Self::Jsonl(store) => store.delete(id),
        }
    }
}

/// Entry point shared by the `sharex` binary.
pub fn run() -> Result<(), String> {
    let parsed = Cli::parse();
    let portable_base = resolve_portable_base_dir();
    let (config_path, fallback_config) =
        resolve_config_location(parsed.config.as_deref(), parsed.portable, &portable_base);

    let command = parsed.command.unwrap_or(Command::Gui);
    match command {
        Command::Gui => gui::launch_gui_with_fallback(config_path, fallback_config)
            .map_err(|err| err.to_string()),
        other => {
            let config = load_config_with_fallback(&config_path, fallback_config)
                .map_err(format_core_error)?;
            let mirror_dir = default_mirror_dir(&config);
            let capturer = XcapCapturer::new();
            let clipboard = SystemClipboard::new();
            let uploaders = UploaderRegistry::with_local_mirror(mirror_dir);
            let output = execute_headless(
                other,
                &config_path,
                config,
                &capturer,
                &clipboard,
                &uploaders,
                Utc::now(),
            )
            .map_err(format_core_error)?;
            if !output.is_empty() {
                println!("{output}");
            }
            Ok(())
        }
    }
}

/// Resolves the active config file path and default fallback config for standard or portable mode.
pub fn resolve_config_location(
    config_override: Option<&Path>,
    portable: bool,
    portable_base_dir: &Path,
) -> (PathBuf, AppConfig) {
    if portable {
        let path = config_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| AppConfig::portable_config_path(portable_base_dir));
        let fallback = AppConfig::portable_for_dir(portable_base_dir);
        (path, fallback)
    } else {
        let path = config_override
            .map(Path::to_path_buf)
            .unwrap_or_else(AppConfig::default_config_path);
        let fallback = AppConfig::default_for_os();
        (path, fallback)
    }
}

/// Loads configuration from `config_path` when present, or returns `fallback` when absent.
pub fn load_config_with_fallback(
    config_path: &Path,
    fallback: AppConfig,
) -> Result<AppConfig, CoreError> {
    if config_path.exists() {
        AppConfig::load_or_default(config_path)
    } else {
        Ok(fallback)
    }
}

fn resolve_portable_base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn default_mirror_dir(config: &AppConfig) -> PathBuf {
    config.output_dir.join("uploaded-mirror")
}

fn format_core_error(err: CoreError) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn portable_config_resolution_uses_portable_paths_and_fallback() {
        let dir = tempdir().expect("tempdir");
        let (path, fallback) = resolve_config_location(None, true, dir.path());

        assert_eq!(path, AppConfig::portable_config_path(dir.path()));
        assert_eq!(fallback.output_dir, dir.path().join("Screenshots"));
        assert_eq!(fallback.history_path, dir.path().join("history.jsonl"));

        let loaded =
            load_config_with_fallback(&path, fallback.clone()).expect("load absent portable");
        assert_eq!(loaded, fallback);

        let mut custom = fallback;
        custom.file_name_pattern = "Portable_%y%mo%d".into();
        custom.save(&path).expect("save portable config");

        let reloaded = load_config_with_fallback(&path, AppConfig::default_for_os())
            .expect("reload saved portable config");
        assert_eq!(reloaded, custom);
    }

    #[test]
    fn portable_mode_respects_explicit_config_override() {
        let dir = tempdir().expect("tempdir");
        let override_path = dir.path().join("custom.toml");
        let (path, fallback) = resolve_config_location(Some(&override_path), true, dir.path());

        assert_eq!(path, override_path);
        assert_eq!(fallback.output_dir, dir.path().join("Screenshots"));
    }
}
