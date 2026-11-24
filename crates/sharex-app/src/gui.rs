use crate::clipboard::SystemClipboard;
use crate::default_mirror_dir;
use chrono::Utc;
use eframe::egui;
use sharex_capture::XcapCapturer;
use sharex_core::{
    AppConfig, CapturePipeline, CaptureSource, CapturedImage, ClipboardSink, FileNameContext,
    FileNamePattern, HistoryEntry, HistoryRepository, ImageFormat, JsonlHistoryStore, MonitorInfo,
    PipelineDeps, Region, ScreenCapturer, TaskReport,
};
use sharex_uploaders::UploaderRegistry;
use std::path::PathBuf;

/// Starts the native desktop application window.
pub fn launch_gui(config_path: PathBuf) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ShareX in Rust")
            .with_inner_size([1040.0, 680.0])
            .with_min_inner_size([760.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ShareX in Rust",
        options,
        Box::new(move |_cc| Ok(Box::new(SharexGuiApp::new(config_path)))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Preview,
    History,
    Settings,
}

#[derive(Debug, Clone)]
struct RegionInputs {
    x: String,
    y: String,
    width: String,
    height: String,
}

impl Default for RegionInputs {
    fn default() -> Self {
        Self {
            x: "0".into(),
            y: "0".into(),
            width: "800".into(),
            height: "600".into(),
        }
    }
}

struct LastCaptureView {
    report: TaskReport,
    texture: egui::TextureHandle,
}

struct SharexGuiApp {
    config_path: PathBuf,
    config: AppConfig,
    output_dir_input: String,
    monitors: Vec<MonitorInfo>,
    selected_monitor_idx: usize,
    region_inputs: RegionInputs,
    active_tab: ActiveTab,
    history_entries: Vec<HistoryEntry>,
    last_capture: Option<LastCaptureView>,
    status_line: String,
    capturer: XcapCapturer,
    clipboard: SystemClipboard,
    uploaders: UploaderRegistry,
}

impl SharexGuiApp {
    fn new(config_path: PathBuf) -> Self {
        let (config, initial_status) = match AppConfig::load_or_default(&config_path) {
            Ok(cfg) => (cfg, "Ready to capture.".to_owned()),
            Err(err) => (
                AppConfig::default_for_os(),
                format!("Loaded defaults after config error: {err}"),
            ),
        };

        let capturer = XcapCapturer::new();
        let monitors = capturer.list_monitors().unwrap_or_default();
        let history_entries = JsonlHistoryStore::new(&config.history_path)
            .list(Some(50))
            .unwrap_or_default();
        let uploaders = UploaderRegistry::with_local_mirror(default_mirror_dir(&config));
        let output_dir_input = config.output_dir.display().to_string();

        Self {
            config_path,
            config,
            output_dir_input,
            monitors,
            selected_monitor_idx: 0,
            region_inputs: RegionInputs::default(),
            active_tab: ActiveTab::Preview,
            history_entries,
            last_capture: None,
            status_line: initial_status,
            capturer,
            clipboard: SystemClipboard::new(),
            uploaders,
        }
    }

    fn run_capture(&mut self, ctx: &egui::Context, source: CaptureSource) {
        let history = JsonlHistoryStore::new(&self.config.history_path);
        let deps = PipelineDeps {
            capturer: &self.capturer,
            clipboard: &self.clipboard,
            uploaders: &self.uploaders,
            history: &history,
        };
        match CapturePipeline::execute(source, &self.config, &deps, Utc::now()) {
            Ok(report) => {
                let texture =
                    create_preview_texture(ctx, &report.image, report.capture_id.as_str());
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
                self.last_capture = Some(LastCaptureView { report, texture });
                self.active_tab = ActiveTab::Preview;
                self.refresh_history();
            }
            Err(err) => {
                self.status_line = format!("Capture failed: {err}");
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
                self.status_line = format!("Detected {} monitor(s).", self.monitors.len());
            }
            Err(err) => {
                self.status_line = format!("Monitor refresh failed: {err}");
            }
        }
    }

    fn refresh_history(&mut self) {
        let store = JsonlHistoryStore::new(&self.config.history_path);
        match store.list(Some(50)) {
            Ok(entries) => self.history_entries = entries,
            Err(err) => self.status_line = format!("History read failed: {err}"),
        }
    }

    fn parse_region_inputs(&self) -> Result<Region, String> {
        let spec = format!(
            "{},{},{},{}",
            self.region_inputs.x.trim(),
            self.region_inputs.y.trim(),
            self.region_inputs.width.trim(),
            self.region_inputs.height.trim()
        );
        Region::parse(&spec).map_err(|err| err.to_string())
    }

    fn render_sidebar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.heading("ShareX in Rust");
        ui.separator();

        ui.label("Capture");
        if ui.button("Fullscreen (Primary)").clicked() {
            self.run_capture(ctx, CaptureSource::PrimaryMonitor);
        }

        ui.horizontal(|ui| {
            let selected_label = self
                .monitors
                .get(self.selected_monitor_idx)
                .map(|m| format!("{} ({}x{})", m.name, m.width, m.height))
                .unwrap_or_else(|| "No monitor".into());
            egui::ComboBox::from_id_salt("monitor_picker")
                .selected_text(selected_label)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (idx, monitor) in self.monitors.iter().enumerate() {
                        let label =
                            format!("{} ({}x{})", monitor.name, monitor.width, monitor.height);
                        ui.selectable_value(&mut self.selected_monitor_idx, idx, label);
                    }
                });
            if ui.button("Refresh").clicked() {
                self.refresh_monitors();
            }
        });

        if ui
            .add_enabled(
                !self.monitors.is_empty(),
                egui::Button::new("Capture Selected Monitor"),
            )
            .clicked()
            && let Some(monitor) = self.monitors.get(self.selected_monitor_idx)
        {
            self.run_capture(ctx, CaptureSource::Monitor { id: monitor.id });
        }

        ui.separator();
        ui.label("Region (x, y, w, h)");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.region_inputs.x).desired_width(42.0));
            ui.add(egui::TextEdit::singleline(&mut self.region_inputs.y).desired_width(42.0));
            ui.add(egui::TextEdit::singleline(&mut self.region_inputs.width).desired_width(48.0));
            ui.add(egui::TextEdit::singleline(&mut self.region_inputs.height).desired_width(48.0));
        });
        if ui.button("Capture Region").clicked() {
            match self.parse_region_inputs() {
                Ok(region) => self.run_capture(ctx, CaptureSource::Region { region }),
                Err(err) => self.status_line = format!("Invalid region: {err}"),
            }
        }

        ui.separator();
        ui.label("After capture");
        ui.checkbox(&mut self.config.pipeline.save_to_file, "Save image to file");
        ui.checkbox(
            &mut self.config.pipeline.copy_image_to_clipboard,
            "Copy image to clipboard",
        );
        ui.checkbox(
            &mut self.config.pipeline.copy_path_to_clipboard,
            "Copy path or URL",
        );
        ui.checkbox(&mut self.config.pipeline.record_history, "Record history");

        ui.separator();
        ui.label("Views");
        ui.selectable_value(&mut self.active_tab, ActiveTab::Preview, "Preview");
        if ui
            .selectable_value(&mut self.active_tab, ActiveTab::History, "History")
            .clicked()
        {
            self.refresh_history();
        }
        ui.selectable_value(&mut self.active_tab, ActiveTab::Settings, "Settings");
    }

    fn render_preview_tab(&mut self, ui: &mut egui::Ui) {
        if let Some(view) = &self.last_capture {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} | {}x{} | {}",
                    view.report.capture_id,
                    view.report.image.width(),
                    view.report.image.height(),
                    view.report.format
                ));
                if let Some(path) = &view.report.saved_path
                    && ui.button("Copy Path").clicked()
                {
                    let text = path.display().to_string();
                    self.status_line = match self.clipboard.copy_text(&text) {
                        Ok(()) => format!("Copied path: {text}"),
                        Err(err) => format!("Clipboard error: {err}"),
                    };
                }
            });

            if let Some(url) = &view.report.upload_url {
                ui.label(format!("Upload URL: {url}"));
            }

            ui.separator();
            let available = ui.available_size();
            let img_size = view.texture.size_vec2();
            let scale = (available.x / img_size.x)
                .min(available.y / img_size.y)
                .clamp(0.1, 1.0);
            ui.image((view.texture.id(), img_size * scale));
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Trigger a capture from the left panel to preview it here.");
            });
        }
    }

    fn render_history_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Capture History");
            if ui.button("Refresh").clicked() {
                self.refresh_history();
            }
            if ui.button("Clear").clicked() {
                let store = JsonlHistoryStore::new(&self.config.history_path);
                match store.clear() {
                    Ok(()) => {
                        self.history_entries.clear();
                        self.status_line = "History cleared.".into();
                    }
                    Err(err) => self.status_line = format!("Failed to clear history: {err}"),
                }
            }
        });
        ui.separator();

        if self.history_entries.is_empty() {
            ui.label("No captures recorded yet.");
            return;
        }

        let mut copy_target: Option<String> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &self.history_entries {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} | {}x{} {}",
                            entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                            entry.width,
                            entry.height,
                            entry.format
                        ));
                        let target = entry
                            .upload_url
                            .clone()
                            .or_else(|| entry.file_path.as_ref().map(|p| p.display().to_string()));
                        if let Some(text) = target {
                            if ui.button("Copy").clicked() {
                                copy_target = Some(text.clone());
                            }
                            ui.label(text);
                        }
                    });
                });
            }
        });

        if let Some(text) = copy_target {
            self.status_line = match self.clipboard.copy_text(&text) {
                Ok(()) => format!("Copied: {text}"),
                Err(err) => format!("Clipboard error: {err}"),
            };
        }
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();

        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Output directory");
                ui.add(egui::TextEdit::singleline(&mut self.output_dir_input).desired_width(380.0));
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
                ui.label(preview);
                ui.end_row();

                ui.label("Image format");
                egui::ComboBox::from_id_salt("format_picker")
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

                ui.label("Uploader");
                let current_uploader = self
                    .config
                    .pipeline
                    .uploader_id
                    .as_deref()
                    .unwrap_or("None")
                    .to_owned();
                egui::ComboBox::from_id_salt("uploader_picker")
                    .selected_text(current_uploader)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.pipeline.uploader_id, None, "None");
                        for meta in self.uploaders.list_metadata() {
                            ui.selectable_value(
                                &mut self.config.pipeline.uploader_id,
                                Some(meta.id.to_owned()),
                                format!("{} ({})", meta.name, meta.id),
                            );
                        }
                    });
                ui.end_row();
            });

        ui.add_space(12.0);
        if ui.button("Save Settings").clicked() {
            self.config.output_dir = PathBuf::from(self.output_dir_input.trim());
            self.uploaders = UploaderRegistry::with_local_mirror(default_mirror_dir(&self.config));
            self.status_line = match self.config.save(&self.config_path) {
                Ok(()) => format!("Saved settings to {}", self.config_path.display()),
                Err(err) => format!("Failed to save settings: {err}"),
            };
        }
    }
}

impl eframe::App for SharexGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.label(&self.status_line);
        });

        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(245.0)
            .show(ctx, |ui| {
                self.render_sidebar(ctx, ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            ActiveTab::Preview => self.render_preview_tab(ui),
            ActiveTab::History => self.render_history_tab(ui),
            ActiveTab::Settings => self.render_settings_tab(ui),
        });
    }
}

fn create_preview_texture(
    ctx: &egui::Context,
    image: &CapturedImage,
    name: &str,
) -> egui::TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.rgba(),
    );
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}
