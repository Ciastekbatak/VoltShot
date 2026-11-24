use crate::domain::{CoreError, ImageFormat};
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
    pub uploader_id: Option<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            save_to_file: true,
            copy_image_to_clipboard: true,
            copy_path_to_clipboard: false,
            record_history: true,
            uploader_id: None,
        }
    }
}

/// Persisted application configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub output_dir: PathBuf,
    pub file_name_pattern: String,
    pub image_format: ImageFormat,
    pub pipeline: PipelineConfig,
    pub history_path: PathBuf,
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
        }
    }

    pub fn default_config_path() -> PathBuf {
        ProjectDirs::from("org", "ShareX", "ShareX-RS")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("sharex-data").join("config.toml"))
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

        cfg.save(&cfg_path).expect("save config");
        let loaded = AppConfig::load_or_default(&cfg_path).expect("load config");
        assert_eq!(loaded, cfg);
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
}
