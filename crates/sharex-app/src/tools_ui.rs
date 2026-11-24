use crate::i18n::Strings;
use crate::theme::SharexPalette;
use chrono::Utc;
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use sharex_core::{CapturedImage, ClipboardSink, ImageFormat, resolve_unique_path};
use sharex_tools::{
    ColorSample, HashAlgorithm, IndexFormat, IndexOptions, OcrEngine, OcrOptions, TesseractCliOcr,
    compute_file_hash, find_tesseract, generate_qr_image, render_index, scan_directory,
    verify_hash,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTab {
    QrCode,
    Color,
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

/// Interactive ShareX Tools workspace panel state.
pub struct ToolsPanelState {
    active_tab: ToolTab,

    qr_input: String,
    qr_module_px: u32,
    qr_cached_key: Option<(String, u32)>,
    qr_image: Option<CapturedImage>,
    qr_texture: Option<egui::TextureHandle>,
    qr_error: Option<String>,

    color_hex_input: String,

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
    ocr_tesseract_path: Option<PathBuf>,
    ocr_checked: bool,
    ocr_output: String,
    ocr_error: Option<String>,
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
            qr_input: "https://getsharex.com".to_owned(),
            qr_module_px: 6,
            qr_cached_key: None,
            qr_image: None,
            qr_texture: None,
            qr_error: None,
            color_hex_input: "#3B82F6".to_owned(),
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
            ocr_tesseract_path: None,
            ocr_checked: false,
            ocr_output: String::new(),
            ocr_error: None,
        }
    }

    pub fn active_tab(&self) -> ToolTab {
        self.active_tab
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        s: &Strings,
        tools_ctx: ToolsContext<'_>,
    ) -> Option<String> {
        ui.horizontal(|ui| {
            ui.heading(s.tools);
            ui.add_space(12.0);
            for (tab, label) in [
                (ToolTab::QrCode, "QR Code"),
                (ToolTab::Color, "Color"),
                (ToolTab::Hash, "Hash"),
                (ToolTab::Indexer, "Indexer"),
                (ToolTab::Ocr, "OCR"),
            ] {
                let selected = self.active_tab == tab;
                let btn = egui::Button::new(RichText::new(label).color(if selected {
                    Color32::WHITE
                } else {
                    p.text_primary
                }))
                .fill(if selected { p.accent } else { p.card_bg });
                if ui.add(btn).clicked() {
                    self.active_tab = tab;
                }
            }
        });
        ui.separator();

        match self.active_tab {
            ToolTab::QrCode => {
                self.render_qr_tab(ctx, ui, p, tools_ctx.output_dir, tools_ctx.clipboard)
            }
            ToolTab::Color => self.render_color_tab(ui, p, tools_ctx.clipboard),
            ToolTab::Hash => {
                self.render_hash_tab(ui, p, tools_ctx.latest_capture, tools_ctx.clipboard)
            }
            ToolTab::Indexer => {
                self.render_indexer_tab(ui, p, tools_ctx.output_dir, tools_ctx.clipboard)
            }
            ToolTab::Ocr => {
                self.render_ocr_tab(ui, p, tools_ctx.latest_capture, tools_ctx.clipboard)
            }
        }
    }

    fn render_qr_tab(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        output_dir: &Path,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        ui.horizontal(|ui| {
            ui.label("Text / URL");
            ui.add(
                egui::TextEdit::singleline(&mut self.qr_input)
                    .hint_text("Enter text or URL to encode...")
                    .desired_width(360.0),
            );
            ui.label("Scale");
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
                                        "QR Image: {}x{} px",
                                        img.width(),
                                        img.height()
                                    ))
                                    .color(p.text_muted),
                                );
                            }
                            ui.add_space(8.0);
                            if ui.button("Copy Image").clicked()
                                && let Some(img) = &self.qr_image
                            {
                                status_msg = Some(match clipboard.copy_image(img) {
                                    Ok(()) => "Copied QR code image to clipboard.".to_owned(),
                                    Err(err) => format!("QR copy failed: {err}"),
                                });
                            }
                            if ui.button("Save PNG").clicked()
                                && let Some(img) = &self.qr_image
                            {
                                status_msg = Some(save_qr_png(img, output_dir));
                            }
                        });
                    });
                }
            });

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
                    "sharex_tools_qr_preview",
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
        p: &SharexPalette,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        ui.horizontal(|ui| {
            ui.label("HEX Color");
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
                                    for (label, value) in &readouts {
                                        ui.label(RichText::new(*label).strong());
                                        ui.label(RichText::new(value).monospace());
                                        if ui.small_button("Copy").clicked() {
                                            status_msg = Some(match clipboard.copy_text(value) {
                                                Ok(()) => format!("Copied {label}: {value}"),
                                                Err(err) => format!("Clipboard error: {err}"),
                                            });
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

    fn render_hash_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        latest_capture: Option<&Path>,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        egui::Grid::new("hash_tool_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("File path");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.hash_path_input)
                            .hint_text("Path to file...")
                            .desired_width(360.0),
                    );
                    if let Some(latest) = latest_capture
                        && ui.button("Use Latest Capture").clicked()
                    {
                        self.hash_path_input = latest.display().to_string();
                    }
                });
                ui.end_row();

                ui.label("Algorithm");
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

                ui.label("Expected hash");
                ui.add(
                    egui::TextEdit::singleline(&mut self.hash_expected_input)
                        .hint_text("Optional expected digest to verify against...")
                        .desired_width(360.0),
                );
                ui.end_row();
            });

        ui.add_space(8.0);
        let compute_btn =
            egui::Button::new(RichText::new("Compute Hash").color(Color32::WHITE)).fill(p.accent);
        if ui.add(compute_btn).clicked() {
            let trimmed = self.hash_path_input.trim();
            if trimmed.is_empty() {
                self.hash_result = None;
                self.hash_error = Some("Enter a file path first.".to_owned());
            } else {
                match compute_file_hash(self.hash_algorithm, Path::new(trimmed)) {
                    Ok(digest) => {
                        self.hash_result = Some(digest);
                        self.hash_error = None;
                        status_msg = Some(format!("Computed {} digest.", self.hash_algorithm));
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
                        if ui.small_button("Copy").clicked() {
                            status_msg = Some(match clipboard.copy_text(digest) {
                                Ok(()) => "Copied hash digest to clipboard.".to_owned(),
                                Err(err) => format!("Clipboard error: {err}"),
                            });
                        }
                    });
                    let expected = self.hash_expected_input.trim();
                    if !expected.is_empty() {
                        ui.add_space(4.0);
                        if verify_hash(digest, expected) {
                            ui.label(
                                RichText::new("MATCH: computed digest matches expected hash.")
                                    .strong()
                                    .color(p.success),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "MISMATCH: computed digest does not match expected hash.",
                                )
                                .strong()
                                .color(p.danger),
                            );
                        }
                    }
                });
        }

        status_msg
    }

    fn render_indexer_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &SharexPalette,
        output_dir: &Path,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;

        if self.indexer_path_input.is_empty() {
            self.indexer_path_input = output_dir.display().to_string();
        }

        egui::Grid::new("indexer_tool_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Folder path");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.indexer_path_input)
                            .desired_width(360.0),
                    );
                    if ui.button("Screenshots Dir").clicked() {
                        self.indexer_path_input = output_dir.display().to_string();
                    }
                });
                ui.end_row();

                ui.label("Format");
                ui.horizontal(|ui| {
                    for (fmt, label) in [
                        (IndexFormat::Markdown, "Markdown"),
                        (IndexFormat::Html, "HTML"),
                        (IndexFormat::Json, "JSON"),
                        (IndexFormat::PlainText, "PlainText"),
                    ] {
                        ui.selectable_value(&mut self.indexer_format, fmt, label);
                    }
                });
                ui.end_row();

                ui.label("Options");
                ui.horizontal(|ui| {
                    ui.label("Max depth:");
                    ui.add(egui::DragValue::new(&mut self.indexer_max_depth).range(1..=10));
                    ui.checkbox(&mut self.indexer_include_hidden, "Include hidden");
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let gen_btn = egui::Button::new(RichText::new("Generate Index").color(Color32::WHITE))
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
                        status_msg = Some("Generated directory index.".to_owned());
                    }
                    Err(err) => {
                        self.indexer_error = Some(err.to_string());
                    }
                }
            }
            if ui
                .add_enabled(!self.indexer_output.is_empty(), egui::Button::new("Copy"))
                .clicked()
            {
                status_msg = Some(match clipboard.copy_text(&self.indexer_output) {
                    Ok(()) => "Copied directory index to clipboard.".to_owned(),
                    Err(err) => format!("Clipboard error: {err}"),
                });
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
        p: &SharexPalette,
        latest_capture: Option<&Path>,
        clipboard: &dyn ClipboardSink,
    ) -> Option<String> {
        let mut status_msg = None;
        let engine = TesseractCliOcr::new(None);
        if !self.ocr_checked {
            self.ocr_tesseract_path = find_tesseract(None);
            self.ocr_checked = true;
        }
        let available = self.ocr_tesseract_path.is_some();

        egui::Frame::NONE
            .fill(p.card_bg)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Tesseract OCR:").strong());
                    if available {
                        let detail = self
                            .ocr_tesseract_path
                            .as_ref()
                            .map(|path| format!("Available ({})", path.display()))
                            .unwrap_or_else(|| "Available".to_owned());
                        ui.label(RichText::new(detail).color(p.success));
                    } else {
                        ui.label(
                            RichText::new("Not found in PATH (install Tesseract CLI to run OCR)")
                                .color(p.warning),
                        );
                    }
                    if ui.small_button("Re-check").clicked() {
                        self.ocr_tesseract_path = find_tesseract(None);
                    }
                });
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Language");
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

        ui.add_space(6.0);
        let latest_display = latest_capture
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "No captured screenshot available yet.".to_owned());
        ui.label(
            RichText::new(format!("Target screenshot: {latest_display}"))
                .size(12.0)
                .color(p.text_muted),
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_run = available && latest_capture.is_some_and(Path::exists);
            let run_btn =
                egui::Button::new(RichText::new("Run OCR on Latest Capture").color(Color32::WHITE))
                    .fill(p.accent);
            if ui.add_enabled(can_run, run_btn).clicked()
                && let Some(path) = latest_capture
            {
                let opts = OcrOptions {
                    language: self.ocr_language.trim().to_owned(),
                    psm: None,
                };
                match CapturedImage::load_from_file(path)
                    .and_then(|img| engine.recognize(&img, &opts))
                {
                    Ok(res) => {
                        self.ocr_output = res.text;
                        self.ocr_error = None;
                        status_msg = Some(format!(
                            "OCR completed ({}, {}).",
                            res.engine_name, res.language
                        ));
                    }
                    Err(err) => {
                        self.ocr_error = Some(err.to_string());
                    }
                }
            }
            if ui
                .add_enabled(!self.ocr_output.is_empty(), egui::Button::new("Copy Text"))
                .clicked()
            {
                status_msg = Some(match clipboard.copy_text(&self.ocr_output) {
                    Ok(()) => "Copied OCR text to clipboard.".to_owned(),
                    Err(err) => format!("Clipboard error: {err}"),
                });
            }
        });

        if let Some(err) = &self.ocr_error {
            ui.add_space(6.0);
            ui.label(RichText::new(err).color(p.danger));
        }

        if !self.ocr_output.is_empty() {
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("ocr_output_scroll")
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.ocr_output)
                            .desired_width(f32::INFINITY)
                            .desired_rows(10),
                    );
                });
        }

        status_msg
    }
}

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

fn save_qr_png(img: &CapturedImage, output_dir: &Path) -> String {
    if let Err(err) = fs::create_dir_all(output_dir) {
        return format!("Failed to create output folder: {err}");
    }
    let stem = format!("ShareX_QR_{}", Utc::now().format("%Y-%m-%d_%H-%M-%S"));
    let target = resolve_unique_path(output_dir, &stem, ImageFormat::Png.extension());
    match img
        .encode(ImageFormat::Png)
        .map_err(|e| e.to_string())
        .and_then(|bytes| fs::write(&target, bytes).map_err(|e| e.to_string()))
    {
        Ok(()) => format!("Saved QR code PNG to {}", target.display()),
        Err(err) => format!("Failed to save QR PNG: {err}"),
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
}
