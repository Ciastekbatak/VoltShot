use crate::clipboard::SystemClipboard;
use crate::editor_ui::{EditorAction, EditorViewState};
use crate::hotkeys::{HotkeyRuntime, HotkeyStatus};
use crate::region_overlay::{OverlayOutcome, RegionOverlayState};
use crate::theme::{SharexPalette, apply_sharex_theme, palette};
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_capture::XcapCapturer;
use sharex_core::{
    AppConfig, CaptureId, CapturePipeline, CaptureSource, CapturedImage, ClipboardSink,
    EmptyUploaderLookup, FileNameContext, FileNamePattern, HistoryEntry, HistoryRepository,
    HotkeyAction, HotkeyCombo, ImageFormat, JsonlHistoryStore, KeyCode, KeyModifiers, MonitorInfo,
    PipelineDeps, ScreenCapturer, default_sharex_hotkeys, resolve_unique_path,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Starts the ShareX-style native desktop application window with global hotkeys.
pub fn launch_gui(config_path: PathBuf) -> Result<(), eframe::Error> {
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
        Box::new(move |cc| Ok(Box::new(SharexGuiApp::new(cc, config_path)))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Captures,
    Editor,
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
    search_filter: String,
    history_entries: Vec<HistoryEntry>,
    selected_entry_id: Option<String>,
    thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    failed_thumbs: HashMap<PathBuf, bool>,
    editor: Option<EditorViewState>,
    overlay: Option<RegionOverlayState>,
    hotkeys: HotkeyRuntime,
    last_hotkey_retry: Instant,
    capturer: XcapCapturer,
    clipboard: SystemClipboard,
    status_line: String,
    theme_applied_dark: bool,
}

impl SharexGuiApp {
    fn new(cc: &eframe::CreationContext<'_>, config_path: PathBuf) -> Self {
        let (config, initial_status) = match AppConfig::load_or_default(&config_path) {
            Ok(cfg) => (
                cfg,
                "Ready. Press PrintScreen or Ctrl+PrintScreen to capture.".to_owned(),
            ),
            Err(err) => (
                AppConfig::default_for_os(),
                format!("Loaded default config after error: {err}"),
            ),
        };

        apply_sharex_theme(&cc.egui_ctx, config.dark_theme);
        let hotkeys = HotkeyRuntime::new(cc.egui_ctx.clone(), &config.hotkeys);
        let capturer = XcapCapturer::new();
        let monitors = capturer.list_monitors().unwrap_or_default();
        let history_entries = JsonlHistoryStore::new(&config.history_path)
            .list(Some(100))
            .unwrap_or_default();
        let output_dir_input = config.output_dir.display().to_string();

        Self {
            config_path,
            theme_applied_dark: config.dark_theme,
            config,
            output_dir_input,
            monitors,
            selected_monitor_idx: 0,
            active_view: ActiveView::Captures,
            search_filter: String::new(),
            history_entries,
            selected_entry_id: None,
            thumbnails: HashMap::new(),
            failed_thumbs: HashMap::new(),
            editor: None,
            overlay: None,
            hotkeys,
            last_hotkey_retry: Instant::now(),
            capturer,
            clipboard: SystemClipboard::new(),
            status_line: initial_status,
        }
    }

    fn persist_config_silent(&mut self) {
        if let Err(err) = self.config.save(&self.config_path) {
            self.status_line = format!("Failed to save config: {err}");
        }
    }

    fn hotkey_hint(&self, action: HotkeyAction) -> String {
        self.config
            .hotkeys
            .iter()
            .find(|b| b.action == action && b.enabled)
            .and_then(|b| b.combo)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "Unbound".into())
    }

    fn trigger_hotkey_action(&mut self, ctx: &egui::Context, action: HotkeyAction) {
        match action {
            HotkeyAction::CaptureFullscreen => {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            HotkeyAction::CaptureRegion => {
                self.start_region_overlay(ctx);
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
        }
    }

    fn start_region_overlay(&mut self, ctx: &egui::Context) {
        match self.capturer.capture_primary_snapshot() {
            Ok(snapshot) => {
                self.overlay = Some(RegionOverlayState::open(ctx, snapshot));
                self.status_line =
                    "Region capture active: drag to select, Enter for fullscreen, Esc to cancel."
                        .into();
            }
            Err(err) => {
                self.status_line = format!("Region capture failed: {err}");
            }
        }
    }

    fn capture_source(&mut self, ctx: &egui::Context, source: CaptureSource) {
        let history = JsonlHistoryStore::new(&self.config.history_path);
        let mut run_cfg = self.config.clone();
        run_cfg.pipeline.uploader_id = None;
        let uploaders = EmptyUploaderLookup;
        let deps = PipelineDeps {
            capturer: &self.capturer,
            clipboard: &self.clipboard,
            uploaders: &uploaders,
            history: &history,
        };
        match CapturePipeline::execute(source, &run_cfg, &deps, Utc::now()) {
            Ok(report) => {
                let saved_note = report
                    .saved_path
                    .as_ref()
                    .map(|p| format!(" -> {}", p.display()))
                    .unwrap_or_default();
                self.status_line = format!(
                    "Captured {}x{} ({}){}",
                    report.image.width(),
                    report.image.height(),
                    report.format,
                    saved_note
                );
                if let Some(path) = &report.saved_path {
                    self.thumbnails.remove(path);
                    self.failed_thumbs.remove(path);
                }
                self.refresh_history();
                self.selected_entry_id = Some(report.capture_id.as_str().to_owned());

                if self.config.pipeline.open_in_editor {
                    self.editor = Some(EditorViewState::open(
                        ctx,
                        report.image,
                        report.saved_path,
                        report.format,
                    ));
                    self.active_view = ActiveView::Editor;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
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
        let now = Utc::now();
        let format = self.config.image_format;
        let saved_path = if self.config.pipeline.save_to_file {
            match self.write_new_capture_file(&image, format, now) {
                Ok(path) => Some(path),
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
        if self.config.pipeline.record_history {
            let store = JsonlHistoryStore::new(&self.config.history_path);
            let _ = store.append(&HistoryEntry {
                id: capture_id.clone(),
                timestamp: now,
                source,
                width: image.width(),
                height: image.height(),
                format,
                file_path: saved_path.clone(),
                upload_url: None,
            });
        }

        self.refresh_history();
        self.selected_entry_id = Some(capture_id.as_str().to_owned());
        self.status_line = match &saved_path {
            Some(p) => format!(
                "Captured region {}x{} -> {}",
                image.width(),
                image.height(),
                p.display()
            ),
            None => format!("Captured region {}x{}.", image.width(), image.height()),
        };

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

    fn refresh_history(&mut self) {
        let store = JsonlHistoryStore::new(&self.config.history_path);
        match store.list(Some(100)) {
            Ok(entries) => self.history_entries = entries,
            Err(err) => self.status_line = format!("History load failed: {err}"),
        }
    }

    fn ensure_thumbnail(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureId> {
        if let Some(tex) = self.thumbnails.get(path) {
            return Some(tex.id());
        }
        if self.failed_thumbs.contains_key(path) || !path.exists() {
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
                self.thumbnails.insert(path.to_path_buf(), tex);
                Some(id)
            }
            Err(_) => {
                self.failed_thumbs.insert(path.to_path_buf(), true);
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
        });
        ui.add_space(6.0);
        ui.separator();

        ui.label(
            RichText::new("CAPTURE")
                .size(11.0)
                .strong()
                .color(p.text_muted),
        );
        if sidebar_action_button(
            ui,
            "Fullscreen",
            &self.hotkey_hint(HotkeyAction::CaptureFullscreen),
            p,
        ) {
            self.capture_source(ctx, CaptureSource::PrimaryMonitor);
        }
        if sidebar_action_button(
            ui,
            "Region",
            &self.hotkey_hint(HotkeyAction::CaptureRegion),
            p,
        ) {
            self.start_region_overlay(ctx);
        }
        if sidebar_action_button(
            ui,
            "Active window",
            &self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
            p,
        ) {
            self.capture_source(ctx, CaptureSource::ActiveWindow);
        }
        if sidebar_action_button(
            ui,
            "Last region",
            &self.hotkey_hint(HotkeyAction::CaptureLastRegion),
            p,
        ) {
            self.trigger_hotkey_action(ctx, HotkeyAction::CaptureLastRegion);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let selected_label = self
                .monitors
                .get(self.selected_monitor_idx)
                .map(|m| format!("{} ({}x{})", m.name, m.width, m.height))
                .unwrap_or_else(|| "No monitor".into());
            egui::ComboBox::from_id_salt("sidebar_monitor")
                .selected_text(selected_label)
                .width(142.0)
                .show_ui(ui, |ui| {
                    for (idx, m) in self.monitors.iter().enumerate() {
                        let label = format!("{} ({}x{})", m.name, m.width, m.height);
                        ui.selectable_value(&mut self.selected_monitor_idx, idx, label);
                    }
                });
            if ui.small_button("Go").clicked()
                && let Some(m) = self.monitors.get(self.selected_monitor_idx)
            {
                self.capture_source(ctx, CaptureSource::Monitor { id: m.id });
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.label(
            RichText::new("AFTER CAPTURE TASKS")
                .size(11.0)
                .strong()
                .color(p.text_muted),
        );
        let mut changed = false;
        changed |= ui
            .checkbox(&mut self.config.pipeline.save_to_file, "Save image to file")
            .changed();
        changed |= ui
            .checkbox(
                &mut self.config.pipeline.copy_image_to_clipboard,
                "Copy image to clipboard",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.config.pipeline.open_in_editor,
                "Open in image editor",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.config.pipeline.copy_path_to_clipboard,
                "Copy file path",
            )
            .changed();
        if changed {
            self.persist_config_silent();
        }

        ui.add_space(6.0);
        ui.separator();
        ui.label(
            RichText::new("WORKSPACE")
                .size(11.0)
                .strong()
                .color(p.text_muted),
        );

        if sidebar_nav_button(ui, "Captures", self.active_view == ActiveView::Captures, p) {
            self.active_view = ActiveView::Captures;
            self.refresh_history();
        }
        let editor_label = match &self.editor {
            Some(ed) if ed.is_dirty() => "Image editor *",
            Some(_) => "Image editor",
            None => "Image editor (empty)",
        };
        if sidebar_nav_button(ui, editor_label, self.active_view == ActiveView::Editor, p) {
            self.active_view = ActiveView::Editor;
        }
        if sidebar_nav_button(
            ui,
            "Hotkey settings",
            self.active_view == ActiveView::Hotkeys,
            p,
        ) {
            self.active_view = ActiveView::Hotkeys;
        }
        if sidebar_nav_button(
            ui,
            "Application settings",
            self.active_view == ActiveView::Settings,
            p,
        ) {
            self.active_view = ActiveView::Settings;
        }
    }

    fn render_captures_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        ui.horizontal(|ui| {
            ui.heading("Captures");
            ui.add_space(8.0);
            let region_btn =
                egui::Button::new(RichText::new("+ Capture Region").color(Color32::WHITE))
                    .fill(p.accent);
            if ui.add(region_btn).clicked() {
                self.start_region_overlay(ctx);
            }
            if ui.button("Fullscreen").clicked() {
                self.capture_source(ctx, CaptureSource::PrimaryMonitor);
            }
            if ui.button("Open Screenshots Folder").clicked() {
                let _ = fs::create_dir_all(&self.config.output_dir);
                open_folder_in_os(&self.config.output_dir);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    self.refresh_history();
                    self.refresh_monitors();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_filter)
                        .hint_text("Filter captures...")
                        .desired_width(180.0),
                );
            });
        });
        ui.separator();

        let query = self.search_filter.trim().to_ascii_lowercase();
        let visible: Vec<HistoryEntry> = self
            .history_entries
            .iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                let path_match = entry.file_path.as_ref().is_some_and(|path| {
                    path.to_string_lossy().to_ascii_lowercase().contains(&query)
                });
                path_match || entry.id.as_str().to_ascii_lowercase().contains(&query)
            })
            .cloned()
            .collect();

        if visible.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(
                        RichText::new("No screenshots yet")
                            .size(18.0)
                            .strong()
                            .color(p.text_primary),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "Press {} for fullscreen, {} for interactive region capture, or {} for active window.",
                            self.hotkey_hint(HotkeyAction::CaptureFullscreen),
                            self.hotkey_hint(HotkeyAction::CaptureRegion),
                            self.hotkey_hint(HotkeyAction::CaptureActiveWindow),
                        ))
                        .color(p.text_muted),
                    );
                });
            });
            return;
        }

        let mut open_editor_path: Option<PathBuf> = None;
        let mut copy_img_path: Option<PathBuf> = None;
        let mut copy_text_val: Option<String> = None;
        let mut delete_file_path: Option<PathBuf> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &visible {
                let is_selected = self
                    .selected_entry_id
                    .as_deref()
                    .is_some_and(|id| id == entry.id.as_str());
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
                                self.selected_entry_id = Some(entry.id.as_str().to_owned());
                            }
                            if img_resp.double_clicked()
                                && let Some(path) = &entry.file_path
                            {
                                open_editor_path = Some(path.clone());
                            }
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(thumb_size, egui::Sense::hover());
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(3), p.sidebar_bg);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "No preview",
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
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .size(14.0)
                                    .color(p.text_primary),
                            );
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
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if let Some(path) = &entry.file_path {
                                    let exists = path.exists();
                                    if ui.add_enabled(exists, egui::Button::new("Edit")).clicked() {
                                        open_editor_path = Some(path.clone());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new("Copy Image"))
                                        .clicked()
                                    {
                                        copy_img_path = Some(path.clone());
                                    }
                                    if ui.button("Copy Path").clicked() {
                                        copy_text_val = Some(path.display().to_string());
                                    }
                                    if ui
                                        .add_enabled(exists, egui::Button::new("Delete File"))
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
                    format!("Deleted {}", path.display())
                }
                Err(err) => format!("Delete failed: {err}"),
            };
        }
    }

    fn render_editor_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, p: &SharexPalette) {
        let Some(editor) = self.editor.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.label(
                        RichText::new("No image loaded in the editor")
                            .size(18.0)
                            .strong(),
                    );
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
                        let store = JsonlHistoryStore::new(&self.config.history_path);
                        let _ = store.append(&HistoryEntry {
                            id: CaptureId::from_timestamp(now, image.width()),
                            timestamp: now,
                            source: CaptureSource::PrimaryMonitor,
                            width: image.width(),
                            height: image.height(),
                            format,
                            file_path: Some(path.clone()),
                            upload_url: None,
                        });
                        if let Some(ed) = self.editor.as_mut() {
                            ed.set_saved(path.clone());
                        }
                        self.status_line = format!("Saved edited copy to {}", path.display());
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

    fn render_hotkeys_view(&mut self, ui: &mut egui::Ui, p: &SharexPalette) {
        ui.horizontal(|ui| {
            ui.heading("Hotkey settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let apply_btn =
                    egui::Button::new(RichText::new("Apply & Save").color(Color32::WHITE))
                        .fill(p.accent);
                if ui.add(apply_btn).clicked() {
                    self.hotkeys.sync(&self.config.hotkeys);
                    self.persist_config_silent();
                    self.status_line =
                        format!("Applied hotkeys ({} active).", self.hotkeys.active_count());
                }
                if ui.button("Reset ShareX Defaults").clicked() {
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
                                None => "Unbound",
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
                        if ui.small_button("Clear").clicked() {
                            binding.combo = None;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (badge_text, badge_color) = match status {
                                HotkeyStatus::Active => ("Active".into(), p.success),
                                HotkeyStatus::Disabled => ("Disabled".into(), p.text_muted),
                                HotkeyStatus::Unbound => ("Unbound".into(), p.warning),
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
        ui.heading("Application settings");
        ui.separator();

        egui::Grid::new("sharex_settings_grid")
            .num_columns(2)
            .spacing([14.0, 10.0])
            .show(ui, |ui| {
                ui.label("Screenshots folder");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_dir_input).desired_width(380.0),
                    );
                    if ui.button("Open").clicked() {
                        let target = PathBuf::from(self.output_dir_input.trim());
                        let _ = fs::create_dir_all(&target);
                        open_folder_in_os(&target);
                    }
                });
                ui.end_row();

                ui.label("Filename pattern");
                ui.add(
                    egui::TextEdit::singleline(&mut self.config.file_name_pattern)
                        .desired_width(380.0),
                );
                ui.end_row();

                ui.label("Pattern preview");
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

                ui.label("Image format");
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

                ui.label("Theme");
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.config.dark_theme, "ShareX Dark")
                        .clicked()
                    {
                        self.config.dark_theme = true;
                    }
                    if ui
                        .selectable_label(!self.config.dark_theme, "Light")
                        .clicked()
                    {
                        self.config.dark_theme = false;
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
            egui::Button::new(RichText::new("Save Settings").color(Color32::WHITE)).fill(p.accent);
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

        if self.last_hotkey_retry.elapsed() >= Duration::from_secs(2) {
            self.hotkeys.retry_conflicts(&self.config.hotkeys);
            self.last_hotkey_retry = Instant::now();
        }

        let pending_actions = self.hotkeys.poll_actions();
        for action in pending_actions {
            self.trigger_hotkey_action(ctx, action);
        }

        if let Some(overlay) = self.overlay.as_mut() {
            match overlay.show(ctx) {
                OverlayOutcome::Pending => {}
                OverlayOutcome::Cancelled => {
                    self.overlay = None;
                    self.status_line = "Region capture cancelled.".into();
                }
                OverlayOutcome::Captured {
                    image,
                    global_region,
                    open_in_editor,
                } => {
                    self.overlay = None;
                    self.config.last_region = Some(global_region);
                    self.persist_config_silent();
                    self.commit_captured_image(
                        ctx,
                        image,
                        CaptureSource::Region {
                            region: global_region,
                        },
                        open_in_editor,
                    );
                }
            }
        }

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
                    ui.label(
                        RichText::new(&self.status_line)
                            .size(12.0)
                            .color(p.text_muted),
                    );
                });
            });

        egui::SidePanel::left("sharex_sidebar")
            .resizable(false)
            .exact_width(220.0)
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
                ActiveView::Hotkeys => self.render_hotkeys_view(ui, &p),
                ActiveView::Settings => self.render_settings_view(ctx, ui, &p),
            });
    }
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
