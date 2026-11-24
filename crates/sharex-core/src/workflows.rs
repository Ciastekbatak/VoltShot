use crate::config::AppConfig;
use crate::domain::{CoreError, ImageFormat};
use crate::hotkeys::{HotkeyAction, HotkeyCombo};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn default_enabled() -> bool {
    true
}

mod serde_uploader_override {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<Option<String>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            None => serializer.serialize_none(),
            Some(None) => serializer.serialize_str(""),
            Some(Some(id)) => serializer.serialize_str(id),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Option::<String>::deserialize(deserializer)?;
        match raw {
            None => Ok(Some(None)),
            Some(id) if id.is_empty() => Ok(Some(None)),
            Some(id) => Ok(Some(Some(id))),
        }
    }
}

/// Optional per-workflow overrides layered over the base `AppConfig`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_format: Option<ImageFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to_file: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_image_to_clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_path_to_clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_in_editor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_history: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_uploader_override"
    )]
    pub uploader_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_preset_name: Option<String>,
}

/// Named hotkey workflow preset bundling a trigger action with task overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPreset {
    pub id: String,
    pub name: String,
    pub action: HotkeyAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo: Option<HotkeyCombo>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub overrides: TaskOverrides,
}

/// Applies `overrides` onto a cloned copy of `base` and validates the resulting `AppConfig`.
pub fn resolve_effective_config(
    base: &AppConfig,
    overrides: &TaskOverrides,
) -> Result<AppConfig, CoreError> {
    let mut effective = base.clone();

    if let Some(output_dir) = &overrides.output_dir {
        effective.output_dir = output_dir.clone();
    }
    if let Some(file_name_pattern) = &overrides.file_name_pattern {
        effective.file_name_pattern = file_name_pattern.clone();
    }
    if let Some(image_format) = overrides.image_format {
        effective.image_format = image_format;
    }
    if let Some(save_to_file) = overrides.save_to_file {
        effective.pipeline.save_to_file = save_to_file;
    }
    if let Some(copy_image_to_clipboard) = overrides.copy_image_to_clipboard {
        effective.pipeline.copy_image_to_clipboard = copy_image_to_clipboard;
    }
    if let Some(copy_path_to_clipboard) = overrides.copy_path_to_clipboard {
        effective.pipeline.copy_path_to_clipboard = copy_path_to_clipboard;
    }
    if let Some(open_in_editor) = overrides.open_in_editor {
        effective.pipeline.open_in_editor = open_in_editor;
    }
    if let Some(record_history) = overrides.record_history {
        effective.pipeline.record_history = record_history;
    }
    if let Some(uploader_id) = &overrides.uploader_id {
        effective.pipeline.uploader_id = uploader_id.clone();
    }

    effective.validate()?;
    Ok(effective)
}

/// Collection of persisted hotkey workflow presets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRegistry {
    #[serde(default)]
    pub workflows: Vec<WorkflowPreset>,
}

impl WorkflowRegistry {
    pub fn from_toml(raw: &str) -> Result<Self, CoreError> {
        toml::from_str(raw).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    pub fn to_toml(&self) -> Result<String, CoreError> {
        toml::to_string_pretty(self).map_err(|err| CoreError::ConfigError(err.to_string()))
    }

    pub fn find_by_id(&self, id: &str) -> Option<&WorkflowPreset> {
        self.workflows.iter().find(|workflow| workflow.id == id)
    }

    pub fn resolve_for_workflow(
        &self,
        id: &str,
        base_config: &AppConfig,
    ) -> Result<AppConfig, CoreError> {
        let workflow = self
            .find_by_id(id)
            .ok_or_else(|| CoreError::ConfigError(format!("unknown workflow `{id}`")))?;
        resolve_effective_config(base_config, &workflow.overrides)
    }
}

/// Configuration for a watched directory rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchFolderRule {
    pub id: String,
    pub folder: PathBuf,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_after_process: Option<PathBuf>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
}

/// File creation or modification event emitted by `WatchFolderScanner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedFileEvent {
    pub rule_id: String,
    pub path: PathBuf,
    pub workflow_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    len: u64,
    modified: Option<SystemTime>,
}

/// Stateful, deterministic folder scanner that detects newly added or modified matching files.
#[derive(Debug, Clone, Default)]
pub struct WatchFolderScanner {
    seen: HashMap<(String, PathBuf), FileSignature>,
}

impl WatchFolderScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.seen.clear();
    }

    pub fn scan(&mut self, rules: &[WatchFolderRule]) -> Result<Vec<WatchedFileEvent>, CoreError> {
        let mut events = Vec::new();

        for rule in rules {
            if !rule.enabled {
                continue;
            }

            let mut entries = Vec::new();
            for entry in fs::read_dir(&rule.folder)? {
                entries.push(entry?);
            }
            entries.sort_by_key(|entry| entry.path());

            let mut present_paths = HashSet::new();

            for entry in entries {
                let metadata = entry.metadata()?;
                if !metadata.is_file() {
                    continue;
                }

                let path = entry.path();
                if !matches_extensions(&path, &rule.extensions) {
                    continue;
                }

                present_paths.insert(path.clone());
                let signature = FileSignature {
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                };
                let key = (rule.id.clone(), path.clone());

                if self.seen.get(&key) != Some(&signature) {
                    self.seen.insert(key, signature);
                    events.push(WatchedFileEvent {
                        rule_id: rule.id.clone(),
                        path,
                        workflow_id: rule.workflow_id.clone(),
                    });
                }
            }

            self.seen
                .retain(|(rule_id, path), _| rule_id != &rule.id || present_paths.contains(path));
        }

        Ok(events)
    }
}

fn matches_extensions(path: &Path, extensions: &[String]) -> bool {
    if extensions.is_empty() {
        return true;
    }

    let file_ext = path.extension().and_then(|ext| ext.to_str());

    extensions.iter().any(|candidate| {
        let normalized = candidate.trim().trim_start_matches('.');
        if normalized == "*" {
            return true;
        }
        if normalized.is_empty() {
            return false;
        }
        file_ext.is_some_and(|ext| ext.eq_ignore_ascii_case(normalized))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::{KeyCode, KeyModifiers};
    use tempfile::tempdir;

    #[test]
    fn resolve_effective_config_overrides_selected_fields_and_preserves_base() {
        let mut base = AppConfig::default_for_os();
        base.output_dir = PathBuf::from("/base/screenshots");
        base.file_name_pattern = "Base_%y-%mo-%d".into();
        base.image_format = ImageFormat::Png;
        base.pipeline.save_to_file = true;
        base.pipeline.copy_image_to_clipboard = true;
        base.pipeline.copy_path_to_clipboard = false;
        base.pipeline.open_in_editor = false;
        base.pipeline.record_history = true;
        base.pipeline.uploader_id = Some("base-uploader".into());
        base.dark_theme = false;

        let base_snapshot = base.clone();

        let overrides = TaskOverrides {
            output_dir: Some(PathBuf::from("/custom/captures")),
            image_format: Some(ImageFormat::Webp),
            copy_path_to_clipboard: Some(true),
            open_in_editor: Some(true),
            uploader_id: Some(None),
            effect_preset_name: Some("drop-shadow".into()),
            ..TaskOverrides::default()
        };

        let resolved = resolve_effective_config(&base, &overrides).expect("valid overrides");

        assert_eq!(base, base_snapshot);
        assert_eq!(resolved.output_dir, PathBuf::from("/custom/captures"));
        assert_eq!(resolved.file_name_pattern, "Base_%y-%mo-%d");
        assert_eq!(resolved.image_format, ImageFormat::Webp);
        assert!(resolved.pipeline.save_to_file);
        assert!(resolved.pipeline.copy_image_to_clipboard);
        assert!(resolved.pipeline.copy_path_to_clipboard);
        assert!(resolved.pipeline.open_in_editor);
        assert!(resolved.pipeline.record_history);
        assert_eq!(resolved.pipeline.uploader_id, None);
        assert_eq!(resolved.history_path, base.history_path);
        assert!(!resolved.dark_theme);
    }

    #[test]
    fn resolve_effective_config_rejects_invalid_filename_pattern() {
        let base = AppConfig::default_for_os();
        let overrides = TaskOverrides {
            file_name_pattern: Some("   ".into()),
            ..TaskOverrides::default()
        };

        assert!(matches!(
            resolve_effective_config(&base, &overrides),
            Err(CoreError::InvalidFileNamePattern(_))
        ));
    }

    #[test]
    fn workflow_registry_round_trips_through_toml_and_resolves_by_id() {
        let registry = WorkflowRegistry {
            workflows: vec![
                WorkflowPreset {
                    id: "quick-upload".into(),
                    name: "Quick WebP Upload".into(),
                    action: HotkeyAction::CaptureRegion,
                    combo: Some(HotkeyCombo::new(KeyModifiers::ctrl(), KeyCode::KeyU)),
                    enabled: true,
                    overrides: TaskOverrides {
                        output_dir: Some(PathBuf::from("/tmp/uploads")),
                        file_name_pattern: Some("Quick_%y%mo%d_%h%mi%s".into()),
                        image_format: Some(ImageFormat::Webp),
                        save_to_file: Some(true),
                        copy_image_to_clipboard: Some(false),
                        copy_path_to_clipboard: Some(true),
                        open_in_editor: Some(false),
                        record_history: Some(true),
                        uploader_id: Some(Some("imgur".into())),
                        effect_preset_name: Some("rounded-border".into()),
                    },
                },
                WorkflowPreset {
                    id: "local-only".into(),
                    name: "Local Only Capture".into(),
                    action: HotkeyAction::CaptureActiveWindow,
                    combo: None,
                    enabled: false,
                    overrides: TaskOverrides {
                        uploader_id: Some(None),
                        open_in_editor: Some(true),
                        ..TaskOverrides::default()
                    },
                },
                WorkflowPreset {
                    id: "inherit-all".into(),
                    name: "Inherit Defaults".into(),
                    action: HotkeyAction::CaptureFullscreen,
                    combo: None,
                    enabled: true,
                    overrides: TaskOverrides::default(),
                },
            ],
        };

        let encoded = registry.to_toml().expect("serialize workflows");
        let decoded = WorkflowRegistry::from_toml(&encoded).expect("deserialize workflows");
        assert_eq!(decoded, registry);

        let found = decoded.find_by_id("quick-upload").expect("workflow exists");
        assert_eq!(found.name, "Quick WebP Upload");
        assert!(decoded.find_by_id("missing").is_none());

        let mut base = AppConfig::default_for_os();
        base.pipeline.uploader_id = Some("default-s3".into());

        let quick_cfg = decoded
            .resolve_for_workflow("quick-upload", &base)
            .expect("resolve quick-upload");
        assert_eq!(quick_cfg.image_format, ImageFormat::Webp);
        assert_eq!(quick_cfg.pipeline.uploader_id.as_deref(), Some("imgur"));

        let local_cfg = decoded
            .resolve_for_workflow("local-only", &base)
            .expect("resolve local-only");
        assert_eq!(local_cfg.pipeline.uploader_id, None);
        assert!(local_cfg.pipeline.open_in_editor);

        let inherit_cfg = decoded
            .resolve_for_workflow("inherit-all", &base)
            .expect("resolve inherit-all");
        assert_eq!(
            inherit_cfg.pipeline.uploader_id.as_deref(),
            Some("default-s3")
        );

        assert!(matches!(
            decoded.resolve_for_workflow("does-not-exist", &base),
            Err(CoreError::ConfigError(_))
        ));
    }

    #[test]
    fn watch_folder_scanner_detects_new_and_modified_matching_files_idempotently() {
        let active_dir = tempdir().expect("active tempdir");
        let disabled_dir = tempdir().expect("disabled tempdir");

        let png_path = active_dir.path().join("alpha.PNG");
        let jpg_path = active_dir.path().join("beta.jpg");
        let txt_path = active_dir.path().join("notes.txt");
        let nested_dir = active_dir.path().join("subdir.png");
        let disabled_png = disabled_dir.path().join("ignored.png");

        fs::write(&png_path, b"png-v1").expect("write png");
        fs::write(&jpg_path, b"jpg-v1").expect("write jpg");
        fs::write(&txt_path, b"plain-text").expect("write txt");
        fs::create_dir(&nested_dir).expect("create subdir");
        fs::write(&disabled_png, b"disabled").expect("write disabled png");

        let rules = vec![
            WatchFolderRule {
                id: "screenshots".into(),
                folder: active_dir.path().to_path_buf(),
                extensions: vec![".png".into(), "JPG".into()],
                move_after_process: None,
                enabled: true,
                workflow_id: Some("quick-upload".into()),
            },
            WatchFolderRule {
                id: "paused".into(),
                folder: disabled_dir.path().to_path_buf(),
                extensions: vec!["png".into()],
                move_after_process: None,
                enabled: false,
                workflow_id: None,
            },
        ];

        let mut scanner = WatchFolderScanner::new();

        let first_scan = scanner.scan(&rules).expect("first scan");
        assert_eq!(
            first_scan,
            vec![
                WatchedFileEvent {
                    rule_id: "screenshots".into(),
                    path: png_path.clone(),
                    workflow_id: Some("quick-upload".into()),
                },
                WatchedFileEvent {
                    rule_id: "screenshots".into(),
                    path: jpg_path.clone(),
                    workflow_id: Some("quick-upload".into()),
                },
            ]
        );

        let second_scan = scanner.scan(&rules).expect("second scan");
        assert!(second_scan.is_empty());

        fs::write(&png_path, b"png-v2-longer-content").expect("modify png size");

        let third_scan = scanner.scan(&rules).expect("third scan");
        assert_eq!(
            third_scan,
            vec![WatchedFileEvent {
                rule_id: "screenshots".into(),
                path: png_path,
                workflow_id: Some("quick-upload".into()),
            }]
        );

        let fourth_scan = scanner.scan(&rules).expect("fourth scan");
        assert!(fourth_scan.is_empty());
    }
}
