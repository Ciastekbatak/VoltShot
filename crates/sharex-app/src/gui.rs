use crate::clipboard::SystemClipboard;
use crate::editor_ui::{EditorAction, EditorViewState};
use crate::effects_ui::EffectsPanelState;
use crate::hotkeys::{HotkeyRuntime, HotkeyStatus, TriggeredHotkey};
use crate::i18n::{self, UiLanguage};
use crate::pin_window::PinManager;
use crate::recording_ui::RecordingPanelState;
use crate::region_overlay::{OverlayOutcome, RegionOverlayState};
use crate::theme::{SharexPalette, apply_sharex_theme, palette};
use crate::thumb_cache::LruCache;
use crate::tools_ui::{ToolsContext, ToolsPanelState};
use crate::tray::{TrayAction, TrayRuntime};
use crate::uploaders_ui::UploadersPanelState;
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_capture::{DesktopSnapshot, XcapCapturer};
use sharex_core::{
    AppConfig, CaptureId, CaptureSource, CapturedImage, ClipboardSink, FileNameContext,
    FileNamePattern, HistoryEntry, HistoryQuery, HistoryRepository, HotkeyAction, HotkeyCombo,
    ImageFormat, JsonlHistoryStore, KeyCode, KeyModifiers, MonitorInfo, ScreenCapturer,
    SqliteHistoryStore, default_sharex_hotkeys, resolve_unique_path,
};
use sharex_media::RecordingFormat;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const THUMB_CACHE_CAPACITY: usize = 64;
const HISTORY_LIMIT: usize = 100;

/// Starts the ShareX-style native desktop application window with global hotkeys.
pub fn launch_gui(config_path: PathBuf) -> Result<(), eframe::Error> {
    launch_gui_with_fallback(config_path, AppConfig::default_for_os())
}

/// Starts the ShareX-style native desktop window with an explicit fallback config (e.g. portable mode).
pub fn launch_gui_with_fallback(
    config_path: PathBuf,
    fallback_config: AppConfig,
) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ShareX (Rust)")
            .with_inner_size([1160.0, 720.0])
            .with_min_inner_size([860.0, 540.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ShareX (Rust)",
        options,
        Box::new(move |cc| {
            Ok(Box::new(SharexGuiApp::new(
                cc,
                config_path,
                fallback_config,
            )))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Captures,
    Editor,
    Effects,
    Tools,
    Recording,
    Uploaders,
    Hotkeys,
    Settings,
}

struct SharexGuiApp {
    config_path: PathBuf,
    config: AppConfig,
    output_dir_input: String,
    monitors: Vec<MonitorInfo>,
    selected_monitor_idx: usize,
    active_view: ActiveView,
    language: UiLanguage,
    search_filter: String,
    sqlite_history: Option<SqliteHistoryStore>,
    jsonl_history: JsonlHistoryStore,
    history_entries: Vec<HistoryEntry>,
    search_results: Option<Vec<HistoryEntry>>,
    older_capture_path: Option<PathBuf>,
    selected_capture: Option<HistoryEntry>,
    tag_input: String,
    thumbnails: LruCache<PathBuf, egui::TextureHandle>,
    failed_thumbs: HashMap<PathBuf, bool>,
    editor: Option<EditorViewState>,
    effects: EffectsPanelState,
    tools: ToolsPanelState,
    recording: RecordingPanelState,
    uploaders: UploadersPanelState,
    overlay: Option<RegionOverlayState>,
    pins: PinManager,
    tray: TrayRuntime,
    hotkeys: HotkeyRuntime,
    last_hotkey_retry: Instant,
    capturer: XcapCapturer,
    clipboard: SystemClipboard,
    status_line: String,
    theme_applied_dark: bool,
}

impl SharexGuiApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        config_path: PathBuf,
        fallback_config: AppConfig,
    ) -> Self {
        let language = UiLanguage::default();
        let ready_text = i18n::strings(language).ready_status.to_owned();
        let (mut config, mut initial_status) = if config_path.exists() {
            match AppConfig::load_or_default(&config_path) {
                Ok(cfg) => (cfg, ready_text),
                Err(err) => (
                    fallback_config,
                    format!("Loaded fallback config after error: {err}"),
                ),
            }
        } else {
            (fallback_config, ready_text)
        };

        if !config.hotkeys.is_empty() {
            for binding in default_sharex_hotkeys().into_iter().filter(|binding| {
                matches!(
                    binding.action,
                    HotkeyAction::RecordVideo | HotkeyAction::RecordGif
                )
            }) {
                if !config
                    .hotkeys
                    .iter()
                    .any(|saved| saved.action == binding.action || saved.id == binding.id)
                {
                    config.hotkeys.push(binding);
                }
            }
        }

        apply_sharex_theme(&cc.egui_ctx, config.dark_theme);
        let hotkeys = HotkeyRuntime::new(cc.egui_ctx.clone(), &config.hotkeys);
        let tray = TrayRuntime::new(cc.egui_ctx.clone());
        let capturer = XcapCapturer::new();
        let monitors = capturer.list_monitors().unwrap_or_default();
        let sqlite_path = config.history_path.with_extension("sqlite");
        let sqlite_history = match SqliteHistoryStore::open(&sqlite_path) {
            Ok(store) => {
                if sqlite_path != config.history_path
                    && let Err(err) = store.migrate_from_jsonl_once(&config.history_path)
                {
                    initial_status.push_str(&format!(" History import failed: {err}"));
                }
                Some(store)
            }
            Err(err) => {
                initial_status
                    .push_str(&format!(" Using JSONL history. SQLite open failed: {err}"));
                None
            }
        };
        let jsonl_history = JsonlHistoryStore::new(&config.history_path);
        let uploaders = UploadersPanelState::new(&crate::default_mirror_dir(&config));
        let output_dir_input = config.output_dir.display().to_string();
        let cache_cap = NonZeroUsize::new(THUMB_CACHE_CAPACITY).expect("non-zero cache capacity");

        let mut app = Self {
            config_path,
            theme_applied_dark: config.dark_theme,
            config,
            output_dir_input,
            monitors,
            selected_monitor_idx: 0,
            active_view: ActiveView::Captures,
            language,
            search_filter: String::new(),
            sqlite_history,
            jsonl_history,
            history_entries: Vec::new(),
            search_results: None,
            older_capture_path: None,
            selected_capture: None,
            tag_input: String::new(),
            thumbnails: LruCache::new(cache_cap),
            failed_thumbs: HashMap::new(),
            editor: None,
            effects: EffectsPanelState::new(),
            tools: ToolsPanelState::new(),
            recording: RecordingPanelState::new(),
            uploaders,
            overlay: None,
            pins: PinManager::new(),
            tray,
            hotkeys,
            last_hotkey_retry: Instant::now(),
            capturer,
            clipboard: SystemClipboard::new(),
            status_line: initial_status,
        };
        app.refresh_history();
        app
    }

    fn persist_config_silent(&mut self) {
        if let Err(err) = self.config.save(&self.config_path) {
            self.status_line = format!("Failed to save config: {err}");
        }
    }

    fn latest_capture_path(&self) -> Option<&Path> {
        first_existing_capture_path(&self.history_entries).or_else(|| {
            self.older_capture_path
                .as_deref()
                .filter(|path| path.is_file())
        })
    }

    fn select_capture(&mut self, entry: &HistoryEntry) {
        if !self.is_capture_selected(entry) {
            self.tag_input.clear();
        }
        self.selected_capture = Some(entry.clone());
    }

    fn is_capture_selected(&self, entry: &HistoryEntry) -> bool {
        self.selected_capture
            .as_ref()
            .is_some_and(|selected| selected.id == entry.id)
    }

    fn hotkey_hint(&self, action: HotkeyAction) -> String {
        self.config
            .hotkeys
            .iter()
            .find(|b| b.action == action && b.enabled)
            .and_then(|b| b.combo)
            .map(|c| c.to_string())
            .unwrap_or_else(|| i18n::strings(self.language).status_unbound.to_owned())
    }

    fn handle_tray_action(&mut self, ctx: &egui::Context, action: TrayAction) {
        match action {
            TrayAction::CaptureFullscreen => {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            TrayAction::CaptureRegion => {
                self.start_region_overlay(ctx);
            }
            TrayAction::CaptureActiveWindow => {
                self.capture_source(ctx, CaptureSource::ActiveWindow);
            }
            TrayAction::OpenMainWindow => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            TrayAction::OpenScreenshotsFolder => {
                let _ = fs::create_dir_all(&self.config.output_dir);
                open_folder_in_os(&self.config.output_dir);
            }
            TrayAction::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn trigger_hotkey_event(&mut self, ctx: &egui::Context, event: TriggeredHotkey) {
        match event.action {
            HotkeyAction::CaptureFullscreen => {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            HotkeyAction::CaptureRegion => {
                if let Some(snapshot) = event.pre_snapshot {
                    self.open_region_overlay_with_snapshot(ctx, snapshot);
                } else {
                    self.start_region_overlay(ctx);
                }
            }
            HotkeyAction::CaptureActiveWindow => {
                self.capture_source(ctx, CaptureSource::ActiveWindow);
            }
            HotkeyAction::CaptureLastRegion => {
                if let Some(region) = self.config.last_region {
                    self.capture_source(ctx, CaptureSource::Region { region });
                } else {
                    self.status_line =
                        "No last region saved yet. Starting region selection.".into();
                    self.start_region_overlay(ctx);
                }
            }
            HotkeyAction::OpenEditor => {
                if self.editor.is_some() {
                    self.active_view = ActiveView::Editor;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else if let Some(first_path) = self
                    .history_entries
                    .iter()
                    .find_map(|e| e.file_path.clone())
                {
                    self.open_file_in_editor(ctx, &first_path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else {
                    self.status_line =
                        "No screenshot available to open in the image editor.".into();
                }
            }
            HotkeyAction::RecordVideo | HotkeyAction::RecordGif => {
                let format = if event.action == HotkeyAction::RecordVideo {
                    RecordingFormat::Mp4H264
                } else {
                    RecordingFormat::Gif
                };
                self.status_line = self
                    .recording
                    .toggle_hotkey_recording(format, &self.config.output_dir);
                self.active_view = ActiveView::Recording;
            }
        }
    }

    fn open_region_overlay_with_snapshot(
        &mut self,
        ctx: &egui::Context,
        snapshot: DesktopSnapshot,
    ) {
        self.overlay = Some(RegionOverlayState::open(ctx, snapshot));
        self.status_line =
            "Region capture active: drag to select, annotate from top bar, Esc to cancel.".into();
    }

    fn start_region_overlay(&mut self, ctx: &egui::Context) {
        match self.capturer.capture_primary_snapshot() {
            Ok(snapshot) => {
                self.open_region_overlay_with_snapshot(ctx, snapshot);
            }
            Err(err) => {
                self.status_line = format!("Region capture failed: {err}");
            }
        }
    }

    fn capture_source(&mut self, ctx: &egui::Context, source: CaptureSource) {
        match self.capturer.capture(&source) {
            Ok(image) => {
                self.commit_captured_image(ctx, image, source, false);
            }
            Err(err) => {
                self.status_line = format!("Capture failed: {err}");
            }
        }
    }

    fn commit_captured_image(
        &mut self,
        ctx: &egui::Context,
        image: CapturedImage,
        source: CaptureSource,
        force_open_in_editor: bool,
    ) {
        let mut effect_note = String::new();
        let image = match self.effects.apply_if_enabled(&image) {
            Ok(processed) => {
                if self.effects.apply_on_capture && self.effects.active_preset().is_some() {
                    effect_note = format!(" [{}]", self.effects.active_preset_name());
                }
                processed
            }
            Err(err) => {
                effect_note = format!(" [effect skipped: {err}]");
                image
            }
        };

        let now = Utc::now();
        let format = self.config.image_format;
        let saved_path = if self.config.pipeline.save_to_file {
            match self.write_new_capture_file(&image, format, now) {
                Ok(path) => {
                    self.thumbnails.remove(&path);
                    self.failed_thumbs.remove(&path);
                    Some(path)
                }
                Err(err) => {
                    self.status_line = format!("Failed to save capture: {err}");
                    return;
                }
            }
        } else {
            None
        };

        if self.config.pipeline.copy_image_to_clipboard
            && let Err(err) = self.clipboard.copy_image(&image)
        {
            self.status_line = format!("Saved capture, but clipboard copy failed: {err}");
        }

        if self.config.pipeline.copy_path_to_clipboard
            && let Some(path) = &saved_path
        {
            let _ = self.clipboard.copy_text(&path.display().to_string());
        }

        let capture_id = CaptureId::from_timestamp(now, image.width() ^ image.height());
        let history_result = if self.config.pipeline.record_history {
            let entry = HistoryEntry {
                id: capture_id,
                timestamp: now,
                source,
                width: image.width(),
                height: image.height(),
                format,
                file_path: saved_path.clone(),
                upload_url: None,
                tags: Vec::new(),
                note: None,
            };
            self.history_repository()
                .append(&entry)
                .map(|()| self.select_capture(&entry))
        } else {
            Ok(())
        };

        self.status_line = match &saved_path {
            Some(p) => format!(
                "Captured {}x{} ({}){} -> {}",
                image.width(),
                image.height(),
                format,
                effect_note,
                p.display()
            ),
            None => format!(
                "Captured {}x{} ({}){}.",
                image.width(),
                image.height(),
                format,
                effect_note
            ),
        };
        if let Err(err) = history_result {
            self.status_line
                .push_str(&format!(" History update failed: {err}"));
        }
        self.refresh_history();

        if force_open_in_editor || self.config.pipeline.open_in_editor {
            self.editor = Some(EditorViewState::open(ctx, image, saved_path, format));
            self.active_view = ActiveView::Editor;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn write_new_capture_file(
        &self,
        image: &CapturedImage,
        format: ImageFormat,
        now: chrono::DateTime<Utc>,
    ) -> Result<PathBuf, String> {
        let pattern =
            FileNamePattern::parse(&self.config.file_name_pattern).map_err(|e| e.to_string())?;
        let stem = pattern.expand(&FileNameContext {
            timestamp: now,
            app_name: "ShareX",
            width: image.width(),
            height: image.height(),
        });
        fs::create_dir_all(&self.config.output_dir).map_err(|e| e.to_string())?;
        let path = resolve_unique_path(&self.config.output_dir, &stem, format.extension());
        let bytes = image.encode(format).map_err(|e| e.to_string())?;
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(path)
    }

    fn open_file_in_editor(&mut self, ctx: &egui::Context, path: &Path) {
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let format = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(|e| e.parse::<ImageFormat>().ok())
                    .unwrap_or(self.config.image_format);
                self.editor = Some(EditorViewState::open(
                    ctx,
                    img,
                    Some(path.to_path_buf()),
                    format,
                ));
                self.active_view = ActiveView::Editor;
                self.status_line = format!("Opened {} in image editor.", path.display());
            }
            Err(err) => {
                self.status_line = format!("Could not open {}: {err}", path.display());
            }
        }
    }

    fn pin_file_to_screen(&mut self, ctx: &egui::Context, path: &Path) {
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Pinned Capture".to_owned());
                let id = self.pins.pin_image(ctx, title.clone(), &img);
                self.status_line = format!("Pinned {title} (#{id}) to screen.");
            }
            Err(err) => {
                self.status_line = format!("Failed to pin {}: {err}", path.display());
            }
        }
    }

    fn refresh_monitors(&mut self) {
        match self.capturer.list_monitors() {
            Ok(list) => {
                self.monitors = list;
                if self.selected_monitor_idx >= self.monitors.len() {
                    self.selected_monitor_idx = 0;
                }
            }
            Err(err) => self.status_line = format!("Monitor refresh failed: {err}"),
        }
    }

    fn history_repository(&self) -> &dyn HistoryRepository {
        match &self.sqlite_history {
            Some(store) => store,
            None => &self.jsonl_history,
        }
    }

    fn refresh_history(&mut self) {
        match self.history_repository().list(Some(HISTORY_LIMIT)) {
            Ok(entries) => self.history_entries = entries,
            Err(err) => self.status_line = format!("History load failed: {err}"),
        }
        self.older_capture_path = None;
        if self.history_entries.len() == HISTORY_LIMIT && self.latest_capture_path().is_none() {
            match self.history_repository().list(None) {
                Ok(entries) => {
                    self.older_capture_path =
                        first_existing_capture_path(&entries).map(Path::to_path_buf);
                }
                Err(err) => self.status_line = format!("History load failed: {err}"),
            }
        }
        self.refresh_history_search();
    }

    fn refresh_history_search(&mut self) {
        let query = capture_history_query(&self.search_filter);
        self.search_results = if query.text.is_none() && query.tag.is_none() {
            None
        } else {
            Some(match self.history_repository().search(&query) {
                Ok(entries) => entries,
                Err(err) => {
                    self.status_line = format!("History search failed: {err}");
                    Vec::new()
                }
            })
        };
    }

    fn save_capture_tags(&mut self, id: &CaptureId, tags: &[String]) -> bool {
        match self.history_repository().set_tags(id, tags) {
            Ok(true) => {
                self.status_line = "Capture tags updated.".into();
                self.refresh_history();
                true
            }
            Ok(false) => {
                self.status_line = "This capture is no longer in history.".into();
                self.refresh_history();
                false
            }
            Err(err) => {
                self.status_line = format!("Could not update capture tags: {err}");
                false
            }
        }
    }

    fn ensure_thumbnail(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureId> {
        let key = path.to_path_buf();
        if let Some(tex) = self.thumbnails.get(&key) {
            return Some(tex.id());
        }
        if self.failed_thumbs.contains_key(&key) || !path.exists() {
            return None;
        }
        match CapturedImage::load_from_file(path) {
            Ok(img) => {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.rgba(),
                );
                let tex = ctx.load_texture(
                    format!("thumb_{}", path.display()),
                    color,
                    egui::TextureOptions::LINEAR,
                );
                let id = tex.id();
                self.thumbnails.insert(key, tex);
                Some(id)
            }
            Err(_) => {
                self.failed_thumbs.insert(key, true);
                None
            }
        }
    }

    fn render_sidebar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("ShareX")
                    .size(20.0)
                    .strong()
                    .color(p.text_primary),
            );
            ui.label(RichText::new("Rust").size(12.0).color(p.accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .selectable_label(self.language == UiLanguage::Pl, "PL")
                    .on_hover_text(UiLanguage::Pl.display_name())
                    .clicked()
                {
                    self.language = UiLanguage::Pl;
                }
                if ui
                    .selectable_label(self.language == UiLanguage::En, "EN")
                    .on_hover_text(UiLanguage::En.display_name())
                    .clicked()
                {
                    self.language = UiLanguage::En;
                }
            });
        });
        ui.add_space(6.0);
        ui.separator();

        let s = i18n::strings(self.language);

        egui::ScrollArea::vertical()
            .id_salt("sharex_sidebar_scroll")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(s.capture_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                if sidebar_action_button(
                    ui,
                    s.fullscreen,
                    &self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::PrimaryMonitor);
                }
                if sidebar_action_button(
                    ui,
                    s.region,
                    &self.hotkey_hint(HotkeyAction::CaptureRegion),
                    p,
                ) {
                    self.start_region_overlay(ctx);
                }
                if sidebar_action_button(
                    ui,
                    s.active_window,
                    &self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                    p,
                ) {
                    self.capture_source(ctx, CaptureSource::ActiveWindow);
                }
                if sidebar_action_button(
                    ui,
                    s.last_region,
                    &self.hotkey_hint(HotkeyAction::CaptureLastRegion),
                    p,
                ) {
                    self.trigger_hotkey_event(
                        ctx,
                        TriggeredHotkey {
                            action: HotkeyAction::CaptureLastRegion,
                            pre_snapshot: None,
                        },
                    );
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let selected_label = self
                        .monitors
                        .get(self.selected_monitor_idx)
                        .map(|m| format!("{} ({}x{})", m.name, m.width, m.height))
                        .unwrap_or_else(|| s.no_monitor.to_owned());
                    egui::ComboBox::from_id_salt("sidebar_monitor")
                        .selected_text(selected_label)
                        .width(142.0)
                        .show_ui(ui, |ui| {
                            for (idx, m) in self.monitors.iter().enumerate() {
                                let label = format!("{} ({}x{})", m.name, m.width, m.height);
                                ui.selectable_value(&mut self.selected_monitor_idx, idx, label);
                            }
                        });
                    if ui.small_button(s.capture_monitor_go).clicked()
                        && let Some(m) = self.monitors.get(self.selected_monitor_idx)
                    {
                        self.capture_source(ctx, CaptureSource::Monitor { id: m.id });
                    }
                });

                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    RichText::new(s.after_capture_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );
                let mut changed = false;
                changed |= ui
                    .checkbox(&mut self.config.pipeline.save_to_file, s.save_image_to_file)
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.copy_image_to_clipboard,
                        s.copy_image_to_clipboard,
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.open_in_editor,
                        s.open_in_image_editor,
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.config.pipeline.copy_path_to_clipboard,
                        s.copy_file_path,
                    )
                    .changed();
                let effects_toggle_label =
                    format!("{}: {}", s.effects, self.effects.active_preset_name());
                ui.checkbox(&mut self.effects.apply_on_capture, effects_toggle_label);
                if changed {
                    self.persist_config_silent();
                }

                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    RichText::new(s.workspace_section)
                        .size(11.0)
                        .strong()
                        .color(p.text_muted),
                );

                if sidebar_nav_button(ui, s.captures, self.active_view == ActiveView::Captures, p) {
                    self.active_view = ActiveView::Captures;
                    self.refresh_history();
                }
                let editor_label = match &self.editor {
                    Some(ed) if ed.is_dirty() => s.image_editor_dirty,
                    Some(_) => s.image_editor,
                    None => s.image_editor_empty,
                };
                if sidebar_nav_button(ui, editor_label, self.active_view == ActiveView::Editor, p) {
                    self.active_view = ActiveView::Editor;
                }
                if sidebar_nav_button(ui, s.effects, self.active_view == ActiveView::Effects, p) {
                    self.active_view = ActiveView::Effects;
                }
                if sidebar_nav_button(ui, s.tools, self.active_view == ActiveView::Tools, p) {
                    self.active_view = ActiveView::Tools;
                }
                let recording_label = if self.recording.is_recording() {
                    format!("{} (REC)", s.recording)
                } else {
                    s.recording.to_owned()
                };
                if sidebar_nav_button(
                    ui,
                    &recording_label,
                    self.active_view == ActiveView::Recording,
                    p,
                ) {
                    self.active_view = ActiveView::Recording;
                }
                if sidebar_nav_button(
                    ui,
                    s.uploaders,
                    self.active_view == ActiveView::Uploaders,
                    p,
                ) {
                    self.active_view = ActiveView::Uploaders;
                    self.refresh_history();
                }
                if sidebar_nav_button(
                    ui,
                    s.hotkey_settings,
                    self.active_view == ActiveView::Hotkeys,
                    p,
                ) {
                    self.active_view = ActiveView::Hotkeys;
                }
                if sidebar_nav_button(
                    ui,
                    s.application_settings,
                    self.active_view == ActiveView::Settings,
                    p,
                ) {
                    self.active_view = ActiveView::Settings;
                }
            });
    }

    fn render_captures_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.horizontal(|ui| {
            ui.heading(s.captures);
            ui.add_space(8.0);
            let region_btn =
                egui::Button::new(RichText::new(s.capture_region_cta).color(Color32::WHITE))
                    .fill(p.accent);
            if ui.add(region_btn).clicked() {
                self.start_region_overlay(ctx);
            }
            if ui.button(s.fullscreen).clicked() {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            if ui.button(s.open_screenshots_folder).clicked() {
                let _ = fs::create_dir_all(&self.config.output_dir);
                open_folder_in_os(&self.config.output_dir);
            }
            if !self.pins.is_empty()
                && ui
                    .button(format!("{} ({})", s.pin_close_all, self.pins.count()))
                    .clicked()
            {
                self.pins.close_all();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(s.refresh).clicked() {
                    self.refresh_history();
                    self.refresh_monitors();
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search_filter)
                            .hint_text(s.filter_captures_hint)
                            .desired_width(180.0_f32),
                    )
                    .on_hover_text("Use #tag or #tag followed by filter text.")
                    .changed()
                {
                    self.refresh_history_search();
                }
            });
        });
        ui.separator();

        let visible = self
            .search_results
            .as_ref()
            .unwrap_or(&self.history_entries)
            .clone();

        if visible.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    if self.search_results.is_some() {
                        ui.label(
                            RichText::new("No captures match this filter.").color(p.text_muted),
                        );
                        return;
                    }
                    ui.label(
                        RichText::new(s.no_screenshots_yet)
                            .size(18.0)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{}: {} | {}: {} | {}: {}",
                            s.fullscreen,
                            self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                            s.region,
                            self.hotkey_hint(HotkeyAction::CaptureRegion),
                            s.active_window,
                            self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                        ))
                        .color(p.text_muted),
                    );
                });
            });
            return;
        }

        let mut open_editor_path: Option<PathBuf> = None;
        let mut pin_file_path: Option<PathBuf> = None;
        let mut copy_img_path: Option<PathBuf> = None;
        let mut copy_text_val: Option<String> = None;
        let mut delete_file_path: Option<PathBuf> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &visible {
                let is_selected = self.is_capture_selected(entry);
                let frame = egui::Frame::NONE
                    .fill(p.card_bg)
                    .stroke(Stroke::new(
                        if is_selected { 1.5_f32 } else { 1.0_f32 },
                        if is_selected { p.accent } else { p.border },
                    ))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::same(10));

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let thumb_size = Vec2::new(128.0, 76.0);
                        let thumb_tex = entry
                            .file_path
                            .as_deref()
                            .and_then(|path| self.ensure_thumbnail(ctx, path));
                        if let Some(tex_id) = thumb_tex {
                            let img_resp = ui.add(
                                egui::Image::new((tex_id, thumb_size))
                                    .corner_radius(CornerRadius::same(3))
                                    .sense(egui::Sense::click()),
                            );
                            if img_resp.clicked() {
                                self.select_capture(entry);
                            }
                            if img_resp.double_clicked()
                                && let Some(path) = &entry.file_path
                            {
                                open_editor_path = Some(path.clone());
                            }
                        } else {
                            let (rect, preview) =
                                ui.allocate_exact_size(thumb_size, egui::Sense::click());
                            if preview.clicked() {
                                self.select_capture(entry);
                            }
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(3), p.sidebar_bg);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                s.no_preview,
                                egui::FontId::proportional(11.0),
                                p.text_muted,
                            );
                        }

                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let title = entry
                                .file_path
                                .as_ref()
                                .and_then(|fp| fp.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| entry.id.as_str().to_owned());
                            let title = egui::Label::new(
                                RichText::new(title)
                                    .strong()
                                    .size(14.0_f32)
                                    .color(p.text_primary),
                            )
                            .sense(egui::Sense::click());
                            if ui.add(title).on_hover_text("Select capture").clicked() {
                                self.select_capture(entry);
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{}x{} {} | {}",
                                    entry.width,
                                    entry.height,
                                    entry.format.extension().to_ascii_uppercase(),
                                    entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                                ))
                                .size(12.0)
                                .color(p.text_muted),
                            );
                            if let Some(path) = &entry.file_path {
                                ui.label(
                                    RichText::new(path.display().to_string())
                                        .size(11.0)
                                        .color(p.text_muted),
                                );
                            }
                            self.render_capture_tags(ui, p, entry);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if let Some(path) = &entry.file_path {
                                    let exists = path.exists();
                                    if ui.add_enabled(exists, egui::Button::new(s.edit)).clicked() {
                                        open_editor_path = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new("Pin"))
                                        .on_hover_text(s.pin_to_screen)
                                        .clicked()
                                    {
                                        pin_file_path = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new(s.copy_image))
                                        .clicked()
                                    {
                                        copy_img_path = Some(path.clone());
                                    }
                                    if ui.button(s.copy_path).clicked() {
                                        copy_text_val = Some(path.display().to_string());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new(s.delete_file))
                                        .clicked()
                                    {
                                        delete_file_path = Some(path.clone());
                                    }
                                }
                            });
                        });
                    });
                });
                ui.add_space(6.0);
            }
        });

        if let Some(path) = open_editor_path {
            self.open_file_in_editor(ctx, &path);
        }
        if let Some(path) = pin_file_path {
            self.pin_file_to_screen(ctx, &path);
        }
        if let Some(path) = copy_img_path {
            self.status_line = match CapturedImage::load_from_file(&path)
                .and_then(|img| self.clipboard.copy_image(&img))
            {
                Ok(()) => format!("Copied image {} to clipboard.", path.display()),
                Err(err) => format!("Copy image failed: {err}"),
            };
        }
        if let Some(text) = copy_text_val {
            self.status_line = match self.clipboard.copy_text(&text) {
                Ok(()) => format!("Copied path: {text}"),
                Err(err) => format!("Clipboard error: {err}"),
            };
        }
        if let Some(path) = delete_file_path {
            self.status_line = match fs::remove_file(&path) {
                Ok(()) => {
                    self.thumbnails.remove(&path);
                    self.failed_thumbs.remove(&path);
                    self.refresh_history();
                    format!("Deleted {}", path.display())
                }
                Err(err) => format!("Delete failed: {err}"),
            };
        }
    }

    fn render_capture_tags(&mut self, ui: &mut egui::Ui, p: &SharexPalette, entry: &HistoryEntry) {
        if !entry.tags.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for tag in &entry.tags {
                    egui::Frame::NONE
                        .fill(p.sidebar_bg)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(5, 1))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("#{tag}")).small().color(p.text_muted),
                                );
                                if ui
                                    .add(egui::Button::new("×").small().frame(false))
                                    .on_hover_text(format!("Remove #{tag}"))
                                    .clicked()
                                {
                                    let tags = entry
                                        .tags
                                        .iter()
                                        .filter(|existing| *existing != tag)
                                        .cloned()
                                        .collect::<Vec<_>>();
                                    self.save_capture_tags(&entry.id, &tags);
                                }
                            });
                        });
                }
            });
        }

        if self.is_capture_selected(entry) {
            ui.horizontal(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.tag_input)
                        .id_salt(("capture_tag", entry.id.as_str()))
                        .hint_text("Add tag")
                        .desired_width(100.0_f32),
                );
                let tag = self.tag_input.trim().trim_start_matches('#').trim();
                let can_add = !tag.is_empty() && !entry.tags.iter().any(|existing| existing == tag);
                let submitted = ui
                    .add_enabled(can_add, egui::Button::new("+").small())
                    .on_hover_text("Add tag")
                    .clicked()
                    || (input.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                if submitted && can_add {
                    let mut tags = entry.tags.clone();
                    tags.push(tag.to_owned());
                    if self.save_capture_tags(&entry.id, &tags) {
                        self.tag_input.clear();
                    }
                }
            });
        }
    }

    fn render_editor_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let Some(editor) = self.editor.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(RichText::new(s.no_image_in_editor).size(18.0).strong());
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Double-click any screenshot in Captures, or enable 'Open in image editor' before taking a screenshot.")
                            .color(p.text_muted),
                    );
                });
            });
            return;
        };

        let action = editor.show(ctx, ui, p);
        match action {
            EditorAction::None => {}
            EditorAction::CopyImage(img) => {
                self.status_line = match self.clipboard.copy_image(&img) {
                    Ok(()) => "Copied edited image to clipboard.".into(),
                    Err(err) => format!("Clipboard error: {err}"),
                };
            }
            EditorAction::Save {
                image,
                target_path,
                format,
            } => {
                let save_result = match target_path {
                    Some(path) => image
                        .encode(format)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| fs::write(&path, bytes).map_err(|e| e.to_string()))
                        .map(|()| path),
                    None => self.write_new_capture_file(&image, format, Utc::now()),
                };
                match save_result {
                    Ok(path) => {
                        self.thumbnails.remove(&path);
                        self.failed_thumbs.remove(&path);
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_saved(path.clone());
                        }
                        self.status_line = format!("Saved edited screenshot to {}", path.display());
                        self.refresh_history();
                    }
                    Err(err) => self.status_line = format!("Save failed: {err}"),
                }
            }
            EditorAction::SaveAsCopy { image, format } => {
                match self.write_new_capture_file(&image, format, Utc::now()) {
                    Ok(path) => {
                        let now = Utc::now();
                        let history_result = self.history_repository().append(&HistoryEntry {
                            id: CaptureId::from_timestamp(now, image.width()),
                            timestamp: now,
                            source: CaptureSource::PrimaryMonitor,
                            width: image.width(),
                            height: image.height(),
                            format,
                            file_path: Some(path.clone()),
                            upload_url: None,
                            tags: Vec::new(),
                            note: None,
                        });
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_saved(path.clone());
                        }
                        self.status_line = format!("Saved edited copy to {}", path.display());
                        if let Err(err) = history_result {
                            self.status_line
                                .push_str(&format!(" History update failed: {err}"));
                        }
                        self.refresh_history();
                    }
                    Err(err) => self.status_line = format!("Save copy failed: {err}"),
                }
            }
            EditorAction::Close => {
                self.editor = None;
                self.active_view = ActiveView::Captures;
            }
        }
    }

    fn render_effects_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let latest = self.latest_capture_path().map(Path::to_path_buf);
        if let Some(msg) = self.effects.show(ctx, ui, p, s, latest.as_deref()) {
            self.status_line = msg;
        }
    }

    fn render_tools_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let latest = self.latest_capture_path().map(Path::to_path_buf);
        let output_dir = self.config.output_dir.clone();
        if let Some(msg) = self.tools.show(
            ctx,
            ui,
            p,
            s,
            ToolsContext {
                output_dir: &output_dir,
                latest_capture: latest.as_deref(),
                clipboard: &self.clipboard,
            },
        ) {
            self.status_line = msg;
        }
    }

    fn render_recording_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);
        let output_dir = self.config.output_dir.clone();
        if let Some(msg) = self
            .recording
            .show(ctx, ui, p, s, &output_dir, &self.clipboard)
        {
            self.status_line = msg;
        }
    }

    fn render_uploaders_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        let selected = self
            .selected_capture
            .as_ref()
            .and_then(|entry| entry.file_path.as_deref())
            .filter(|path| path.is_file());
        let capture = selected.map(|path| (path.to_path_buf(), true)).or_else(|| {
            self.latest_capture_path()
                .map(|path| (path.to_path_buf(), false))
        });
        self.uploaders.show(
            ui,
            p,
            i18n::strings(self.language),
            capture
                .as_ref()
                .map(|(path, selected)| (path.as_path(), *selected)),
        );
    }

    fn render_hotkeys_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.horizontal(|ui| {
            ui.heading(s.hotkey_settings);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply_btn =
                    egui::Button::new(RichText::new(s.apply_and_save).color(Color32::WHITE))
                        .fill(p.accent);
                if ui.add(apply_btn).clicked() {
                    self.hotkeys.sync(&self.config.hotkeys);
                    self.persist_config_silent();
                    self.status_line =
                        format!("Applied hotkeys ({} active).", self.hotkeys.active_count());
                }
                if ui.button(s.reset_defaults).clicked() {
                    self.config.hotkeys = default_sharex_hotkeys();
                    self.hotkeys.sync(&self.config.hotkeys);
                    self.persist_config_silent();
                    self.status_line = "Restored default ShareX keybinds.".into();
                }
            });
        });
        ui.label(
            RichText::new("Global hotkeys work even while ShareX is minimized or behind other applications. If Windows Snipping Tool owns PrintScreen, bind another key or disable 'Use the Print screen key to open screen capture' in Windows Accessibility settings.")
                .size(12.0)
                .color(p.text_muted),
        );
        ui.separator();

        for binding in &mut self.config.hotkeys {
            let status = self.hotkeys.status_for(&binding.id);
            egui::Frame::NONE
                .fill(p.card_bg)
                .stroke(Stroke::new(1.0_f32, p.border))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut binding.enabled, "");
                        ui.add_sized(
                            [170.0, 22.0],
                            egui::Label::new(
                                RichText::new(binding.action.label()).strong().size(13.0),
                            ),
                        );

                        let mut combo = binding.combo.unwrap_or_else(|| {
                            HotkeyCombo::new(KeyModifiers::NONE, KeyCode::PrintScreen)
                        });
                        let mut edited = false;

                        if ui.selectable_label(combo.modifiers.ctrl, "Ctrl").clicked() {
                            combo.modifiers.ctrl = !combo.modifiers.ctrl;
                            edited = true;
                        }
                        if ui.selectable_label(combo.modifiers.alt, "Alt").clicked() {
                            combo.modifiers.alt = !combo.modifiers.alt;
                            edited = true;
                        }
                        if ui
                            .selectable_label(combo.modifiers.shift, "Shift")
                            .clicked()
                        {
                            combo.modifiers.shift = !combo.modifiers.shift;
                            edited = true;
                        }

                        egui::ComboBox::from_id_salt(format!("hotkey_key_{}", binding.id))
                            .selected_text(match binding.combo {
                                Some(c) => c.key.display_name(),
                                None => s.status_unbound,
                            })
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for key in KeyCode::ALL {
                                    if ui
                                        .selectable_label(
                                            binding.combo.is_some_and(|c| c.key == key),
                                            key.display_name(),
                                        )
                                        .clicked()
                                    {
                                        combo.key = key;
                                        edited = true;
                                    }
                                }
                            });

                        if edited {
                            binding.combo = Some(combo);
                        }
                        if ui.small_button(s.clear).clicked() {
                            binding.combo = None;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (badge_text, badge_color) = match status {
                                HotkeyStatus::Active => (s.status_active.to_owned(), p.success),
                                HotkeyStatus::Disabled => {
                                    (s.status_disabled.to_owned(), p.text_muted)
                                }
                                HotkeyStatus::Unbound => (s.status_unbound.to_owned(), p.warning),
                                HotkeyStatus::Error(msg) => (format!("Conflict: {msg}"), p.danger),
                            };
                            ui.label(RichText::new(badge_text).size(12.0).color(badge_color));
                        });
                    });
                });
            ui.add_space(4.0);
        }
    }

    fn render_settings_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let s = i18n::strings(self.language);

        ui.heading(s.application_settings);
        ui.separator();

        egui::Grid::new("sharex_settings_grid")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.label(s.screenshots_folder);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_dir_input).desired_width(380.0),
                    );
                    if ui.button(s.open).clicked() {
                        let target = PathBuf::from(self.output_dir_input.trim());
                        let _ = fs::create_dir_all(&target);
                        open_folder_in_os(&target);
                    }
                });
                ui.end_row();

                ui.label(s.filename_pattern);
                ui.add(
                    egui::TextEdit::singleline(&mut self.config.file_name_pattern)
                        .desired_width(380.0),
                );
                ui.end_row();

                ui.label(s.pattern_preview);
                let preview = match FileNamePattern::parse(&self.config.file_name_pattern) {
                    Ok(pattern) => format!(
                        "{}.{}",
                        pattern.expand(&FileNameContext {
                            timestamp: Utc::now(),
                            app_name: "ShareX",
                            width: 1920,
                            height: 1080,
                        }),
                        self.config.image_format.extension()
                    ),
                    Err(err) => format!("Invalid pattern: {err}"),
                };
                ui.label(RichText::new(preview).monospace().color(p.text_muted));
                ui.end_row();

                ui.label(s.image_format);
                egui::ComboBox::from_id_salt("settings_format_picker")
                    .selected_text(self.config.image_format.extension().to_ascii_uppercase())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.image_format, ImageFormat::Png, "PNG");
                        ui.selectable_value(
                            &mut self.config.image_format,
                            ImageFormat::Jpeg,
                            "JPEG",
                        );
                        ui.selectable_value(
                            &mut self.config.image_format,
                            ImageFormat::Webp,
                            "WEBP",
                        );
                    });
                ui.end_row();

                ui.label(s.theme);
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.config.dark_theme, s.theme_dark)
                        .clicked()
                    {
                        self.config.dark_theme = true;
                    }
                    if ui
                        .selectable_label(!self.config.dark_theme, s.theme_light)
                        .clicked()
                    {
                        self.config.dark_theme = false;
                    }
                });
                ui.end_row();

                ui.label(s.language);
                ui.horizontal(|ui| {
                    for lang in UiLanguage::ALL {
                        if ui
                            .selectable_label(self.language == lang, lang.display_name())
                            .clicked()
                        {
                            self.language = lang;
                        }
                    }
                });
                ui.end_row();
            });

        if self.config.dark_theme != self.theme_applied_dark {
            apply_sharex_theme(ctx, self.config.dark_theme);
            self.theme_applied_dark = self.config.dark_theme;
        }

        ui.add_space(14.0);
        let save_btn =
            egui::Button::new(RichText::new(s.save_settings).color(Color32::WHITE)).fill(p.accent);
        if ui.add(save_btn).clicked() {
            self.config.output_dir = PathBuf::from(self.output_dir_input.trim());
            self.status_line = match self.config.save(&self.config_path) {
                Ok(()) => format!("Saved settings to {}", self.config_path.display()),
                Err(err) => format!("Failed to save settings: {err}"),
            };
        }
    }
}

impl eframe::App for SharexGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(80));
        self.uploaders.poll_upload(&self.clipboard);

        if self.last_hotkey_retry.elapsed() >= Duration::from_secs(2) {
            self.hotkeys.retry_conflicts(&self.config.hotkeys);
            self.last_hotkey_retry = Instant::now();
        }

        for action in self.tray.poll_actions() {
            self.handle_tray_action(ctx, action);
        }

        let pending_actions = self.hotkeys.poll_actions();
        for event in pending_actions {
            self.trigger_hotkey_event(ctx, event);
        }

        if let Some(overlay) = self.overlay.as_mut() {
            match overlay.show(ctx) {
                OverlayOutcome::Pending => {}
                OverlayOutcome::Cancelled => {
                    if let Some(closing) = self.overlay.take() {
                        closing.restore_previous_focus();
                    }
                    self.status_line = "Region capture cancelled.".into();
                }
                OverlayOutcome::Captured {
                    image,
                    global_region,
                    open_in_editor,
                } => {
                    let closing = self.overlay.take();
                    self.config.last_region = Some(global_region);
                    self.persist_config_silent();
                    let will_open_editor = open_in_editor || self.config.pipeline.open_in_editor;
                    self.commit_captured_image(
                        ctx,
                        image,
                        CaptureSource::Region {
                            region: global_region,
                        },
                        open_in_editor,
                    );
                    if !will_open_editor && let Some(prev) = closing {
                        prev.restore_previous_focus();
                    }
                }
            }
        }

        if let Some(msg) = self.recording.poll_finalize() {
            self.status_line = msg;
        }
        if self.recording.is_finalizing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        self.pins.show_all(ctx);

        let p = palette(self.config.dark_theme);

        egui::TopBottomPanel::bottom("sharex_status_bar")
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::symmetric(10, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Hotkeys: {}/{} active",
                            self.hotkeys.active_count(),
                            self.config.hotkeys.len()
                        ))
                        .size(12.0)
                        .color(if self.hotkeys.active_count() > 0 {
                            p.success
                        } else {
                            p.warning
                        }),
                    );
                    ui.separator();
                    let tray_label = if self.tray.is_available() {
                        "Tray: on"
                    } else {
                        "Tray: off"
                    };
                    ui.label(RichText::new(tray_label).size(12.0).color(
                        if self.tray.is_available() {
                            p.success
                        } else {
                            p.text_muted
                        },
                    ));
                    if !self.pins.is_empty() {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("Pins: {}", self.pins.count()))
                                .size(12.0)
                                .color(p.accent),
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(&self.status_line)
                            .size(12.0)
                            .color(p.text_muted),
                    );
                });
            });

        egui::SidePanel::left("sharex_sidebar")
            .resizable(false)
            .exact_width(228.0)
            .frame(
                egui::Frame::NONE
                    .fill(p.sidebar_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                self.render_sidebar(ctx, ui, &p);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(p.panel_bg)
                    .inner_margin(Margin::same(12)),
            )
            .show(ctx, |ui| match self.active_view {
                ActiveView::Captures => self.render_captures_view(ctx, ui, &p),
                ActiveView::Editor => self.render_editor_view(ctx, ui, &p),
                ActiveView::Effects => self.render_effects_view(ctx, ui, &p),
                ActiveView::Tools => self.render_tools_view(ctx, ui, &p),
                ActiveView::Recording => self.render_recording_view(ctx, ui, &p),
                ActiveView::Uploaders => self.render_uploaders_view(ui, &p),
                ActiveView::Hotkeys => self.render_hotkeys_view(ui, &p),
                ActiveView::Settings => self.render_settings_view(ctx, ui, &p),
            });
    }
}

fn first_existing_capture_path(entries: &[HistoryEntry]) -> Option<&Path> {
    entries
        .iter()
        .find_map(|entry| entry.file_path.as_deref().filter(|path| path.is_file()))
}

fn capture_history_query(filter: &str) -> HistoryQuery {
    let filter = filter.trim();
    let mut query = HistoryQuery {
        limit: Some(HISTORY_LIMIT),
        ..HistoryQuery::default()
    };
    if let Some(tag_filter) = filter.strip_prefix('#') {
        let mut parts = tag_filter.splitn(2, char::is_whitespace);
        if let Some(tag) = parts.next().filter(|tag| !tag.is_empty()) {
            query.tag = Some(tag.to_owned());
            query.text = parts
                .next()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            return query;
        }
    }
    query.text = (!filter.is_empty()).then(|| filter.to_owned());
    query
}

fn sidebar_action_button(ui: &mut egui::Ui, label: &str, hotkey: &str, p: &SharexPalette) -> bool {
    let desired = Vec2::new(ui.available_width(), 28.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if resp.hovered() {
        p.hover_bg
    } else {
        p.card_bg
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, if resp.hovered() { p.accent } else { p.border }),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        p.text_primary,
    );
    ui.painter().text(
        egui::pos2(rect.max.x - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        hotkey,
        egui::FontId::monospace(10.5),
        p.text_muted,
    );
    resp.clicked()
}

fn sidebar_nav_button(ui: &mut egui::Ui, label: &str, active: bool, p: &SharexPalette) -> bool {
    let desired = Vec2::new(ui.available_width(), 28.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if active {
        p.accent
    } else if resp.hovered() {
        p.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().text(
        egui::pos2(rect.min.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if active {
            Color32::WHITE
        } else {
            p.text_primary
        },
    );
    resp.clicked()
}

fn open_folder_in_os(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_history_filter_supports_plain_text_and_tag_prefix() {
        for (filter, text, tag) in [
            ("   ", None, None),
            (" Step #2 ", Some("Step #2"), None),
            (" #Bug ", None, Some("Bug")),
            (" #Bug   Login screen ", Some("Login screen"), Some("Bug")),
        ] {
            let query = capture_history_query(filter);
            assert_eq!(query.text.as_deref(), text);
            assert_eq!(query.tag.as_deref(), tag);
            assert_eq!(query.limit, Some(HISTORY_LIMIT));
        }
    }
}
