use crate::i18n::Strings;
use crate::theme::{VoltShotPalette, view_heading};
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::SystemTime;
use voltshot_core::{CapturedImage, ClipboardSink, CoreError, ImageFormat, resolve_unique_path};
use voltshot_tools::{
    ColorSample, CombineOptions, CombineOrientation, ContactSheetOptions, FallbackOcrChain,
    HashAlgorithm, IndexFormat, IndexOptions, OcrEngine, OcrOptions, OcrResult, RulerMeasurement,
    WinRtOcr, combine_images, compute_file_hash, decode_qr_image, find_tesseract,
    generate_contact_sheet, generate_qr_image, render_index, scan_directory, verify_hash,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTab {
    QrCode,
    Color,
    Ruler,
    Combiner,
    Hash,
    Indexer,
    Ocr,
}

/// Shared host context passed into `ToolsPanelState::show`.
pub struct ToolsContext<'a> {
    pub output_dir: &'a Path,
    pub latest_capture: Option<&'a Path>,
    pub clipboard: &'a dyn ClipboardSink,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CombineLayout {
    Vertical,
    Horizontal,
    Grid,
}

struct SavedImage {
    path: PathBuf,
    dimensions: (u32, u32),
}

/// Interactive VoltShot Tools workspace panel state.
pub struct ToolsPanelState {
    active_tab: ToolTab,

    qr_input: String,
    qr_module_px: u32,
    qr_cached_key: Option<(String, u32)>,
    qr_image: Option<CapturedImage>,
    qr_texture: Option<egui::TextureHandle>,
    qr_error: Option<String>,
    qr_decode_path: String,
    qr_decoded: Option<Result<Vec<String>, CoreError>>,

    color_hex_input: String,
    pick_color_requested: bool,

    ruler_start: (i32, i32),
    ruler_end: (i32, i32),

    combine_layout: CombineLayout,
    combine_paths: String,
    combine_options: ContactSheetOptions,
    combine_result: Option<Result<SavedImage, CoreError>>,
    no_recent_screenshots: bool,

    hash_path_input: String,
    hash_algorithm: HashAlgorithm,
    hash_expected_input: String,
    hash_result: Option<String>,
    hash_error: Option<String>,

    indexer_path_input: String,
    indexer_format: IndexFormat,
    indexer_max_depth: usize,
    indexer_include_hidden: bool,
    indexer_output: String,
    indexer_error: Option<String>,

    ocr_language: String,
    ocr_winrt_available: bool,
    ocr_tesseract_path: Option<PathBuf>,
    ocr_checked: bool,
    ocr_region_requested: bool,
    ocr_pending: Option<Receiver<Result<OcrResult, CoreError>>>,
    ocr_output: String,
    ocr_engine: Option<String>,
    ocr_error: Option<CoreError>,
}

impl Default for ToolsPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolsPanelState {
    pub fn new() -> Self {
        Self {
            active_tab: ToolTab::QrCode,
            qr_input: "https://voltshot.app".to_owned(),
            qr_module_px: 6,
            qr_cached_key: None,
            qr_image: None,
            qr_texture: None,
            qr_error: None,
            qr_decode_path: String::new(),
            qr_decoded: None,
            color_hex_input: "#3B82F6".to_owned(),
            pick_color_requested: false,
            ruler_start: (0, 0),
            ruler_end: (160, 90),
            combine_layout: CombineLayout::Vertical,
            combine_paths: String::new(),
            combine_options: ContactSheetOptions::default(),
            combine_result: None,
            no_recent_screenshots: false,
            hash_path_input: String::new(),
            hash_algorithm: HashAlgorithm::Sha256,
            hash_expected_input: String::new(),
            hash_result: None,
            hash_error: None,
            indexer_path_input: String::new(),
            indexer_format: IndexFormat::Markdown,
            indexer_max_depth: 3,
            indexer_include_hidden: false,
            indexer_output: String::new(),
            indexer_error: None,
            ocr_language: "eng".to_owned(),
            ocr_winrt_available: false,
            ocr_tesseract_path: None,
            ocr_checked: false,
            ocr_region_requested: false,
            ocr_pending: None,
            ocr_output: String::new(),
            ocr_engine: None,
            ocr_error: None,
        }
    }

    pub fn active_tab(&self) -> ToolTab {
        self.active_tab
    }

    /// Consume the request to open the screen color picker.
    pub fn take_pick_color_request(&mut self) -> bool {
        std::mem::take(&mut self.pick_color_requested)
    }

    /// Display the color selected by the screen overlay.
    pub fn set_picked_color(&mut self, hex: String) {
        self.active_tab = ToolTab::Color;
        self.color_hex_input = hex;
    }

    /// Consume the request to capture a region for OCR.
    pub fn take_ocr_region_request(&mut self) -> bool {
        std::mem::take(&mut self.ocr_region_requested)
    }

    /// Display recognized text and the engine that produced it.
    pub fn set_ocr_result(&mut self, text: String, engine: &str) {
        self.active_tab = ToolTab::Ocr;
        self.update_ocr_result(text, engine);
    }

    fn update_ocr_result(&mut self, text: String, engine: &str) {
        self.ocr_pending = None;
        self.ocr_output = text;
        self.ocr_engine = Some(engine.to_owned());
        self.ocr_error = None;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools_ctx: ToolsContext<'_>,
    ) -> Option<String> {
        ui.horizontal_wrapped(|ui| {
            view_heading(ui, s.tools, p);
            ui.add_space(12.0_f32);
            for (tab, label) in [
                (ToolTab::QrCode, s.qr_code),
                (ToolTab::Color, s.color_tool),
                (ToolTab::Ruler, s.ruler_tool),
                (ToolTab::Combiner, s.combiner_tool),
                (ToolTab::Hash, s.hash_tool),
                (ToolTab::Indexer, s.indexer_tool),
                (ToolTab::Ocr, s.ocr_tool),
            ] {
                let selected = self.active_tab == tab;
                let btn = egui::Button::new(RichText::new(label).color(if selected {
                    p.text_primary
                } else {
                    p.text_muted
                }))
                .fill(if selected { p.accent_subtle } else { p.card_bg })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected { p.accent } else { p.border },
                ))
                .corner_radius(CornerRadius::same(6));
                if ui.add(btn).clicked() {
                    self.active_tab = tab;
                }
            }
        });
        ui.add_space(6.0_f32);
        ui.separator();
        ui.add_space(6.0_f32);

        egui::ScrollArea::vertical()
            .id_salt("tools_panel")
            .show(ui, |ui| match self.active_tab {
                ToolTab::QrCode => self.render_qr_tab(ctx, ui, p, s, &tools_ctx),
                ToolTab::Color => self.render_color_tab(ui, p, s, tools_ctx.clipboard),
                ToolTab::Ruler => {
                    self.render_ruler_tab(ui, p, s);
                    None
                }
                ToolTab::Combiner => self.render_combiner_tab(ui, p, s, &tools_ctx),
                ToolTab::Hash => self.render_hash_tab(ui, p, s, &tools_ctx),
                ToolTab::Indexer => self.render_indexer_tab(ui, p, s, &tools_ctx),
                ToolTab::Ocr => self.render_ocr_tab(ui, p, s, &tools_ctx),
            })
            .inner
    }

    fn render_qr_tab(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools: &ToolsContext<'_>,
    ) -> Option<String> {
        let mut status_msg = None;

        ui.horizontal_wrapped(|ui| {
            ui.label(s.text_or_url);
            ui.add(
                egui::TextEdit::singleline(&mut self.qr_input)
                    .hint_text(s.qr_input_hint)
                    .desired_width(360.0),
            );
            ui.label(s.qr_scale);
            for px in [4_u32, 6_u32, 8_u32] {
                ui.selectable_value(&mut self.qr_module_px, px, format!("{px}px"));
            }
        });
        ui.add_space(8.0);

        self.refresh_qr_preview(ctx);

        egui::Frame::NONE
            .fill(p.card_bg)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                if let Some(err) = &self.qr_error {
                    ui.label(RichText::new(err).color(p.danger));
                } else if let Some(tex) = &self.qr_texture {
                    ui.horizontal(|ui| {
                        let size = tex.size_vec2().min(Vec2::splat(260.0));
                        ui.add(
                            egui::Image::new((tex.id(), size)).corner_radius(CornerRadius::same(4)),
                        );
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            if let Some(img) = &self.qr_image {
                                ui.label(
                                    RichText::new(format!(
                                        "{}: {} × {} px",
                                        s.image_dimensions,
                                        img.width(),
                                        img.height()
                                    ))
                                    .color(p.text_muted),
                                );
                            }
                            ui.add_space(8.0);
                            if ui.button(s.copy_image).clicked()
                                && let Some(img) = &self.qr_image
                            {
                                status_msg = Some(copy_status(s, tools.clipboard.copy_image(img)));
                            }
                            if ui.button(s.save_png).clicked()
                                && let Some(img) = &self.qr_image
                            {
                                status_msg = Some(save_status(
                                    s,
                                    &save_tool_png(img, tools.output_dir, "VoltShot_QR"),
                                ));
                            }
                        });
                    });
                }
            });

        ui.separator();
        ui.heading(s.qr_decode_section);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.qr_decode_path)
                    .hint_text(s.image_path)
                    .desired_width(320.0),
            );
            let path = self.qr_decode_path.trim();
            if ui
                .add_enabled(!path.is_empty(), egui::Button::new(s.decode_qr))
                .clicked()
            {
                self.qr_decoded = Some(decode_qr_file(Path::new(path)));
            }
            if let Some(latest) = tools.latest_capture
                && ui.button(s.decode_latest_capture).clicked()
            {
                self.qr_decode_path = latest.display().to_string();
                self.qr_decoded = Some(decode_qr_file(latest));
            }
        });
        match &self.qr_decoded {
            Some(Ok(payloads)) if payloads.is_empty() => {
                ui.label(s.no_qr_found);
            }
            Some(Ok(payloads)) => {
                for payload in payloads {
                    ui.horizontal(|ui| {
                        if ui.small_button(s.copy).clicked() {
                            status_msg = Some(copy_status(s, tools.clipboard.copy_text(payload)));
                        }
                        ui.add(egui::Label::new(RichText::new(payload).monospace()).wrap());
                    });
                }
            }
            Some(Err(error)) => {
                ui.label(RichText::new(error.to_string()).color(p.danger));
            }
            None => {}
        }
        status_msg
    }

    fn refresh_qr_preview(&mut self, ctx: &egui::Context) {
        let key = (self.qr_input.clone(), self.qr_module_px);
        if self.qr_cached_key.as_ref() == Some(&key) {
            return;
        }
        match generate_qr_image(&self.qr_input, self.qr_module_px, 2) {
            Ok(img) => {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.rgba(),
                );
                let tex = ctx.load_texture(
                    "voltshot_tools_qr_preview",
                    color,
                    egui::TextureOptions::NEAREST,
                );
                self.qr_image = Some(img);
                self.qr_texture = Some(tex);
                self.qr_error = None;
            }
            Err(err) => {
                self.qr_image = None;
                self.qr_texture = None;
                self.qr_error = Some(err.to_string());
            }
        }
        self.qr_cached_key = Some(key);
    }

    fn render_color_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        ui.horizontal_wrapped(|ui| {
            ui.label(s.hex_color);
            ui.add(
                egui::TextEdit::singleline(&mut self.color_hex_input)
                    .hint_text("#3B82F6")
                    .desired_width(140.0),
            );
            for preset in ["#3B82F6", "#10B981", "#EF4444", "#F59E0B", "#1B1F24"] {
                if ui.small_button(preset).clicked() {
                    self.color_hex_input = preset.to_owned();
                }
            }
        });
        if ui.button(s.pick_from_screen).clicked() {
            self.pick_color_requested = true;
        }
        ui.add_space(10.0);

        match ColorSample::from_hex(&self.color_hex_input) {
            Ok(sample) => {
                let readouts = format_color_readouts(sample);
                egui::Frame::NONE
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (swatch_rect, _) =
                                ui.allocate_exact_size(Vec2::new(96.0, 96.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                swatch_rect,
                                CornerRadius::same(6),
                                Color32::from_rgba_unmultiplied(
                                    sample.r, sample.g, sample.b, sample.a,
                                ),
                            );
                            ui.painter().rect_stroke(
                                swatch_rect,
                                CornerRadius::same(6),
                                Stroke::new(1.0_f32, p.border),
                                egui::StrokeKind::Inside,
                            );
                            ui.add_space(14.0);

                            egui::Grid::new("color_converter_grid")
                                .num_columns(3)
                                .spacing([12.0, 6.0])
                                .show(ui, |ui| {
                                    for (index, (label, value)) in readouts.iter().enumerate() {
                                        let label =
                                            if index == 4 { s.decimal_color } else { *label };
                                        ui.label(RichText::new(label).strong());
                                        ui.label(RichText::new(value).monospace());
                                        if ui.small_button(s.copy).clicked() {
                                            status_msg =
                                                Some(copy_status(s, clipboard.copy_text(value)));
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    });
            }
            Err(err) => {
                ui.label(RichText::new(err.to_string()).color(p.danger));
            }
        }

        status_msg
    }

    fn render_ruler_tab(&mut self, ui: &mut egui::Ui, p: &VoltShotPalette, s: &Strings) {
        ui.horizontal_wrapped(|ui| {
            for (label, point) in [
                (s.ruler_start, &mut self.ruler_start),
                (s.ruler_end, &mut self.ruler_end),
            ] {
                ui.label(label);
                ui.add(egui::DragValue::new(&mut point.0).prefix("x: "));
                ui.add(egui::DragValue::new(&mut point.1).prefix("y: "));
            }
        });
        let measurement = RulerMeasurement::between(self.ruler_start, self.ruler_end);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().min(320.0), 156.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(6), p.card_bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, p.border),
            egui::StrokeKind::Inside,
        );
        let delta = Vec2::new(
            (i64::from(measurement.end.0) - i64::from(measurement.start.0)) as f32,
            (i64::from(measurement.end.1) - i64::from(measurement.start.1)) as f32,
        );
        let inner = rect.shrink(12.0).size().max(Vec2::ZERO);
        let scale = (inner.x / delta.x.abs().max(1.0)).min(inner.y / delta.y.abs().max(1.0));
        let half = delta * scale / 2.0;
        let start = rect.center() - half;
        let end = rect.center() + half;
        painter.line_segment([start, end], Stroke::new(2.0_f32, p.accent));
        painter.circle_filled(start, 4.0, p.text_primary);
        painter.circle_filled(end, 3.0, p.accent);

        egui::Grid::new("ruler_measurements").show(ui, |ui| {
            for (label, value) in ruler_readouts(s, measurement) {
                ui.label(label);
                ui.monospace(value);
                ui.end_row();
            }
        });
    }

    fn render_combiner_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools: &ToolsContext<'_>,
    ) -> Option<String> {
        ui.horizontal_wrapped(|ui| {
            for (layout, label) in [
                (CombineLayout::Vertical, s.vertical_strip),
                (CombineLayout::Horizontal, s.horizontal_strip),
                (CombineLayout::Grid, s.contact_sheet_grid),
            ] {
                ui.selectable_value(&mut self.combine_layout, layout, label);
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.combine_paths)
                .hint_text(s.image_list_hint)
                .desired_rows(5)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            if let Some(path) = tools.latest_capture
                && ui.button(s.add_latest_capture).clicked()
            {
                append_image_path(&mut self.combine_paths, path);
            }
            if ui.button(s.add_recent_screenshots).clicked() {
                let paths = recent_screenshots(tools.output_dir);
                self.no_recent_screenshots = paths.is_empty();
                for path in paths {
                    append_image_path(&mut self.combine_paths, &path);
                }
            }
            if ui.button(s.clear).clicked() {
                self.combine_paths.clear();
                self.combine_result = None;
                self.no_recent_screenshots = false;
            }
        });
        if self.no_recent_screenshots {
            ui.label(s.no_recent_screenshots);
        }

        let opts = &mut self.combine_options;
        ui.horizontal_wrapped(|ui| {
            if self.combine_layout == CombineLayout::Grid {
                ui.label(s.grid_columns);
                ui.add(egui::DragValue::new(&mut opts.columns).range(1..=12));
                ui.label(s.thumbnail_width);
                ui.add(
                    egui::DragValue::new(&mut opts.thumb_width)
                        .range(1..=1024)
                        .suffix(" px"),
                );
                ui.label(s.thumbnail_height);
                ui.add(
                    egui::DragValue::new(&mut opts.thumb_height)
                        .range(1..=1024)
                        .suffix(" px"),
                );
                ui.label(s.image_padding);
                ui.add(
                    egui::DragValue::new(&mut opts.padding)
                        .range(0..=128)
                        .suffix(" px"),
                );
            }
            ui.label(s.image_gap);
            ui.add(
                egui::DragValue::new(&mut opts.gap)
                    .range(0..=128)
                    .suffix(" px"),
            );
        });
        let mut status = None;
        if ui
            .add_enabled(
                !self.combine_paths.trim().is_empty(),
                egui::Button::new(s.combine_and_save),
            )
            .clicked()
        {
            let result = combine_and_save(
                &self.combine_paths,
                self.combine_layout,
                opts,
                tools.output_dir,
            );
            status = Some(save_status(s, &result));
            self.combine_result = Some(result);
        }
        if let Some(result) = &self.combine_result {
            let color = if let Ok(saved) = result {
                let (width, height) = saved.dimensions;
                ui.label(format!("{}: {width} × {height} px", s.image_dimensions));
                p.success
            } else {
                p.danger
            };
            ui.label(RichText::new(save_status(s, result)).color(color));
        }
        status
    }

    fn render_hash_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools: &ToolsContext<'_>,
    ) -> Option<String> {
        let mut status_msg = None;
        let old_algorithm = self.hash_algorithm;
        let mut file_changed = false;

        egui::Grid::new("hash_tool_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(s.file_path);
                ui.horizontal(|ui| {
                    file_changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.hash_path_input)
                                .desired_width(320.0),
                        )
                        .changed();
                    if let Some(latest) = tools.latest_capture
                        && ui.button(s.use_latest_capture).clicked()
                    {
                        self.hash_path_input = latest.display().to_string();
                        file_changed = true;
                    }
                });
                ui.end_row();

                ui.label(s.hash_algorithm);
                ui.horizontal(|ui| {
                    for alg in [
                        HashAlgorithm::Md5,
                        HashAlgorithm::Sha1,
                        HashAlgorithm::Sha256,
                        HashAlgorithm::Sha512,
                    ] {
                        ui.selectable_value(&mut self.hash_algorithm, alg, alg.name());
                    }
                });
                ui.end_row();

                ui.label(s.expected_hash);
                ui.add(
                    egui::TextEdit::singleline(&mut self.hash_expected_input)
                        .hint_text(s.expected_hash_hint)
                        .desired_width(360.0),
                );
                ui.end_row();
            });

        if file_changed || old_algorithm != self.hash_algorithm {
            self.hash_result = None;
            self.hash_error = None;
        }
        ui.add_space(8.0);
        let compute_btn =
            egui::Button::new(RichText::new(s.compute_hash).color(Color32::WHITE)).fill(p.accent);
        if ui.add(compute_btn).clicked() {
            let trimmed = self.hash_path_input.trim();
            if trimmed.is_empty() {
                status_msg = Some(s.enter_file_path.to_owned());
            } else {
                match compute_file_hash(self.hash_algorithm, Path::new(trimmed)) {
                    Ok(digest) => {
                        self.hash_result = Some(digest);
                        self.hash_error = None;
                        status_msg = Some(format!("{}: {}", s.computed_hash, self.hash_algorithm));
                    }
                    Err(err) => {
                        self.hash_result = None;
                        self.hash_error = Some(err.to_string());
                    }
                }
            }
        }

        ui.add_space(10.0);
        if let Some(err) = &self.hash_error {
            ui.label(RichText::new(err).color(p.danger));
        }
        if let Some(digest) = &self.hash_result {
            egui::Frame::NONE
                .fill(p.card_bg)
                .stroke(Stroke::new(1.0_f32, p.border))
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{}:", self.hash_algorithm.name())).strong(),
                        );
                        ui.label(RichText::new(digest).monospace());
                        if ui.small_button(s.copy).clicked() {
                            status_msg = Some(copy_status(s, tools.clipboard.copy_text(digest)));
                        }
                    });
                    let expected = self.hash_expected_input.trim();
                    if !expected.is_empty() {
                        ui.add_space(4.0);
                        if verify_hash(digest, expected) {
                            ui.label(RichText::new(s.hash_match).strong().color(p.success));
                        } else {
                            ui.label(RichText::new(s.hash_mismatch).strong().color(p.danger));
                        }
                    }
                });
        }

        status_msg
    }

    fn render_indexer_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools: &ToolsContext<'_>,
    ) -> Option<String> {
        let mut status_msg = None;

        if self.indexer_path_input.is_empty() {
            self.indexer_path_input = tools.output_dir.display().to_string();
        }

        egui::Grid::new("indexer_tool_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(s.folder_path);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.indexer_path_input)
                            .desired_width(360.0),
                    );
                    if ui.button(s.screenshots_folder).clicked() {
                        self.indexer_path_input = tools.output_dir.display().to_string();
                    }
                });
                ui.end_row();

                ui.label(s.format);
                ui.horizontal(|ui| {
                    for (fmt, label) in [
                        (IndexFormat::Markdown, "Markdown"),
                        (IndexFormat::Html, "HTML"),
                        (IndexFormat::Json, "JSON"),
                        (IndexFormat::PlainText, "TXT"),
                    ] {
                        ui.selectable_value(&mut self.indexer_format, fmt, label);
                    }
                });
                ui.end_row();

                ui.label(s.options);
                ui.horizontal(|ui| {
                    ui.label(s.max_depth);
                    ui.add(egui::DragValue::new(&mut self.indexer_max_depth).range(1..=10));
                    ui.checkbox(&mut self.indexer_include_hidden, s.include_hidden);
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let gen_btn = egui::Button::new(RichText::new(s.generate_index).color(Color32::WHITE))
                .fill(p.accent);
            if ui.add(gen_btn).clicked() {
                let target = Path::new(self.indexer_path_input.trim());
                let opts = IndexOptions {
                    max_depth: self.indexer_max_depth,
                    include_hidden: self.indexer_include_hidden,
                    format: self.indexer_format,
                };
                match scan_directory(target, &opts)
                    .and_then(|tree| render_index(&tree, self.indexer_format))
                {
                    Ok(rendered) => {
                        self.indexer_output = rendered;
                        self.indexer_error = None;
                        status_msg = Some(s.index_generated.to_owned());
                    }
                    Err(err) => {
                        self.indexer_error = Some(err.to_string());
                    }
                }
            }
            if ui
                .add_enabled(!self.indexer_output.is_empty(), egui::Button::new(s.copy))
                .clicked()
            {
                status_msg = Some(copy_status(
                    s,
                    tools.clipboard.copy_text(&self.indexer_output),
                ));
            }
        });

        if let Some(err) = &self.indexer_error {
            ui.add_space(6.0);
            ui.label(RichText::new(err).color(p.danger));
        }

        if !self.indexer_output.is_empty() {
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("indexer_output_scroll")
                .max_height(320.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.indexer_output)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(14),
                    );
                });
        }

        status_msg
    }

    fn render_ocr_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &VoltShotPalette,
        s: &Strings,
        tools: &ToolsContext<'_>,
    ) -> Option<String> {
        if !self.ocr_checked {
            self.check_ocr_engines();
        }
        if let Some(receiver) = &self.ocr_pending {
            match receiver.try_recv() {
                Ok(Ok(result)) => self.update_ocr_result(result.text, result.engine_name),
                Ok(Err(error)) => self.set_ocr_error(error),
                Err(TryRecvError::Disconnected) => {
                    self.ocr_pending = None;
                    return Some(s.ocr_interrupted.to_owned());
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        for (engine, available) in [
            (s.winrt_ocr, self.ocr_winrt_available),
            (s.tesseract_cli, self.ocr_tesseract_path.is_some()),
        ] {
            ui.horizontal(|ui| {
                ui.strong(engine);
                let (label, color) = if available {
                    (s.available, p.success)
                } else {
                    (s.unavailable, p.warning)
                };
                ui.label(RichText::new(label).color(color));
            });
        }
        if ui.small_button(s.recheck_ocr).clicked() {
            self.check_ocr_engines();
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(s.language);
            ui.add(
                egui::TextEdit::singleline(&mut self.ocr_language)
                    .hint_text("eng")
                    .desired_width(100.0),
            );
            for preset in ["eng", "pol", "eng+pol"] {
                if ui.small_button(preset).clicked() {
                    self.ocr_language = preset.to_owned();
                }
            }
        });
        if let Some(path) = tools.latest_capture {
            ui.label(format!("{}: {}", s.ocr_target, path.display()));
        } else {
            ui.label(s.no_screenshots_yet);
        }

        let ready = self.ocr_pending.is_none()
            && (self.ocr_winrt_available || self.ocr_tesseract_path.is_some());
        let mut status = None;
        ui.horizontal_wrapped(|ui| {
            let run = egui::Button::new(s.run_ocr_latest);
            if ui
                .add_enabled(ready && tools.latest_capture.is_some(), run)
                .clicked()
                && let Some(path) = tools.latest_capture
                && let Err(error) = self.start_ocr(path.to_path_buf(), ui.ctx().clone())
            {
                self.set_ocr_error(error);
            }
            if ui
                .add_enabled(ready, egui::Button::new(s.capture_ocr_region))
                .clicked()
            {
                self.ocr_region_requested = true;
            }
            if ui
                .add_enabled(!self.ocr_output.is_empty(), egui::Button::new(s.copy))
                .clicked()
            {
                status = Some(copy_status(s, tools.clipboard.copy_text(&self.ocr_output)));
            }
        });
        if self.ocr_pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(s.running_ocr);
            });
        }
        if let Some(error) = &self.ocr_error {
            ui.label(RichText::new(error.to_string()).color(p.danger));
        }
        if let Some(engine) = &self.ocr_engine {
            let label = match engine.as_str() {
                "windows-winrt-ocr" => s.winrt_ocr,
                "tesseract-cli" => s.tesseract_cli,
                other => other,
            };
            ui.label(format!("{} ({label})", s.ocr_completed));
            if self.ocr_output.is_empty() {
                ui.label(s.ocr_no_text);
            } else {
                ui.add(
                    egui::TextEdit::multiline(&mut self.ocr_output)
                        .desired_width(f32::INFINITY)
                        .desired_rows(8),
                );
            }
        }
        status
    }

    fn check_ocr_engines(&mut self) {
        self.ocr_winrt_available = WinRtOcr::new().is_available();
        self.ocr_tesseract_path = find_tesseract(None);
        self.ocr_checked = true;
    }

    fn start_ocr(&mut self, path: PathBuf, repaint: egui::Context) -> Result<(), CoreError> {
        let options = OcrOptions {
            language: self.ocr_language.trim().to_owned(),
            psm: None,
        };
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("voltshot-ocr".into())
            .spawn(move || {
                let result = CapturedImage::load_from_file(&path)
                    .and_then(|image| FallbackOcrChain::default().recognize(&image, &options));
                let _ = sender.send(result);
                repaint.request_repaint();
            })?;
        self.ocr_pending = Some(receiver);
        self.ocr_error = None;
        self.ocr_engine = None;
        self.ocr_output.clear();
        Ok(())
    }

    fn set_ocr_error(&mut self, error: CoreError) {
        self.ocr_pending = None;
        self.ocr_error = Some(error);
        self.ocr_engine = None;
        self.ocr_output.clear();
    }
}

/// Format a color sample in the five formats displayed by the converter.
pub fn format_color_readouts(sample: ColorSample) -> [(&'static str, String); 5] {
    let (h, s, l) = sample.to_hsl();
    let (c, m, y, k) = sample.to_cmyk();
    [
        ("HEX", sample.to_hex()),
        ("RGB", sample.to_rgb_string()),
        ("HSL", format!("hsl({h:.0}°, {s:.0}%, {l:.0}%)")),
        ("CMYK", format!("cmyk({c:.0}%, {m:.0}%, {y:.0}%, {k:.0}%)")),
        ("Decimal", sample.to_decimal().to_string()),
    ]
}

fn decode_qr_file(path: &Path) -> Result<Vec<String>, CoreError> {
    decode_qr_image(&CapturedImage::load_from_file(path)?)
}

fn ruler_readouts(s: &Strings, m: RulerMeasurement) -> [(&'static str, String); 5] {
    [
        ("dx", format!("{} px", m.dx)),
        ("dy", format!("{} px", m.dy)),
        (s.ruler_distance, format!("{:.2} px", m.distance)),
        (s.ruler_angle, format!("{:.2}°", m.angle_deg)),
        (s.ruler_area, format!("{} px²", m.area)),
    ]
}

fn append_image_path(list: &mut String, path: &Path) {
    let path = path.to_string_lossy();
    if !list.lines().any(|entry| entry.trim() == path) {
        if !list.is_empty() && !list.ends_with('\n') {
            list.push('\n');
        }
        list.push_str(&path);
    }
}

fn recent_screenshots(output_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(output_dir) else {
        return Vec::new();
    };
    let mut images: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_stem()?.to_str()?;
            if name.starts_with("VoltShot_Combiner_") || name.starts_with("VoltShot_QR_") {
                return None;
            }
            path.extension()?.to_str()?.parse::<ImageFormat>().ok()?;
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path))
        })
        .collect();
    images.sort_unstable_by(|left, right| right.cmp(left));
    images.into_iter().take(8).map(|(_, path)| path).collect()
}

fn combine_and_save(
    list: &str,
    layout: CombineLayout,
    options: &ContactSheetOptions,
    output_dir: &Path,
) -> Result<SavedImage, CoreError> {
    let images = list
        .lines()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| CapturedImage::load_from_file(Path::new(entry)))
        .collect::<Result<Vec<_>, _>>()?;
    let image = if layout == CombineLayout::Grid {
        generate_contact_sheet(&images, options)?
    } else {
        let orientation = if layout == CombineLayout::Vertical {
            CombineOrientation::Vertical
        } else {
            CombineOrientation::Horizontal
        };
        let refs: Vec<_> = images.iter().collect();
        combine_images(
            &refs,
            &CombineOptions {
                orientation,
                gap: options.gap,
                ..Default::default()
            },
        )?
    };
    save_tool_png(&image, output_dir, "VoltShot_Combiner")
}

fn save_tool_png(
    image: &CapturedImage,
    output_dir: &Path,
    prefix: &str,
) -> Result<SavedImage, CoreError> {
    fs::create_dir_all(output_dir)?;
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S");
    let path = resolve_unique_path(
        output_dir,
        &format!("{prefix}_{timestamp}"),
        ImageFormat::Png.extension(),
    );
    fs::write(&path, image.encode(ImageFormat::Png)?)?;
    Ok(SavedImage {
        path,
        dimensions: (image.width(), image.height()),
    })
}

fn save_status(s: &Strings, result: &Result<SavedImage, CoreError>) -> String {
    match result {
        Ok(saved) => format!("{} {}", s.saved_to, saved.path.display()),
        Err(error) => format!("{}: {error}", s.save_failed),
    }
}

fn copy_status(s: &Strings, result: Result<(), CoreError>) -> String {
    match result {
        Ok(()) => s.copied.to_owned(),
        Err(error) => format!("{}: {error}", s.clipboard_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_readouts_format_hex_rgb_hsl_cmyk_and_decimal() {
        let sample = ColorSample::from_hex("#3B82F6").expect("valid hex");
        let readouts = format_color_readouts(sample);
        assert_eq!(readouts[0], ("HEX", "#3B82F6".to_owned()));
        assert_eq!(readouts[1], ("RGB", "rgb(59, 130, 246)".to_owned()));
        assert!(readouts[2].1.starts_with("hsl("));
        assert!(readouts[3].1.starts_with("cmyk("));
        assert_eq!(readouts[4], ("Decimal", "3900150".to_owned()));
    }

    #[test]
    fn tools_state_defaults_are_well_formed() {
        let state = ToolsPanelState::new();
        assert_eq!(state.active_tab(), ToolTab::QrCode);
        assert!(ColorSample::from_hex(&state.color_hex_input).is_ok());
        assert!(generate_qr_image(&state.qr_input, state.qr_module_px, 2).is_ok());
    }

    #[test]
    fn decodes_a_generated_qr_png() {
        let folder = tempfile::tempdir().unwrap();
        let payload = "VoltShot QR round-trip";
        let image = generate_qr_image(payload, 8, 4).unwrap();
        let saved = save_tool_png(&image, folder.path(), "VoltShot_QR").unwrap();
        assert_eq!(decode_qr_file(&saved.path).unwrap(), vec![payload]);
    }

    #[test]
    fn color_pick_request_is_consumed_and_result_updates_converter() {
        let mut tools = ToolsPanelState::new();
        assert!(!tools.take_pick_color_request());
        tools.pick_color_requested = true;
        assert!(tools.take_pick_color_request());
        assert!(!tools.take_pick_color_request());

        tools.set_picked_color("#102030".into());
        assert_eq!(tools.active_tab(), ToolTab::Color);
        assert_eq!(
            ColorSample::from_hex(&tools.color_hex_input)
                .unwrap()
                .to_hex(),
            "#102030"
        );
    }

    #[test]
    fn ocr_region_request_is_consumed_and_result_updates_ocr_tab() {
        let mut tools = ToolsPanelState::new();
        assert!(!tools.take_ocr_region_request());
        tools.ocr_region_requested = true;
        assert!(tools.take_ocr_region_request());
        assert!(!tools.take_ocr_region_request());

        tools.ocr_error = Some(CoreError::CaptureFailed("old error".into()));
        tools.set_ocr_result("Recognized text".into(), "windows-winrt-ocr");
        assert_eq!(tools.active_tab(), ToolTab::Ocr);
        assert_eq!(tools.ocr_output, "Recognized text");
        assert_eq!(tools.ocr_engine.as_deref(), Some("windows-winrt-ocr"));
        assert!(tools.ocr_error.is_none());
    }

    #[test]
    fn ruler_readouts_report_signed_deltas_and_units() {
        let s = crate::i18n::UiLanguage::En.strings();
        let measurement = RulerMeasurement::between((10, 20), (13, 16));
        assert_eq!(
            ruler_readouts(s, measurement).map(|(_, value)| value),
            ["3 px", "-4 px", "5.00 px", "-53.13°", "12 px²"].map(str::to_owned)
        );
    }

    #[test]
    fn combines_files_as_a_strip_and_as_a_contact_sheet() {
        let folder = tempfile::tempdir().unwrap();
        let image = CapturedImage::from_rgba(2, 3, vec![255; 24]).unwrap();
        let inputs = ["first", "second"]
            .map(|prefix| save_tool_png(&image, folder.path(), prefix).unwrap().path);
        let mut list = String::new();
        for path in &inputs {
            append_image_path(&mut list, path);
            append_image_path(&mut list, path);
        }
        assert_eq!(list.lines().count(), 2);
        let opts = ContactSheetOptions {
            columns: 1,
            thumb_width: 4,
            thumb_height: 4,
            gap: 1,
            padding: 2,
            ..Default::default()
        };
        for (layout, expected) in [
            (CombineLayout::Vertical, (2, 7)),
            (CombineLayout::Horizontal, (5, 3)),
            (CombineLayout::Grid, (8, 13)),
        ] {
            let saved = combine_and_save(&list, layout, &opts, folder.path()).unwrap();
            assert_eq!(saved.dimensions, expected);
            let png = CapturedImage::load_from_file(&saved.path).unwrap();
            assert_eq!((png.width(), png.height()), expected);
        }
    }
}
