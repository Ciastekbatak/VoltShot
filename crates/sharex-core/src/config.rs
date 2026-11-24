use crate::domain::{CoreError, ImageFormat, Region};
use crate::hotkeys::{HotkeyBinding, default_sharex_hotkeys};
use crate::naming::FileNamePattern;
use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_FILE_NAME_PATTERN: &str = "ShareX_%y-%mo-%d_%h-%mi-%s";

/// After-capture actions executed by `CapturePipeline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub save_to_file: bool,
    pub copy_image_to_clipboard: bool,
    pub copy_path_to_clipboard: bool,
    pub record_history: bool,
    #[serde(default)]
    pub open_in_editor: bool,
    pub uploader_id: Option<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            save_to_file: true,
            copy_image_to_clipboard: true,
            copy_path_to_clipboard: false,
            record_history: true,
            open_in_editor: false,
            uploader_id: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_ui_language() -> String {
    "en".into()
}

/// Persisted application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub output_dir: PathBuf,
    pub file_name_pattern: String,
    pub image_format: ImageFormat,
    pub pipeline: PipelineConfig,
    pub history_path: PathBuf,
    #[serde(default = "default_sharex_hotkeys")]
    pub hotkeys: Vec<HotkeyBinding>,
    #[serde(default)]
    pub last_region: Option<Region>,
    #[serde(default = "default_true")]
    pub dark_theme: bool,
    #[serde(default = "default_true")]
    pub show_toasts: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub play_capture_sound: bool,
    #[serde(default = "default_ui_language")]
    pub ui_language: String,
}

impl AppConfig {
    pub fn default_for_os() -> Self {
        let data_dir = ProjectDirs::from("org", "ShareX", "ShareX-RS")
            .map(|dirs| dirs.data_local_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("sharex-data"));

        let output_dir = UserDirs::new()
            .and_then(|dirs| dirs.picture_dir().map(|p| p.join("ShareX-RS")))
            .unwrap_or_else(|| data_dir.join("Screenshots"));

        let history_path = data_dir.join("history.jsonl");

        Self {
            output_dir,
            file_name_pattern: DEFAULT_FILE_NAME_PATTERN.to_owned(),
            image_format: ImageFormat::Png,
            pipeline: PipelineConfig::default(),
            history_path,
            hotkeys: default_sharex_hotkeys(),
            last_region: None,
            dark_theme: true,
            show_toasts: true,
            close_to_tray: true,
            start_minimized: false,
            play_capture_sound: false,
            ui_language: default_ui_language(),
        }
    }

    pub fn default_config_path() -> PathBuf {
        ProjectDirs::from("org", "ShareX", "ShareX-RS")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("sharex-data").join("config.toml"))
    }

    /// Creates a self-contained portable configuration rooted at `base_dir`.
    pub fn portable_for_dir(base_dir: &Path) -> Self {
        Self {
            output_dir: base_dir.join("Screenshots"),
            file_name_pattern: DEFAULT_FILE_NAME_PATTERN.to_owned(),
            image_format: ImageFormat::Png,
            pipeline: PipelineConfig::default(),
            history_path: base_dir.join("history.jsonl"),
            hotkeys: default_sharex_hotkeys(),
            last_region: None,
            dark_theme: true,
            show_toasts: true,
            close_to_tray: true,
            start_minimized: false,
            play_capture_sound: false,
            ui_language: default_ui_language(),
        }
    }

    /// Returns the portable configuration file path inside `base_dir`.
    pub fn portable_config_path(base_dir: &Path) -> PathBuf {
        base_dir.join("config.toml")
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        FileNamePattern::parse(&self.file_name_pattern)?;
        if self.output_dir.as_os_str().is_empty() {
            return Err(CoreError::ConfigError("output_dir cannot be empty".into()));
        }
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Result<Self, CoreError> {
        if !path.exists() {
            return Ok(Self::default_for_os());
        }
        let raw = fs::read_to_string(path)?;
        let parsed: Self =
            toml::from_str(&raw).map_err(|err| CoreError::ConfigError(err.to_string()))?;
        parsed.validate()?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        self.validate()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let encoded =
            toml::to_string_pretty(self).map_err(|err| CoreError::ConfigError(err.to_string()))?;
        fs::write(path, encoded)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_round_trips_through_toml_and_validates_on_load() {
        let dir = tempdir().expect("tempdir");
        let cfg_path = dir.path().join("nested").join("config.toml");

        let mut cfg = AppConfig::default_for_os();
        cfg.output_dir = dir.path().join("shots");
        cfg.image_format = ImageFormat::Webp;
        cfg.pipeline.copy_path_to_clipboard = true;
        cfg.pipeline.uploader_id = Some("local-mirror".into());
        cfg.show_toasts = false;
        cfg.close_to_tray = true;
        cfg.start_minimized = true;
        cfg.play_capture_sound = true;
        cfg.ui_language = "pl".into();

        cfg.save(&cfg_path).expect("save config");
        let loaded = AppConfig::load_or_default(&cfg_path).expect("load config");
        assert_eq!(loaded, cfg);
    }

    #[test]
    fn legacy_toml_defaults_desktop_preferences() {
        let cfg: AppConfig = toml::from_str(
            r#"
output_dir = "Screenshots"
file_name_pattern = "ShareX_%y-%mo-%d"
image_format = "png"
history_path = "history.jsonl"

[pipeline]
save_to_file = true
copy_image_to_clipboard = true
copy_path_to_clipboard = false
record_history = true
"#,
        )
        .expect("legacy config");
        assert!(cfg.dark_theme);
        assert!(cfg.show_toasts);
        assert!(cfg.close_to_tray);
        assert!(!cfg.start_minimized);
        assert!(!cfg.play_capture_sound);
        assert_eq!(cfg.ui_language, "en");
    }

    #[test]
    fn config_rejects_empty_filename_pattern() {
        let dir = tempdir().expect("tempdir");
        let cfg_path = dir.path().join("config.toml");

        let mut cfg = AppConfig::default_for_os();
        cfg.file_name_pattern = "   ".into();
        assert!(matches!(
            cfg.save(&cfg_path),
            Err(CoreError::InvalidFileNamePattern(_))
        ));
    }

    #[test]
    fn portable_mode_keeps_all_paths_under_base_dir_and_round_trips() {
        let dir = tempdir().expect("tempdir");
        let base_dir = dir.path().join("ShareX-Portable");

        let cfg = AppConfig::portable_for_dir(&base_dir);
        let cfg_path = AppConfig::portable_config_path(&base_dir);

        assert_eq!(cfg.output_dir, base_dir.join("Screenshots"));
        assert_eq!(cfg.history_path, base_dir.join("history.jsonl"));
        assert_eq!(cfg_path, base_dir.join("config.toml"));
        assert_eq!(cfg.file_name_pattern, DEFAULT_FILE_NAME_PATTERN);
        assert_eq!(cfg.image_format, ImageFormat::Png);
        assert!(cfg.dark_theme);

        cfg.save(&cfg_path).expect("save portable config");
        let loaded = AppConfig::load_or_default(&cfg_path).expect("load portable config");
        assert_eq!(loaded, cfg);
    }
}
