use crate::theme::{VoltShotPalette, card_frame, section_label, view_heading};
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use voltshot_core::{
    AppConfig, CoreError, FileNamePattern, HotkeyAction, ImageFormat, TaskOverrides,
    WatchFolderRule, WatchedFileEvent, WorkflowPreset, WorkflowRegistry, resolve_effective_config,
};

/// Action returned by `WorkflowsPanelState::show` when workflow or watcher state changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowsUiAction {
    None,
    RulesChanged(Vec<WatchFolderRule>),
    Saved(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WorkflowsDocument {
    #[serde(default)]
    workflows: Vec<WorkflowPreset>,
    #[serde(default)]
    watch_folders: Vec<WatchFolderRule>,
}

/// Persisted state and UI controller for per-hotkey workflow presets and watch folders.
pub struct WorkflowsPanelState {
    /// Registered per-hotkey workflow presets.
    pub registry: WorkflowRegistry,
    /// Configured watch-folder rules.
    pub watch_rules: Vec<WatchFolderRule>,
    editing_workflow_idx: Option<usize>,
    watch_extension_drafts: HashMap<String, String>,
    last_processed_event: Option<WatchedFileEvent>,
    status_message: Option<Result<String, String>>,
}

impl Default for WorkflowsPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowsPanelState {
    /// Creates an empty `WorkflowsPanelState`.
    pub fn new() -> Self {
        Self {
            registry: WorkflowRegistry::default(),
            watch_rules: Vec::new(),
            editing_workflow_idx: None,
            watch_extension_drafts: HashMap::new(),
            last_processed_event: None,
            status_message: None,
        }
    }

    /// Returns the default `workflows.toml` path sibling to an application `config.toml` path.
    pub fn default_path_for_config(config_path: &Path) -> PathBuf {
        config_path.with_file_name("workflows.toml")
    }

    /// Loads workflows and watch-folder rules from a TOML file, returning defaults when absent.
    pub fn load_or_default(path: &Path) -> Result<Self, CoreError> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let raw = fs::read_to_string(path)?;
        let doc: WorkflowsDocument =
            toml::from_str(&raw).map_err(|err| CoreError::ConfigError(err.to_string()))?;

        let mut watch_extension_drafts = HashMap::with_capacity(doc.watch_folders.len());
        for rule in &doc.watch_folders {
            watch_extension_drafts.insert(rule.id.clone(), format_extensions(&rule.extensions));
        }

        Ok(Self {
            registry: WorkflowRegistry {
                workflows: doc.workflows,
            },
            watch_rules: doc.watch_folders,
            editing_workflow_idx: None,
            watch_extension_drafts,
            last_processed_event: None,
            status_message: None,
        })
    }

    /// Persists all workflow presets and watch-folder rules to `path` as TOML.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let doc = WorkflowsDocument {
            workflows: self.registry.workflows.clone(),
            watch_folders: self.watch_rules.clone(),
        };
        let encoded =
            toml::to_string_pretty(&doc).map_err(|err| CoreError::ConfigError(err.to_string()))?;
        fs::write(path, encoded)?;
        Ok(())
    }

    /// Returns the workflow registry.
    pub fn registry(&self) -> &WorkflowRegistry {
        &self.registry
    }

    /// Returns the configured watch-folder rules.
    pub fn watch_rules(&self) -> &[WatchFolderRule] {
        &self.watch_rules
    }

    /// Records the most recently processed watch-folder file event for status badge display.
    pub fn record_processed_event(&mut self, event: WatchedFileEvent) {
        self.last_processed_event = Some(event);
    }

    /// Returns the most recently processed watch-folder file event, if any.
    pub fn last_processed_event(&self) -> Option<&WatchedFileEvent> {
        self.last_processed_event.as_ref()
    }

    /// Appends a new workflow preset and selects it for inline editing.
    pub fn add_workflow_preset(&mut self) -> usize {
        let id = next_unique_id(
            "workflow",
            self.registry.workflows.iter().map(|p| p.id.as_str()),
        );
        let index = self.registry.workflows.len();
        self.registry.workflows.push(WorkflowPreset {
            id: id.clone(),
            name: format!("Workflow {}", index + 1),
            action: HotkeyAction::CaptureRegion,
            combo: None,
            enabled: true,
            overrides: TaskOverrides::default(),
        });
        self.editing_workflow_idx = Some(index);
        index
    }

    /// Appends a new watch-folder rule with default image extensions.
    pub fn add_watch_folder(&mut self) -> usize {
        let id = next_unique_id("watch", self.watch_rules.iter().map(|r| r.id.as_str()));
        let extensions = vec!["png".into(), "jpg".into(), "webp".into()];
        self.watch_extension_drafts
            .insert(id.clone(), format_extensions(&extensions));
        let index = self.watch_rules.len();
        self.watch_rules.push(WatchFolderRule {
            id,
            folder: PathBuf::new(),
            extensions,
            move_after_process: None,
            enabled: true,
            workflow_id: None,
        });
        index
    }

    /// Renders the Workflows & Watch Folders management panel.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        workflows_path: &Path,
        active_watched_count: usize,
        watcher_active: bool,
    ) -> WorkflowsUiAction {
        let mut action = WorkflowsUiAction::None;

        ui.horizontal(|ui| {
            view_heading(ui, "Workflows & Watch Folders", p);
        });
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);

        if let Some(status) = &self.status_message {
            let (text, color) = match status {
                Ok(msg) => (msg.as_str(), p.success),
                Err(msg) => (msg.as_str(), p.danger),
            };
            ui.label(RichText::new(text).size(12.5).color(color));
            ui.add_space(4.0);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let wf_action = self.show_workflows_section(ui, p, workflows_path);
            if wf_action != WorkflowsUiAction::None {
                action = wf_action;
            }

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            let watch_action = self.show_watch_folders_section(
                ui,
                p,
                workflows_path,
                active_watched_count,
                watcher_active,
            );
            if watch_action != WorkflowsUiAction::None {
                action = watch_action;
            }
        });

        action
    }

    fn show_workflows_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        workflows_path: &Path,
    ) -> WorkflowsUiAction {
        let mut action = WorkflowsUiAction::None;

        ui.horizontal(|ui| {
            section_label(ui, "Per-Hotkey Workflow Presets", p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save Workflows").clicked() {
                    match self.save(workflows_path) {
                        Ok(()) => {
                            let msg = format!(
                                "Saved {} workflow preset(s).",
                                self.registry.workflows.len()
                            );
                            self.status_message = Some(Ok(msg.clone()));
                            action = WorkflowsUiAction::Saved(msg);
                        }
                        Err(err) => {
                            self.status_message =
                                Some(Err(format!("Failed to save workflows: {err}")));
                        }
                    }
                }
                if ui.button("+ Add Workflow Preset").clicked() {
                    self.add_workflow_preset();
                }
            });
        });
        ui.add_space(6.0);

        if self.registry.workflows.is_empty() {
            card_frame(p).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "No workflow presets yet. Click \"+ Add Workflow Preset\" to override capture settings per hotkey.",
                    )
                    .color(p.text_muted),
                );
            });
            return action;
        }

        let mut delete_idx: Option<usize> = None;
        let count = self.registry.workflows.len();

        for idx in 0..count {
            let is_editing = self.editing_workflow_idx == Some(idx);
            let preset = &mut self.registry.workflows[idx];

            egui::Frame::NONE
                .fill(p.card_bg)
                .stroke(Stroke::new(
                    if is_editing { 1.5_f32 } else { 1.0_f32 },
                    if is_editing { p.accent } else { p.border },
                ))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut preset.enabled, "");
                        ui.label(
                            RichText::new(&preset.name)
                                .strong()
                                .color(if preset.enabled {
                                    p.text_primary
                                } else {
                                    p.text_muted
                                }),
                        );
                        ui.label(
                            RichText::new(format!(
                                "[{} · hotkey: {}]",
                                preset.id,
                                hotkey_id_for_action(preset.action)
                            ))
                            .size(11.5)
                            .monospace()
                            .color(p.text_muted),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("Delete").color(p.danger)).clicked() {
                                delete_idx = Some(idx);
                            }
                            let toggle_label = if is_editing { "Done" } else { "Edit" };
                            if ui.button(toggle_label).clicked() {
                                self.editing_workflow_idx =
                                    if is_editing { None } else { Some(idx) };
                            }
                        });
                    });

                    if is_editing {
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                        show_workflow_inline_editor(ui, p, idx, preset);
                    }
                });
            ui.add_space(6.0);
        }

        if let Some(idx) = delete_idx {
            self.registry.workflows.remove(idx);
            self.editing_workflow_idx = match self.editing_workflow_idx {
                Some(editing) if editing == idx => None,
                Some(editing) if editing > idx => Some(editing - 1),
                other => other,
            };
        }

        action
    }

    fn show_watch_folders_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        workflows_path: &Path,
        active_watched_count: usize,
        watcher_active: bool,
    ) -> WorkflowsUiAction {
        let mut action = WorkflowsUiAction::None;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("WATCH FOLDERS")
                    .size(11.5)
                    .strong()
                    .color(p.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save & Apply Watchers").clicked() {
                    match self.save(workflows_path) {
                        Ok(()) => {
                            let enabled = self.watch_rules.iter().filter(|r| r.enabled).count();
                            self.status_message = Some(Ok(format!(
                                "Saved and applied {} active watch folder rule(s).",
                                enabled
                            )));
                            action = WorkflowsUiAction::RulesChanged(self.watch_rules.clone());
                        }
                        Err(err) => {
                            self.status_message =
                                Some(Err(format!("Failed to save watch folders: {err}")));
                        }
                    }
                }
                if ui.button("+ Add Watch Folder").clicked() {
                    self.add_watch_folder();
                }
            });
        });
        ui.add_space(6.0);

        show_watcher_status_badge(
            ui,
            p,
            active_watched_count,
            watcher_active,
            self.last_processed_event.as_ref(),
        );
        ui.add_space(8.0);

        if self.watch_rules.is_empty() {
            card_frame(p).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "No watch folders configured. Click \"+ Add Watch Folder\" to monitor directories automatically.",
                    )
                    .color(p.text_muted),
                );
            });
            return action;
        }

        let workflow_choices: Vec<(String, String)> = self
            .registry
            .workflows
            .iter()
            .map(|preset| (preset.id.clone(), preset.name.clone()))
            .collect();

        let mut delete_idx: Option<usize> = None;

        for (idx, rule) in self.watch_rules.iter_mut().enumerate() {
            egui::Frame::NONE
                .fill(p.card_bg)
                .stroke(Stroke::new(1.0_f32, p.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut rule.enabled, "Enabled");
                        ui.label(
                            RichText::new(&rule.id)
                                .size(11.5)
                                .monospace()
                                .color(p.text_muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("Delete").color(p.danger)).clicked() {
                                delete_idx = Some(idx);
                            }
                        });
                    });

                    ui.add_space(4.0);
                    egui::Grid::new(format!("watch_rule_grid_{idx}"))
                        .num_columns(2)
                        .spacing([10.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("Folder");
                            let mut folder_text = rule.folder.to_string_lossy().into_owned();
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut folder_text)
                                        .hint_text("Path to watched directory")
                                        .desired_width(360.0),
                                )
                                .changed()
                            {
                                rule.folder = PathBuf::from(folder_text);
                            }
                            ui.end_row();

                            ui.label("Extensions");
                            let draft = self
                                .watch_extension_drafts
                                .entry(rule.id.clone())
                                .or_insert_with(|| format_extensions(&rule.extensions));
                            if ui
                                .add(
                                    egui::TextEdit::singleline(draft)
                                        .hint_text("png, jpg, webp (blank = all files)")
                                        .desired_width(360.0),
                                )
                                .changed()
                            {
                                rule.extensions = parse_extensions(draft);
                            }
                            ui.end_row();

                            ui.label("Workflow");
                            let selected_label = rule
                                .workflow_id
                                .as_deref()
                                .map(|id| {
                                    workflow_choices
                                        .iter()
                                        .find(|(wf_id, _)| wf_id == id)
                                        .map(|(wf_id, name)| format!("{name} ({wf_id})"))
                                        .unwrap_or_else(|| id.to_owned())
                                })
                                .unwrap_or_else(|| "None (default settings)".to_owned());

                            egui::ComboBox::from_id_salt(format!("watch_workflow_{idx}"))
                                .selected_text(selected_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut rule.workflow_id,
                                        None,
                                        "None (default settings)",
                                    );
                                    for (wf_id, wf_name) in &workflow_choices {
                                        ui.selectable_value(
                                            &mut rule.workflow_id,
                                            Some(wf_id.clone()),
                                            format!("{wf_name} ({wf_id})"),
                                        );
                                    }
                                });
                            ui.end_row();
                        });
                });
            ui.add_space(6.0);
        }

        if let Some(idx) = delete_idx {
            let removed = self.watch_rules.remove(idx);
            self.watch_extension_drafts.remove(&removed.id);
        }

        action
    }
}

/// Returns the canonical hotkey identifier corresponding to a `HotkeyAction`.
pub const fn hotkey_id_for_action(action: HotkeyAction) -> &'static str {
    match action {
        HotkeyAction::CaptureFullscreen => "capture-fullscreen",
        HotkeyAction::CaptureRegion => "capture-region",
        HotkeyAction::CaptureActiveWindow => "capture-active-window",
        HotkeyAction::CaptureLastRegion => "capture-last-region",
        HotkeyAction::OpenEditor => "open-editor",
        HotkeyAction::RecordVideo => "record-video",
        HotkeyAction::RecordGif => "record-gif",
        HotkeyAction::CaptureOcr => "capture-ocr",
        HotkeyAction::PickScreenColor => "pick-screen-color",
    }
}

/// Resolves the effective `AppConfig` for `hotkey_id` by applying any matching enabled workflow preset.
pub fn resolve_config_for_hotkey(
    base: &AppConfig,
    registry: &WorkflowRegistry,
    hotkey_id: &str,
) -> AppConfig {
    workflow_for_hotkey(base, registry, hotkey_id)
        .and_then(|preset| resolve_effective_config(base, &preset.overrides).ok())
        .unwrap_or_else(|| base.clone())
}

pub(crate) fn workflow_for_hotkey<'a>(
    base: &AppConfig,
    registry: &'a WorkflowRegistry,
    hotkey_id: &str,
) -> Option<&'a WorkflowPreset> {
    registry
        .workflows
        .iter()
        .find(|preset| preset.enabled && preset.id == hotkey_id)
        .or_else(|| {
            let action = action_for_hotkey_id(base, hotkey_id)?;
            registry
                .workflows
                .iter()
                .find(|preset| preset.enabled && preset.action == action)
        })
}

fn action_for_hotkey_id(base: &AppConfig, hotkey_id: &str) -> Option<HotkeyAction> {
    if let Some(binding) = base.hotkeys.iter().find(|binding| binding.id == hotkey_id) {
        return Some(binding.action);
    }
    HotkeyAction::ALL
        .into_iter()
        .find(|&action| hotkey_id_for_action(action) == hotkey_id)
}

fn show_workflow_inline_editor(
    ui: &mut egui::Ui,
    p: &VoltShotPalette,
    idx: usize,
    preset: &mut WorkflowPreset,
) {
    egui::Grid::new(format!("workflow_editor_grid_{idx}"))
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Preset ID");
            ui.add(egui::TextEdit::singleline(&mut preset.id).desired_width(260.0));
            ui.end_row();

            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut preset.name).desired_width(260.0));
            ui.end_row();

            ui.label("Hotkey");
            egui::ComboBox::from_id_salt(format!("workflow_hotkey_{idx}"))
                .selected_text(format!(
                    "{} ({})",
                    hotkey_id_for_action(preset.action),
                    preset.action.label()
                ))
                .show_ui(ui, |ui| {
                    for action in HotkeyAction::ALL {
                        ui.selectable_value(
                            &mut preset.action,
                            action,
                            format!("{} ({})", hotkey_id_for_action(action), action.label()),
                        );
                    }
                });
            ui.end_row();

            ui.label("Filename pattern");
            let mut pattern_text = preset
                .overrides
                .file_name_pattern
                .clone()
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut pattern_text)
                        .hint_text("Inherit base pattern (e.g. Custom_%y%mo%d_%h%mi%s)")
                        .desired_width(300.0),
                )
                .changed()
            {
                preset.overrides.file_name_pattern = if pattern_text.trim().is_empty() {
                    None
                } else {
                    Some(pattern_text.clone())
                };
            }
            ui.end_row();

            if let Some(pattern) = &preset.overrides.file_name_pattern
                && let Err(err) = FileNamePattern::parse(pattern)
            {
                ui.label("");
                ui.label(
                    RichText::new(format!("Pattern warning: {err}"))
                        .size(11.5)
                        .color(p.warning),
                );
                ui.end_row();
            }

            ui.label("Image format");
            let format_label = match preset.overrides.image_format {
                None => "Inherit",
                Some(ImageFormat::Png) => "PNG",
                Some(ImageFormat::Jpeg) => "JPEG",
                Some(ImageFormat::Webp) => "WebP",
            };
            egui::ComboBox::from_id_salt(format!("workflow_format_{idx}"))
                .selected_text(format_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut preset.overrides.image_format, None, "Inherit");
                    ui.selectable_value(
                        &mut preset.overrides.image_format,
                        Some(ImageFormat::Png),
                        "PNG",
                    );
                    ui.selectable_value(
                        &mut preset.overrides.image_format,
                        Some(ImageFormat::Jpeg),
                        "JPEG",
                    );
                    ui.selectable_value(
                        &mut preset.overrides.image_format,
                        Some(ImageFormat::Webp),
                        "WebP",
                    );
                });
            ui.end_row();

            show_bool_override_row(
                ui,
                format!("workflow_save_{idx}"),
                "Save to file",
                &mut preset.overrides.save_to_file,
            );
            show_bool_override_row(
                ui,
                format!("workflow_copy_img_{idx}"),
                "Copy image to clipboard",
                &mut preset.overrides.copy_image_to_clipboard,
            );
            show_bool_override_row(
                ui,
                format!("workflow_open_editor_{idx}"),
                "Open in editor",
                &mut preset.overrides.open_in_editor,
            );

            ui.label("Uploader");
            show_uploader_override(ui, idx, &mut preset.overrides.uploader_id);
            ui.end_row();
        });
}

fn show_bool_override_row(
    ui: &mut egui::Ui,
    id_salt: String,
    label: &str,
    value: &mut Option<bool>,
) {
    ui.label(label);
    let current = match value {
        None => "Inherit",
        Some(true) => "Enabled",
        Some(false) => "Disabled",
    };
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(current)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Inherit");
            ui.selectable_value(value, Some(true), "Enabled");
            ui.selectable_value(value, Some(false), "Disabled");
        });
    ui.end_row();
}

fn show_uploader_override(ui: &mut egui::Ui, idx: usize, uploader_id: &mut Option<Option<String>>) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum UploaderMode {
        Inherit,
        Disabled,
        Custom,
    }

    let mut mode = match uploader_id {
        None => UploaderMode::Inherit,
        Some(None) => UploaderMode::Disabled,
        Some(Some(_)) => UploaderMode::Custom,
    };
    let mode_label = match mode {
        UploaderMode::Inherit => "Inherit",
        UploaderMode::Disabled => "None (disable upload)",
        UploaderMode::Custom => "Specific uploader",
    };

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(format!("workflow_uploader_mode_{idx}"))
            .selected_text(mode_label)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut mode, UploaderMode::Inherit, "Inherit")
                    .clicked()
                {
                    *uploader_id = None;
                }
                if ui
                    .selectable_value(&mut mode, UploaderMode::Disabled, "None (disable upload)")
                    .clicked()
                {
                    *uploader_id = Some(None);
                }
                if ui
                    .selectable_value(&mut mode, UploaderMode::Custom, "Specific uploader")
                    .clicked()
                {
                    *uploader_id = Some(Some("local-mirror".into()));
                }
            });

        if let Some(Some(id)) = uploader_id {
            ui.add(
                egui::TextEdit::singleline(id)
                    .hint_text("e.g. local-mirror, imgur")
                    .desired_width(160.0),
            );
        }
    });
}

fn show_watcher_status_badge(
    ui: &mut egui::Ui,
    p: &VoltShotPalette,
    active_watched_count: usize,
    watcher_active: bool,
    last_event: Option<&WatchedFileEvent>,
) {
    let badge_color = if watcher_active && active_watched_count > 0 {
        p.success
    } else {
        p.text_muted
    };
    let status_text = if watcher_active && active_watched_count > 0 {
        format!("Active · {active_watched_count} watched folder(s)")
    } else {
        format!("Idle · {active_watched_count} watched folder(s)")
    };

    egui::Frame::NONE
        .fill(p.card_bg)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::Frame::NONE
                    .fill(badge_color)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(status_text)
                                .size(11.5)
                                .strong()
                                .color(Color32::WHITE),
                        );
                    });

                ui.add_space(6.0);
                let event_summary = match last_event {
                    Some(event) => {
                        let name = event
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| event.path.display().to_string());
                        match &event.workflow_id {
                            Some(wf_id) => {
                                format!(
                                    "Last event: {name} (rule: {}, workflow: {wf_id})",
                                    event.rule_id
                                )
                            }
                            None => format!("Last event: {name} (rule: {})", event.rule_id),
                        }
                    }
                    None => "Last event: none yet".to_owned(),
                };
                ui.label(RichText::new(event_summary).size(12.0).color(p.text_muted));
            });
        });
}

fn format_extensions(extensions: &[String]) -> String {
    extensions.join(", ")
}

fn parse_extensions(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|part| part.trim().trim_start_matches('.'))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn next_unique_id<'a>(prefix: &str, existing: impl Iterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = existing.collect();
    let mut counter = 1usize;
    loop {
        let candidate = format!("{prefix}-{counter}");
        if !taken.iter().any(|item| *item == candidate) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn workflows_panel_state_round_trips_presets_and_watch_rules_through_toml() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("nested").join("workflows.toml");

        let absent = WorkflowsPanelState::load_or_default(&file_path).expect("load absent");
        assert!(absent.registry.workflows.is_empty());
        assert!(absent.watch_rules.is_empty());

        let mut state = WorkflowsPanelState::new();
        state.registry.workflows.push(WorkflowPreset {
            id: "region-webp".into(),
            name: "Region WebP Upload".into(),
            action: HotkeyAction::CaptureRegion,
            combo: None,
            enabled: true,
            overrides: TaskOverrides {
                file_name_pattern: Some("Region_%y%mo%d_%h%mi%s".into()),
                image_format: Some(ImageFormat::Webp),
                save_to_file: Some(true),
                copy_image_to_clipboard: Some(false),
                open_in_editor: Some(true),
                uploader_id: Some(Some("local-mirror".into())),
                ..TaskOverrides::default()
            },
        });
        state.watch_rules.push(WatchFolderRule {
            id: "watch-1".into(),
            folder: dir.path().join("incoming"),
            extensions: vec!["png".into(), "jpg".into()],
            move_after_process: None,
            enabled: true,
            workflow_id: Some("region-webp".into()),
        });

        state.save(&file_path).expect("save workflows");
        let loaded = WorkflowsPanelState::load_or_default(&file_path).expect("reload workflows");

        assert_eq!(loaded.registry, state.registry);
        assert_eq!(loaded.watch_rules, state.watch_rules);
    }

    #[test]
    fn resolve_config_for_hotkey_applies_bound_overrides_and_leaves_unbound_untouched() {
        let mut base = AppConfig::default_for_os();
        base.image_format = ImageFormat::Png;
        base.file_name_pattern = "Base_%y-%mo-%d".into();
        base.pipeline.save_to_file = true;
        base.pipeline.copy_image_to_clipboard = true;
        base.pipeline.open_in_editor = false;
        base.pipeline.uploader_id = Some("default-uploader".into());

        let registry = WorkflowRegistry {
            workflows: vec![
                WorkflowPreset {
                    id: "region-preset".into(),
                    name: "Region WebP".into(),
                    action: HotkeyAction::CaptureRegion,
                    combo: None,
                    enabled: true,
                    overrides: TaskOverrides {
                        file_name_pattern: Some("Region_%y%mo%d".into()),
                        image_format: Some(ImageFormat::Webp),
                        copy_image_to_clipboard: Some(false),
                        open_in_editor: Some(true),
                        uploader_id: Some(Some("imgur".into())),
                        ..TaskOverrides::default()
                    },
                },
                WorkflowPreset {
                    id: "disabled-window".into(),
                    name: "Disabled Window".into(),
                    action: HotkeyAction::CaptureActiveWindow,
                    combo: None,
                    enabled: false,
                    overrides: TaskOverrides {
                        image_format: Some(ImageFormat::Jpeg),
                        ..TaskOverrides::default()
                    },
                },
            ],
        };

        let resolved_region = resolve_config_for_hotkey(&base, &registry, "capture-region");
        assert_eq!(resolved_region.file_name_pattern, "Region_%y%mo%d");
        assert_eq!(resolved_region.image_format, ImageFormat::Webp);
        assert!(resolved_region.pipeline.save_to_file);
        assert!(!resolved_region.pipeline.copy_image_to_clipboard);
        assert!(resolved_region.pipeline.open_in_editor);
        assert_eq!(
            resolved_region.pipeline.uploader_id.as_deref(),
            Some("imgur")
        );

        let resolved_fullscreen = resolve_config_for_hotkey(&base, &registry, "capture-fullscreen");
        assert_eq!(resolved_fullscreen, base);

        let resolved_disabled =
            resolve_config_for_hotkey(&base, &registry, "capture-active-window");
        assert_eq!(resolved_disabled, base);

        let resolved_unknown = resolve_config_for_hotkey(&base, &registry, "unbound-hotkey");
        assert_eq!(resolved_unknown, base);
    }
}
