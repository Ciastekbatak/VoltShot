use crate::theme::VoltShotPalette;
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use voltshot_core::{CapturedImage, ImageFormat, Region};
use voltshot_editor::{
    Annotation, AnnotationKind, EditorDocument, EditorTool, Point, RectF, ResizeHandle, RgbaColor,
    VOLTSHOT_PALETTE, speech_balloon_outline,
};

/// Actions emitted by the VoltShot image editor back to the parent application shell.
pub enum EditorAction {
    None,
    Save {
        image: CapturedImage,
        target_path: Option<PathBuf>,
        format: ImageFormat,
    },
    SaveAsCopy {
        image: CapturedImage,
        format: ImageFormat,
    },
    CopyImage(CapturedImage),
    Close,
}

enum DragDraft {
    Shape {
        start: Point,
        current: Point,
    },
    Freehand {
        points: Vec<Point>,
    },
    MoveSelected {
        id: u64,
        last: Point,
        recorded: bool,
    },
    ResizeSelected(Box<ResizeDrag>),
    SmartEraser {
        points: Vec<Point>,
        stroke_width: f32,
        color: RgbaColor,
    },
}

struct OverlayStamp {
    image: CapturedImage,
    texture: egui::TextureHandle,
}

enum HistoryEntry {
    DocumentEdit,
    ReplacedDocument(Box<EditorDocument>),
}

/// Interactive VoltShot screenshot annotation editor state.
pub struct EditorViewState {
    document: EditorDocument,
    source_path: Option<PathBuf>,
    format: ImageFormat,
    texture: egui::TextureHandle,
    texture_version: u64,
    active_tool: EditorTool,
    color: RgbaColor,
    stroke_width: f32,
    font_size: f32,
    text_draft: String,
    text_background: bool,
    effect_strength: u32,
    magnifier_zoom: u8,
    spotlight_alpha: u8,
    stamp_path: String,
    stamp: Option<OverlayStamp>,
    stamp_opacity: f32,
    overlay_textures: HashMap<u64, egui::TextureHandle>,
    canvas_size: [u32; 2],
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    document_replaced: bool,
    zoom: f32,
    fit_to_view: bool,
    drag: Option<DragDraft>,
    status: String,
}

impl EditorViewState {
    pub fn open(
        ctx: &egui::Context,
        image: CapturedImage,
        source_path: Option<PathBuf>,
        format: ImageFormat,
    ) -> Self {
        let texture = load_editor_texture(ctx, &image, 1);
        let canvas_size = [image.width(), image.height()];
        let status = match &source_path {
            Some(path) => format!("Editing {}", path.display()),
            None => format!(
                "Editing unsaved capture ({}x{})",
                image.width(),
                image.height()
            ),
        };
        Self {
            document: EditorDocument::new(image),
            source_path,
            format,
            texture,
            texture_version: 1,
            active_tool: EditorTool::Arrow,
            color: RgbaColor::RED,
            stroke_width: 4.0,
            font_size: 20.0,
            text_draft: "Note".into(),
            text_background: true,
            effect_strength: 12,
            magnifier_zoom: 2,
            spotlight_alpha: 160,
            stamp_path: String::new(),
            stamp: None,
            stamp_opacity: 1.0,
            overlay_textures: HashMap::new(),
            canvas_size,
            undo: Vec::new(),
            redo: Vec::new(),
            document_replaced: false,
            zoom: 1.0,
            fit_to_view: true,
            drag: None,
            status,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty() || self.document_replaced
    }

    pub fn set_saved(&mut self, path: PathBuf) {
        self.source_path = Some(path.clone());
        self.document.mark_saved();
        self.document_replaced = false;
        self.status = format!("Saved to {}", path.display());
    }

    fn reload_texture(&mut self, ctx: &egui::Context) {
        self.texture_version = self.texture_version.saturating_add(1);
        self.texture = load_editor_texture(ctx, self.document.base_image(), self.texture_version);
        self.canvas_size = [
            self.document.base_image().width(),
            self.document.base_image().height(),
        ];
    }

    fn record_edit(&mut self, entry: HistoryEntry) {
        self.undo.push(entry);
        self.redo.clear();
    }

    fn push_annotation(&mut self, kind: AnnotationKind) {
        self.document.push_annotation(kind);
        self.record_edit(HistoryEntry::DocumentEdit);
    }

    fn replace_document(&mut self, ctx: &egui::Context, replacement: EditorDocument) {
        let previous = std::mem::replace(&mut self.document, replacement);
        self.record_edit(HistoryEntry::ReplacedDocument(Box::new(previous)));
        self.document_replaced = true;
        self.drag = None;
        self.overlay_textures.clear();
        self.reload_texture(ctx);
    }

    fn step_history(&mut self, ctx: &egui::Context, redo: bool) {
        let entry = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let Some(entry) = entry else {
            return;
        };
        self.drag = None;
        let reciprocal = match entry {
            HistoryEntry::DocumentEdit => {
                if redo {
                    self.document.redo();
                } else {
                    self.document.undo();
                }
                HistoryEntry::DocumentEdit
            }
            HistoryEntry::ReplacedDocument(mut previous) => {
                std::mem::swap(&mut self.document, previous.as_mut());
                self.document_replaced = true;
                self.overlay_textures.clear();
                HistoryEntry::ReplacedDocument(previous)
            }
        };
        if redo {
            self.undo.push(reciprocal);
        } else {
            self.redo.push(reciprocal);
        }
        self.reload_texture(ctx);
        self.status = if redo {
            "Redid change."
        } else {
            "Undid last change."
        }
        .into();
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        palette: &VoltShotPalette,
    ) -> EditorAction {
        let mut action = EditorAction::None;
        self.handle_keyboard_shortcuts(ctx, &mut action);

        egui::Frame::NONE
            .fill(palette.card_bg)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .corner_radius(CornerRadius::same(4))
            .show(ui, |ui| {
                self.render_tool_row(ui, palette);
                ui.add_space(4.0);
                self.render_options_and_actions_row(ctx, ui, palette, &mut action);
                self.render_project_controls(ctx, ui);
            });

        ui.add_space(6.0);

        let canvas_frame = egui::Frame::NONE
            .fill(palette.sidebar_bg)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .corner_radius(CornerRadius::same(4));
        canvas_frame.show(ui, |ui| {
            self.render_canvas(ctx, ui, palette);
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&self.status)
                    .size(12.0)
                    .color(palette.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}x{} | {} annotation(s) | Zoom {:.0}%",
                        self.document.base_image().width(),
                        self.document.base_image().height(),
                        self.document.annotations().len(),
                        self.zoom * 100.0
                    ))
                    .size(12.0)
                    .color(palette.text_muted),
                );
            });
        });

        action
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context, action: &mut EditorAction) {
        let typing = ctx.wants_keyboard_input();

        if !typing && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
            self.step_history(ctx, false);
            return;
        }
        if !typing && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
            self.step_history(ctx, true);
            return;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            match self.document.flatten() {
                Ok(img) => {
                    *action = EditorAction::Save {
                        image: img,
                        target_path: self.source_path.clone(),
                        format: self.format,
                    };
                }
                Err(err) => self.status = format!("Flatten failed: {err}"),
            }
            return;
        }
        if !typing && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::C)) {
            match self.document.flatten() {
                Ok(img) => *action = EditorAction::CopyImage(img),
                Err(err) => self.status = format!("Flatten failed: {err}"),
            }
            return;
        }
        if typing {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && self.document.delete_selected() {
            self.drag = None;
            self.record_edit(HistoryEntry::DocumentEdit);
            self.status = "Deleted selected annotation.".into();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drag = None;
            self.document.select(None);
        }

        let shortcut_map = [
            (egui::Key::V, EditorTool::Select),
            (egui::Key::R, EditorTool::Rectangle),
            (egui::Key::F, EditorTool::FilledRectangle),
            (egui::Key::E, EditorTool::Ellipse),
            (egui::Key::A, EditorTool::Arrow),
            (egui::Key::L, EditorTool::Line),
            (egui::Key::D, EditorTool::Freehand),
            (egui::Key::H, EditorTool::Highlighter),
            (egui::Key::T, EditorTool::Text),
            (egui::Key::N, EditorTool::StepNumber),
            (egui::Key::P, EditorTool::Pixelate),
            (egui::Key::B, EditorTool::Blur),
            (egui::Key::Q, EditorTool::SpeechBalloon),
            (egui::Key::M, EditorTool::Magnifier),
            (egui::Key::S, EditorTool::Spotlight),
            (egui::Key::I, EditorTool::ImageOverlay),
            (egui::Key::X, EditorTool::SmartEraser),
            (egui::Key::C, EditorTool::Crop),
        ];
        for (key, tool) in shortcut_map {
            if ctx.input(|i| !i.modifiers.any() && i.key_pressed(key)) {
                self.active_tool = tool;
                self.drag = None;
            }
        }
    }

    fn render_tool_row(&mut self, ui: &mut egui::Ui, palette: &VoltShotPalette) {
        ui.horizontal_wrapped(|ui| {
            for tool in EditorTool::ALL {
                let selected = self.active_tool == tool;
                let text = format!("{} [{}]", tool.label(), tool.shortcut_hint());
                let btn =
                    egui::Button::new(egui::RichText::new(text).size(12.0).color(if selected {
                        Color32::WHITE
                    } else {
                        palette.text_primary
                    }))
                    .fill(if selected {
                        palette.accent
                    } else {
                        palette.panel_bg
                    });
                if ui.add(btn).clicked() {
                    self.active_tool = tool;
                    self.drag = None;
                }
            }
        });
    }

    fn render_options_and_actions_row(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        palette: &VoltShotPalette,
        action: &mut EditorAction,
    ) {
        ui.horizontal_wrapped(|ui| {
            for swatch in VOLTSHOT_PALETTE {
                let is_current = self.color == swatch;
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(3), to_egui_color(swatch));
                let border_stroke = if is_current {
                    Stroke::new(2.0_f32, Color32::WHITE)
                } else {
                    Stroke::new(1.0_f32, palette.border)
                };
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(3),
                    border_stroke,
                    egui::StrokeKind::Inside,
                );
                if resp.clicked() {
                    self.color = swatch;
                }
            }

            let mut custom = to_egui_color(self.color);
            if ui.color_edit_button_srgba(&mut custom).changed() {
                self.color = RgbaColor::rgba(custom.r(), custom.g(), custom.b(), custom.a());
            }

            ui.separator();

            match self.active_tool {
                EditorTool::Text | EditorTool::SpeechBalloon => {
                    ui.label("Text:");
                    ui.add(egui::TextEdit::singleline(&mut self.text_draft).desired_width(130.0));
                    ui.add(
                        egui::DragValue::new(&mut self.font_size)
                            .range(12.0..=64.0)
                            .prefix("Size "),
                    );
                    if self.active_tool == EditorTool::Text {
                        ui.checkbox(&mut self.text_background, "Bg");
                    }
                }
                EditorTool::Pixelate | EditorTool::Blur => {
                    ui.add(
                        egui::DragValue::new(&mut self.effect_strength)
                            .range(4..=32)
                            .prefix("Strength "),
                    );
                }
                EditorTool::StepNumber => {
                    ui.label(format!("Next step: #{}", self.document.next_step_number()));
                    ui.add(
                        egui::DragValue::new(&mut self.font_size)
                            .range(12.0..=42.0)
                            .prefix("Radius "),
                    );
                }
                EditorTool::Magnifier => {
                    ui.add(
                        egui::DragValue::new(&mut self.magnifier_zoom)
                            .range(2..=3)
                            .prefix("Magnification ")
                            .suffix("×"),
                    );
                }
                EditorTool::Spotlight => {
                    ui.add(egui::DragValue::new(&mut self.spotlight_alpha).prefix("Dim opacity "));
                }
                EditorTool::ImageOverlay => self.render_stamp_controls(ctx, ui),
                EditorTool::SmartEraser => {
                    ui.add(
                        egui::Slider::new(&mut self.stroke_width, 1.5..=64.0).text("Eraser width"),
                    );
                }
                _ => {
                    ui.add(
                        egui::DragValue::new(&mut self.stroke_width)
                            .range(1.0..=28.0)
                            .prefix("Width ")
                            .suffix("px"),
                    );
                }
            }

            ui.separator();

            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                .clicked()
            {
                self.step_history(ctx, false);
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                .clicked()
            {
                self.step_history(ctx, true);
            }
            if ui
                .add_enabled(
                    self.document.selected_id().is_some(),
                    egui::Button::new("Delete"),
                )
                .clicked()
                && self.document.delete_selected()
            {
                self.drag = None;
                self.record_edit(HistoryEntry::DocumentEdit);
            }
            if ui
                .add_enabled(
                    !self.document.annotations().is_empty(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.document.clear_annotations();
                self.drag = None;
                self.record_edit(HistoryEntry::DocumentEdit);
            }

            ui.separator();

            if ui.selectable_label(self.fit_to_view, "Fit").clicked() {
                self.fit_to_view = true;
            }
            if ui.button("100%").clicked() {
                self.fit_to_view = false;
                self.zoom = 1.0;
            }
            if ui.button("-").clicked() {
                self.fit_to_view = false;
                self.zoom = (self.zoom * 0.85).clamp(0.15, 6.0);
            }
            if ui.button("+").clicked() {
                self.fit_to_view = false;
                self.zoom = (self.zoom * 1.15).clamp(0.15, 6.0);
            }

            ui.separator();

            if ui.button("Copy Image").clicked() {
                match self.document.flatten() {
                    Ok(img) => *action = EditorAction::CopyImage(img),
                    Err(err) => self.status = format!("Flatten failed: {err}"),
                }
            }
            let save_btn = egui::Button::new(egui::RichText::new("Save").color(Color32::WHITE))
                .fill(palette.accent);
            if ui.add(save_btn).clicked() {
                match self.document.flatten() {
                    Ok(img) => {
                        *action = EditorAction::Save {
                            image: img,
                            target_path: self.source_path.clone(),
                            format: self.format,
                        };
                    }
                    Err(err) => self.status = format!("Flatten failed: {err}"),
                }
            }
            if ui.button("Save Copy").clicked() {
                match self.document.flatten() {
                    Ok(img) => {
                        *action = EditorAction::SaveAsCopy {
                            image: img,
                            format: self.format,
                        };
                    }
                    Err(err) => self.status = format!("Flatten failed: {err}"),
                }
            }
            if ui.button("Close").clicked() {
                *action = EditorAction::Close;
            }
        });
    }

    fn render_stamp_controls(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::singleline(&mut self.stamp_path)
                .hint_text("Stamp PNG path")
                .desired_width(135.0),
        );
        let image = if ui.button("Load PNG Stamp").clicked() {
            Some(load_png_stamp(Path::new(self.stamp_path.trim())))
        } else if ui.button("Stamp Sample Badge").clicked() {
            Some(sample_badge())
        } else {
            None
        };
        if let Some(image) = image {
            self.status = match image.and_then(|image| {
                let texture = rgba_texture(
                    ctx,
                    "stamp_preview",
                    image.width(),
                    image.height(),
                    image.rgba(),
                )
                .ok_or_else(|| "Image exceeds the texture size limit.".to_owned())?;
                Ok(OverlayStamp { image, texture })
            }) {
                Ok(stamp) => {
                    self.stamp = Some(stamp);
                    "Stamp ready. Drag on the canvas to place it.".into()
                }
                Err(err) => format!("Could not load stamp: {err}"),
            };
        }
        ui.add(egui::Slider::new(&mut self.stamp_opacity, 0.0..=1.0).text("Stamp opacity"));
    }

    fn render_project_controls(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Export .sxed")
                .on_hover_text("Saves an editable project containing the original unredacted base image and annotation layers. Use Save for a flattened redacted PNG.")
                .clicked()
            {
                self.status = match project_path(self.source_path.as_deref()).and_then(|path| {
                    save_project(&self.document, &path)?;
                    Ok(path)
                }) {
                    Ok(path) => format!("Exported project to {}.", path.display()),
                    Err(err) => format!("Project export failed: {err}"),
                };
            }
            if ui.button("Import .sxed").clicked() {
                self.status = match project_path(self.source_path.as_deref()).and_then(|path| {
                    let document = load_project(&path, self.document.base_image())?;
                    let base = document.base_image();
                    validate_canvas_size(
                        base.width(),
                        base.height(),
                        ctx.input(|input| input.max_texture_side),
                    )?;
                    self.replace_document(ctx, document);
                    Ok(path)
                }) {
                    Ok(path) => format!("Imported project from {}.", path.display()),
                    Err(err) => format!("Project import failed: {err}"),
                };
            }
            ui.menu_button("Resize Canvas", |ui| {
                ui.label("Anchor: top left. New space is transparent.");
                ui.horizontal(|ui| {
                    let limit = ctx.input(|input| input.max_texture_side) as u32;
                    ui.add(
                        egui::DragValue::new(&mut self.canvas_size[0])
                            .range(1..=limit)
                            .prefix("W "),
                    );
                    ui.label("x");
                    ui.add(
                        egui::DragValue::new(&mut self.canvas_size[1])
                            .range(1..=limit)
                            .prefix("H "),
                    );
                });
                if ui.button("Apply size").clicked() {
                    let [width, height] = self.canvas_size;
                    let limit = ctx.input(|input| input.max_texture_side);
                    self.status = match resize_canvas(&self.document, width, height, limit) {
                        Ok(Some(document)) => {
                            self.replace_document(ctx, document);
                            format!("Resized canvas to {width}x{height}.")
                        }
                        Ok(None) => "Canvas already has that size.".into(),
                        Err(err) => format!("Canvas resize failed: {err}"),
                    };
                    ui.close_menu();
                }
            });
        });
    }

    fn render_canvas(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, palette: &VoltShotPalette) {
        let img_w = self.document.base_image().width() as f32;
        let img_h = self.document.base_image().height() as f32;
        let avail = ui.available_size();
        let canvas_height = (avail.y - 26.0).max(260.0);

        if self.fit_to_view {
            let scale_x = ((avail.x - 24.0).max(100.0)) / img_w.max(1.0);
            let scale_y = ((canvas_height - 24.0).max(100.0)) / img_h.max(1.0);
            self.zoom = scale_x.min(scale_y).clamp(0.1, 4.0);
        }

        let zoom = self.zoom;
        let display_size = Vec2::new(img_w * zoom, img_h * zoom);

        egui::ScrollArea::both()
            .max_height(canvas_height)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(display_size, Sense::click_and_drag());
                let painter = ui.painter_at(rect);

                let to_screen =
                    |pt: Point| Pos2::new(rect.min.x + pt.x * zoom, rect.min.y + pt.y * zoom);
                let to_image = |pos: Pos2| {
                    Point::new(
                        ((pos.x - rect.min.x) / zoom).clamp(0.0, img_w),
                        ((pos.y - rect.min.y) / zoom).clamp(0.0, img_h),
                    )
                };

                self.handle_canvas_pointer(ctx, &response, &to_image);
                self.sync_overlay_textures(ctx);
                paint_texture(&painter, &self.texture, rect, 1.0);

                let mut selected_item = None;
                for original in self.document.annotations() {
                    let item = match &self.drag {
                        Some(DragDraft::ResizeSelected(resize)) if resize.id == original.id => {
                            resize.annotation()
                        }
                        _ => original,
                    };
                    let selected = self.active_tool == EditorTool::Select
                        && self.document.selected_id() == Some(original.id);
                    if selected {
                        selected_item = Some((original, item));
                    }
                    if let AnnotationKind::ImageOverlay { rect, opacity, .. } = &item.kind
                        && let Some(texture) = self.overlay_textures.get(&original.id)
                    {
                        paint_texture(
                            &painter,
                            texture,
                            rect_to_screen(*rect, &to_screen),
                            *opacity,
                        );
                    }
                    self.paint_magnifier_sample(&painter, &item.kind, rect, zoom);
                    paint_annotation(&painter, item, &to_screen, zoom, selected, palette.accent);
                }
                if let Some((original, item)) = selected_item
                    && self
                        .document
                        .hit_test_resize_handle(original.id, original.resize_handles()[0].1, 0.0)
                        .is_some()
                {
                    for (_, corner) in item.resize_handles() {
                        let handle = Rect::from_center_size(to_screen(corner), Vec2::splat(7.0));
                        painter.rect_filled(handle, 0.0, palette.accent);
                        painter.rect_stroke(
                            handle,
                            0.0,
                            Stroke::new(1.0_f32, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                if let Some(draft) = &self.drag {
                    self.paint_draft(&painter, draft, &to_screen, rect, zoom, palette.accent);
                }
            });
    }

    fn sync_overlay_textures(&mut self, ctx: &egui::Context) {
        let annotations = self.document.annotations();
        self.overlay_textures
            .retain(|id, _| annotations.iter().any(|item| item.id == *id));
        for item in annotations {
            if let AnnotationKind::ImageOverlay {
                width,
                height,
                rgba,
                ..
            } = &item.kind
                && let std::collections::hash_map::Entry::Vacant(entry) =
                    self.overlay_textures.entry(item.id)
                && let Some(texture) =
                    rgba_texture(ctx, "editor_image_overlay", *width, *height, rgba)
            {
                entry.insert(texture);
            }
        }
    }

    fn handle_canvas_pointer(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        to_image: &dyn Fn(Pos2) -> Point,
    ) {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match self.active_tool {
                EditorTool::Select => {
                    self.document.select_at(pt, 10.0 / self.zoom.max(0.25));
                }
                EditorTool::StepNumber => {
                    let num = self.document.next_step_number();
                    self.push_annotation(AnnotationKind::StepNumber {
                        center: pt,
                        number: num,
                        color: self.color,
                        radius: (self.font_size * 0.85).clamp(12.0, 36.0),
                    });
                    self.status = format!("Added step #{num}.");
                }
                EditorTool::Text => {
                    let text = self.text_draft.trim();
                    if !text.is_empty() {
                        self.push_annotation(AnnotationKind::Text {
                            position: pt,
                            text: text.to_owned(),
                            color: self.color,
                            font_size: self.font_size,
                            background: self
                                .text_background
                                .then_some(RgbaColor::rgba(20, 24, 30, 210)),
                        });
                        self.status = "Placed text label.".into();
                    }
                }
                EditorTool::Magnifier => {
                    self.push_annotation(AnnotationKind::Magnifier {
                        center: pt,
                        radius: 40.0,
                        zoom: f32::from(self.magnifier_zoom),
                        border_color: self.color,
                    });
                }
                _ => {}
            }
        }

        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            let start = ctx
                .input(|input| input.pointer.press_origin())
                .map_or(pt, to_image);
            match self.active_tool {
                EditorTool::Select => {
                    if let Some(resize) = ResizeDrag::start(&self.document, start, 7.0 / self.zoom)
                    {
                        self.drag = Some(DragDraft::ResizeSelected(Box::new(resize)));
                    } else if let Some(id) =
                        self.document.select_at(start, 10.0 / self.zoom.max(0.25))
                    {
                        self.drag = Some(DragDraft::MoveSelected {
                            id,
                            last: start,
                            recorded: false,
                        });
                    }
                }
                EditorTool::Freehand | EditorTool::Highlighter => {
                    self.drag = Some(DragDraft::Freehand {
                        points: vec![start],
                    });
                }
                EditorTool::SmartEraser => {
                    let flattened = self.document.flatten().ok();
                    let color = sample_pixel(
                        flattened
                            .as_ref()
                            .unwrap_or_else(|| self.document.base_image()),
                        start,
                    );
                    self.drag = Some(DragDraft::SmartEraser {
                        points: vec![start],
                        stroke_width: self.stroke_width,
                        color,
                    });
                }
                EditorTool::ImageOverlay if self.stamp.is_none() => {
                    self.status = "Load a PNG stamp or choose the sample badge first.".into();
                }
                EditorTool::Rectangle
                | EditorTool::FilledRectangle
                | EditorTool::Ellipse
                | EditorTool::Arrow
                | EditorTool::Line
                | EditorTool::Pixelate
                | EditorTool::Blur
                | EditorTool::SpeechBalloon
                | EditorTool::Magnifier
                | EditorTool::Spotlight
                | EditorTool::ImageOverlay
                | EditorTool::Crop => {
                    self.drag = Some(DragDraft::Shape { start, current: pt });
                }
                EditorTool::Text | EditorTool::StepNumber => {}
            }
        }

        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            let mut moved = false;
            match &mut self.drag {
                Some(DragDraft::Shape { current, .. }) => {
                    *current = pt;
                }
                Some(DragDraft::Freehand { points } | DragDraft::SmartEraser { points, .. }) => {
                    append_stroke_point(points, pt, 1.5);
                }
                Some(DragDraft::ResizeSelected(resize)) => resize.update(pt),
                Some(DragDraft::MoveSelected { id, last, recorded }) => {
                    let dx = pt.x - last.x;
                    let dy = pt.y - last.y;
                    if dx != 0.0 || dy != 0.0 {
                        if !*recorded {
                            self.document.begin_move_undo();
                            *recorded = true;
                            moved = true;
                        }
                        self.document.translate_annotation(*id, dx, dy);
                        *last = pt;
                    }
                }
                None => {}
            }
            if moved {
                self.record_edit(HistoryEntry::DocumentEdit);
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(draft) = self.drag.take()
        {
            let end = response.interact_pointer_pos().map(to_image);
            match draft {
                DragDraft::MoveSelected { recorded, .. } => {
                    if recorded {
                        self.status = "Moved annotation.".into();
                    }
                }
                DragDraft::ResizeSelected(mut resize) => {
                    if let Some(end) = end {
                        resize.update(end);
                    }
                    if resize.commit(&mut self.document) {
                        self.record_edit(HistoryEntry::DocumentEdit);
                        self.status = "Resized annotation.".into();
                    }
                }
                DragDraft::SmartEraser {
                    mut points,
                    stroke_width,
                    color,
                } => {
                    if let Some(end) = end {
                        append_stroke_point(&mut points, end, 0.1);
                    }
                    self.push_annotation(AnnotationKind::SmartEraser {
                        points,
                        stroke_width,
                        sample_color: Some(color),
                    });
                }
                DragDraft::Freehand { mut points } => {
                    if let Some(end) = end {
                        append_stroke_point(&mut points, end, 0.1);
                    }
                    if points.len() >= 2 {
                        let kind = if self.active_tool == EditorTool::Highlighter {
                            AnnotationKind::Highlighter {
                                points,
                                color: self.color,
                                stroke_width: (self.stroke_width * 3.0).max(14.0),
                            }
                        } else {
                            AnnotationKind::Freehand {
                                points,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            }
                        };
                        self.push_annotation(kind);
                    }
                }
                DragDraft::Shape { start, current } => {
                    let end = end.unwrap_or(current);
                    if let Some(kind) = self.new_tool_annotation(start, end) {
                        self.push_annotation(kind);
                        return;
                    }
                    let rect = RectF::from_points(start, end);
                    if rect.width < 3.0 && rect.height < 3.0 {
                        return;
                    }
                    match self.active_tool {
                        EditorTool::Rectangle => {
                            self.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: false,
                            });
                        }
                        EditorTool::FilledRectangle => {
                            self.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: true,
                            });
                        }
                        EditorTool::Ellipse => {
                            self.push_annotation(AnnotationKind::Ellipse {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Line => {
                            self.push_annotation(AnnotationKind::Line {
                                start,
                                end,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Arrow => {
                            self.push_annotation(AnnotationKind::Arrow {
                                start,
                                end,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Pixelate => {
                            self.push_annotation(AnnotationKind::Pixelate {
                                rect,
                                block_size: self.effect_strength,
                            });
                        }
                        EditorTool::Blur => {
                            self.push_annotation(AnnotationKind::Blur {
                                rect,
                                radius: (self.effect_strength / 2).max(2),
                            });
                        }
                        EditorTool::Crop => {
                            let x = rect.x.floor().max(0.0) as i32;
                            let y = rect.y.floor().max(0.0) as i32;
                            let max_w = self.document.base_image().width().saturating_sub(x as u32);
                            let max_h =
                                self.document.base_image().height().saturating_sub(y as u32);
                            let w = (rect.width.round() as u32).min(max_w);
                            let h = (rect.height.round() as u32).min(max_h);
                            if let Ok(region) = Region::new(x, y, w, h) {
                                match self.document.crop(region) {
                                    Ok(()) => {
                                        self.record_edit(HistoryEntry::DocumentEdit);
                                        self.reload_texture(ctx);
                                        self.status = format!("Cropped to {}x{}.", w, h);
                                    }
                                    Err(err) => self.status = format!("Crop failed: {err}"),
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn new_tool_annotation(&self, start: Point, end: Point) -> Option<AnnotationKind> {
        let rect = RectF::from_points(start, end);
        match self.active_tool {
            EditorTool::SpeechBalloon if rect.width >= 3.0 && rect.height >= 3.0 => {
                let tail = Point::new(
                    rect.x + rect.width * 0.25,
                    rect.bottom() + (rect.height * 0.4).clamp(12.0, 28.0),
                );
                Some(AnnotationKind::SpeechBalloon {
                    rect,
                    tail,
                    text: self.text_draft.trim().to_owned(),
                    fill: RgbaColor::WHITE,
                    border: self.color,
                    text_color: RgbaColor::BLACK,
                    font_size: self.font_size,
                })
            }
            EditorTool::Magnifier if rect.width >= 3.0 || rect.height >= 3.0 => {
                Some(AnnotationKind::Magnifier {
                    center: start,
                    radius: rect.width.hypot(rect.height),
                    zoom: f32::from(self.magnifier_zoom),
                    border_color: self.color,
                })
            }
            EditorTool::Spotlight if rect.width >= 3.0 && rect.height >= 3.0 => {
                Some(AnnotationKind::Spotlight {
                    rect,
                    dim_alpha: self.spotlight_alpha,
                })
            }
            EditorTool::ImageOverlay if rect.width >= 1.0 && rect.height >= 1.0 => self
                .stamp
                .as_ref()
                .map(|stamp| AnnotationKind::ImageOverlay {
                    rect,
                    width: stamp.image.width(),
                    height: stamp.image.height(),
                    rgba: stamp.image.rgba().to_vec(),
                    opacity: self.stamp_opacity,
                }),
            _ => None,
        }
    }

    fn paint_magnifier_sample(
        &self,
        painter: &egui::Painter,
        kind: &AnnotationKind,
        canvas: Rect,
        editor_zoom: f32,
    ) {
        let AnnotationKind::Magnifier {
            center,
            radius,
            zoom,
            ..
        } = kind
        else {
            return;
        };
        let radius = *radius * editor_zoom;
        if radius <= 0.0 || !radius.is_finite() {
            return;
        }
        let center_pos = canvas.min + Vec2::new(center.x, center.y) * editor_zoom;
        let mut mesh = egui::Mesh::with_texture(self.texture.id());
        let mut add_vertex = |offset: Vec2| {
            let source = center_pos - canvas.min + offset / zoom.max(1.0);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: center_pos + offset,
                uv: Pos2::new(source.x / canvas.width(), source.y / canvas.height()),
                color: Color32::WHITE,
            });
        };
        add_vertex(Vec2::ZERO);
        let segments = 64;
        for i in 0..segments {
            let angle = i as f32 * std::f32::consts::TAU / segments as f32;
            let (sin, cos) = angle.sin_cos();
            add_vertex(Vec2::new(cos, sin) * radius);
        }
        for i in 0..segments {
            mesh.add_triangle(0, i + 1, (i + 1) % segments + 1);
        }
        painter.add(mesh);
    }

    fn paint_draft(
        &self,
        painter: &egui::Painter,
        draft: &DragDraft,
        to_screen: &dyn Fn(Point) -> Pos2,
        canvas_rect: Rect,
        zoom: f32,
        accent: Color32,
    ) {
        match draft {
            DragDraft::MoveSelected { .. } | DragDraft::ResizeSelected(_) => {}
            DragDraft::SmartEraser {
                points,
                stroke_width,
                color,
            } => {
                paint_eraser_stroke(painter, points, to_screen, zoom, *stroke_width, *color);
            }
            DragDraft::Freehand { points } => {
                if points.len() >= 2 {
                    let screen_pts: Vec<Pos2> = points.iter().copied().map(to_screen).collect();
                    let (col, width) = if self.active_tool == EditorTool::Highlighter {
                        (
                            to_egui_color(self.color.with_alpha(96)),
                            (self.stroke_width * 3.0).max(14.0) * zoom,
                        )
                    } else {
                        (
                            to_egui_color(self.color),
                            (self.stroke_width * zoom).max(1.0),
                        )
                    };
                    painter.add(egui::Shape::line(screen_pts, Stroke::new(width, col)));
                }
            }
            DragDraft::Shape { start, current } => {
                if self.active_tool == EditorTool::ImageOverlay {
                    if let Some(stamp) = &self.stamp {
                        let rect = rect_to_screen(RectF::from_points(*start, *current), to_screen);
                        paint_texture(painter, &stamp.texture, rect, self.stamp_opacity);
                    }
                    return;
                }
                if let Some(kind) = self.new_tool_annotation(*start, *current) {
                    self.paint_magnifier_sample(painter, &kind, canvas_rect, zoom);
                    let draft = Annotation { id: 0, kind };
                    paint_annotation(painter, &draft, to_screen, zoom, false, accent);
                    return;
                }
                let r = RectF::from_points(*start, *current);
                let sr = Rect::from_min_max(
                    to_screen(Point::new(r.x, r.y)),
                    to_screen(Point::new(r.right(), r.bottom())),
                );
                let col = to_egui_color(self.color);
                let sw = (self.stroke_width * zoom).max(1.5);

                match self.active_tool {
                    EditorTool::Rectangle => {
                        painter.rect_stroke(
                            sr,
                            0.0,
                            Stroke::new(sw, col),
                            egui::StrokeKind::Middle,
                        );
                    }
                    EditorTool::FilledRectangle => {
                        painter.rect_filled(sr, 0.0, col);
                    }
                    EditorTool::Ellipse => {
                        paint_ellipse_stroke(painter, sr, Stroke::new(sw, col));
                    }
                    EditorTool::Line => {
                        painter.line_segment(
                            [to_screen(*start), to_screen(*current)],
                            Stroke::new(sw, col),
                        );
                    }
                    EditorTool::Arrow => {
                        paint_arrow(painter, to_screen(*start), to_screen(*current), col, sw);
                    }
                    EditorTool::Pixelate | EditorTool::Blur => {
                        painter.rect_filled(
                            sr,
                            0.0,
                            Color32::from_rgba_unmultiplied(59, 130, 246, 55),
                        );
                        painter.rect_stroke(
                            sr,
                            0.0,
                            Stroke::new(1.5_f32, accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                    EditorTool::Crop => {
                        let dim = Color32::from_black_alpha(140);
                        painter.rect_filled(
                            Rect::from_min_max(
                                canvas_rect.min,
                                Pos2::new(canvas_rect.max.x, sr.min.y),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(canvas_rect.min.x, sr.max.y),
                                canvas_rect.max,
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(canvas_rect.min.x, sr.min.y),
                                Pos2::new(sr.min.x, sr.max.y),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(sr.max.x, sr.min.y),
                                Pos2::new(canvas_rect.max.x, sr.max.y),
                            ),
                            0.0,
                            dim,
                        );
                        painter.rect_stroke(
                            sr,
                            0.0,
                            Stroke::new(2.0_f32, accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

struct ResizeDrag {
    id: u64,
    handle: ResizeHandle,
    press: Point,
    corner: Point,
    target: Point,
    preview: EditorDocument,
    preview_id: u64,
    changed: bool,
}

impl ResizeDrag {
    fn start(document: &EditorDocument, press: Point, tolerance: f32) -> Option<Self> {
        let id = document.selected_id()?;
        let handle = document.hit_test_resize_handle(id, press, tolerance)?;
        let annotation = document.annotations().iter().find(|item| item.id == id)?;
        let corner = annotation
            .resize_handles()
            .into_iter()
            .find_map(|(candidate, corner)| (candidate == handle).then_some(corner))?;
        let mut preview = EditorDocument::new(CapturedImage::from_rgba(1, 1, vec![0; 4]).ok()?);
        let preview_id = preview.push_annotation(annotation.kind.clone());
        Some(Self {
            id,
            handle,
            press,
            corner,
            target: corner,
            preview,
            preview_id,
            changed: false,
        })
    }

    fn update(&mut self, point: Point) {
        let target = Point::new(
            self.corner.x + (point.x - self.press.x),
            self.corner.y + (point.y - self.press.y),
        );
        if target == self.target {
            return;
        }
        if self.changed {
            self.preview.undo();
        }
        self.target = target;
        self.changed = self
            .preview
            .resize_annotation(self.preview_id, self.handle, target);
    }

    fn annotation(&self) -> &Annotation {
        &self.preview.annotations()[0]
    }

    fn commit(&self, document: &mut EditorDocument) -> bool {
        self.changed && document.resize_annotation(self.id, self.handle, self.target)
    }
}

fn project_path(source: Option<&Path>) -> Result<PathBuf, String> {
    source
        .map(|path| path.with_extension("sxed"))
        .ok_or_else(|| "Save the image first to choose a project location.".into())
}

fn save_project(document: &EditorDocument, path: &Path) -> Result<(), String> {
    let png = document
        .base_image()
        .encode(ImageFormat::Png)
        .map_err(|err| err.to_string())?;
    let annotations = document
        .export_annotations_json()
        .map_err(|err| err.to_string())?;
    let mut json = String::from("{\n  \"base_png_hex_v1\": \"");
    let digits = b"0123456789abcdef";
    for byte in png {
        json.push(char::from(digits[usize::from(byte >> 4)]));
        json.push(char::from(digits[usize::from(byte & 15)]));
    }
    json.push_str("\",\n  \"annotations\": ");
    json.push_str(&annotations);
    json.push_str("\n}\n");
    std::fs::write(path, json).map_err(|err| err.to_string())
}

fn load_project(path: &Path, fallback_base: &CapturedImage) -> Result<EditorDocument, String> {
    let json = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let json = json.trim();
    if json.starts_with('[') {
        return document_with_layers(fallback_base.clone(), json);
    }
    let invalid = || "Invalid or unsupported .sxed project.".to_owned();
    let (encoded, layers) = json
        .strip_prefix('{')
        .and_then(|body| body.strip_suffix('}'))
        .and_then(|body| project_value(body, "\"base_png_hex_v1\""))
        .and_then(|value| value.strip_prefix('"'))
        .and_then(|value| value.split_once('"'))
        .ok_or_else(invalid)?;
    let layers = layers
        .trim_start()
        .strip_prefix(',')
        .and_then(|value| project_value(value, "\"annotations\""))
        .ok_or_else(invalid)?;
    let pairs = encoded.as_bytes().chunks_exact(2);
    if !pairs.remainder().is_empty() {
        return Err(invalid());
    }
    let mut png = Vec::with_capacity(encoded.len() / 2);
    for pair in pairs {
        let high = char::from(pair[0]).to_digit(16).ok_or_else(invalid)?;
        let low = char::from(pair[1]).to_digit(16).ok_or_else(invalid)?;
        png.push((high * 16 + low) as u8);
    }
    document_with_layers(decode_png(&png)?, layers)
}

fn project_value<'json>(json: &'json str, key: &str) -> Option<&'json str> {
    Some(
        json.trim_start()
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix(':')?
            .trim_start(),
    )
}

fn document_with_layers(base: CapturedImage, annotations: &str) -> Result<EditorDocument, String> {
    let mut document = EditorDocument::new(base);
    document
        .import_annotations_json(annotations)
        .map_err(|err| err.to_string())?;
    Ok(document)
}

fn validate_canvas_size(width: u32, height: u32, limit: usize) -> Result<(), String> {
    if width == 0
        || height == 0
        || width as usize > limit
        || height as usize > limit
        || u64::from(width) * u64::from(height) > 64 * 1024 * 1024
    {
        return Err(format!(
            "Canvas must be at most {limit}px per side and 64 megapixels."
        ));
    }
    Ok(())
}

fn resize_canvas(
    document: &EditorDocument,
    width: u32,
    height: u32,
    limit: usize,
) -> Result<Option<EditorDocument>, String> {
    validate_canvas_size(width, height, limit)?;
    let original = document.base_image();
    if (width, height) == (original.width(), original.height()) {
        return Ok(None);
    }
    let mut canvas = vec![0; width as usize * height as usize * 4];
    let source_stride = original.width() as usize * 4;
    let target_stride = width as usize * 4;
    let copy_bytes = source_stride.min(target_stride);
    for (source, target) in original
        .rgba()
        .chunks_exact(source_stride)
        .zip(canvas.chunks_exact_mut(target_stride))
    {
        target[..copy_bytes].copy_from_slice(&source[..copy_bytes]);
    }
    let base = CapturedImage::from_rgba(width, height, canvas).map_err(|err| err.to_string())?;
    let layers = document
        .export_annotations_json()
        .map_err(|err| err.to_string())?;
    let mut replacement = document_with_layers(base, &layers)?;
    replacement.select(document.selected_id());
    Ok(Some(replacement))
}

fn load_png_stamp(path: &Path) -> Result<CapturedImage, String> {
    let contents = std::fs::read(path).map_err(|err| err.to_string())?;
    decode_png(&contents)
}

fn decode_png(contents: &[u8]) -> Result<CapturedImage, String> {
    if !contents.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Expected a PNG image.".into());
    }
    CapturedImage::from_encoded_bytes(contents).map_err(|err| err.to_string())
}

fn sample_badge() -> Result<CapturedImage, String> {
    let width = 48;
    let height = 48;
    let mut rgba = Vec::new();
    for row in 0..height {
        for column in 0..width {
            let x = f64::from(column) - 23.5;
            let y = f64::from(row) - 23.5;
            let distance = x.hypot(y);
            let pixel = if distance > 22.0 {
                [0, 0, 0, 0]
            } else if distance > 19.5
                || ((x - y).abs() < 2.5 || (x + y).abs() < 2.5) && distance < 13.0
            {
                [255, 255, 255, 255]
            } else {
                [56, 130, 246, 255]
            };
            rgba.extend(pixel);
        }
    }
    CapturedImage::from_rgba(width, height, rgba).map_err(|err| err.to_string())
}

fn sample_pixel(image: &CapturedImage, point: Point) -> RgbaColor {
    let column = (point.x as u32).min(image.width() - 1) as usize;
    let row = (point.y as u32).min(image.height() - 1) as usize;
    let start = (row * image.width() as usize + column) * 4;
    let pixel = &image.rgba()[start..start + 4];
    if pixel[3] == 0 {
        RgbaColor::BLACK
    } else {
        RgbaColor::rgb(pixel[0], pixel[1], pixel[2])
    }
}

fn append_stroke_point(points: &mut Vec<Point>, point: Point, spacing: f32) {
    if points
        .last()
        .is_none_or(|last| (last.x - point.x).hypot(last.y - point.y) >= spacing)
    {
        points.push(point);
    }
}

fn paint_eraser_stroke(
    painter: &egui::Painter,
    points: &[Point],
    to_screen: &dyn Fn(Point) -> Pos2,
    zoom: f32,
    width: f32,
    color: RgbaColor,
) {
    if !width.is_finite()
        || width <= 0.0
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return;
    }
    let width = width.max(1.5) * zoom;
    let color = to_egui_color(color.with_alpha(255));
    let stroke = Stroke::new(width, color);
    for segment in points.windows(2) {
        painter.line_segment([to_screen(segment[0]), to_screen(segment[1])], stroke);
    }
    for &point in points {
        painter.circle_filled(to_screen(point), width * 0.5, color);
    }
}

fn rgba_texture(
    ctx: &egui::Context,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Option<egui::TextureHandle> {
    let [width, height] = [width as usize, height as usize];
    let limit = ctx.input(|input| input.max_texture_side);
    if width == 0
        || height == 0
        || width > limit
        || height > limit
        || width.checked_mul(height)?.checked_mul(4)? != rgba.len()
    {
        return None;
    }
    Some(ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([width, height], rgba),
        egui::TextureOptions::LINEAR,
    ))
}

fn paint_texture(painter: &egui::Painter, texture: &egui::TextureHandle, rect: Rect, opacity: f32) {
    if rect.is_positive() && rect.is_finite() {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0).round() as u8),
        );
    }
}

pub(crate) fn paint_annotation(
    painter: &egui::Painter,
    annotation: &Annotation,
    to_screen: &dyn Fn(Point) -> Pos2,
    zoom: f32,
    selected: bool,
    accent: Color32,
) {
    match &annotation.kind {
        AnnotationKind::Rectangle {
            rect,
            color,
            stroke_width,
            filled,
        } => {
            let sr = rect_to_screen(*rect, to_screen);
            let col = to_egui_color(*color);
            if *filled {
                painter.rect_filled(sr, 0.0, col);
            } else {
                painter.rect_stroke(
                    sr,
                    0.0,
                    Stroke::new((*stroke_width * zoom).max(1.0), col),
                    egui::StrokeKind::Inside,
                );
            }
        }
        AnnotationKind::Ellipse {
            rect,
            color,
            stroke_width,
        } => {
            let sr = rect_to_screen(*rect, to_screen);
            paint_ellipse_stroke(
                painter,
                sr,
                Stroke::new((*stroke_width * zoom).max(1.0), to_egui_color(*color)),
            );
        }
        AnnotationKind::Line {
            start,
            end,
            color,
            stroke_width,
        } => {
            painter.line_segment(
                [to_screen(*start), to_screen(*end)],
                Stroke::new((*stroke_width * zoom).max(1.0), to_egui_color(*color)),
            );
        }
        AnnotationKind::Arrow {
            start,
            end,
            color,
            stroke_width,
        } => {
            paint_arrow(
                painter,
                to_screen(*start),
                to_screen(*end),
                to_egui_color(*color),
                (*stroke_width * zoom).max(1.5),
            );
        }
        AnnotationKind::Freehand {
            points,
            color,
            stroke_width,
        } => {
            if points.len() >= 2 {
                let pts: Vec<Pos2> = points.iter().copied().map(to_screen).collect();
                painter.add(egui::Shape::line(
                    pts,
                    Stroke::new((*stroke_width * zoom).max(1.0), to_egui_color(*color)),
                ));
            }
        }
        AnnotationKind::Highlighter {
            points,
            color,
            stroke_width,
        } => {
            if points.len() >= 2 {
                let pts: Vec<Pos2> = points.iter().copied().map(to_screen).collect();
                painter.add(egui::Shape::line(
                    pts,
                    Stroke::new(
                        (*stroke_width * zoom).max(4.0),
                        to_egui_color(color.with_alpha(96)),
                    ),
                ));
            }
        }
        AnnotationKind::Text {
            position,
            text,
            color,
            font_size,
            background,
        } => {
            let pos = to_screen(*position);
            let font = FontId::proportional((*font_size * zoom).max(10.0));
            let galley = painter.layout_no_wrap(text.clone(), font, to_egui_color(*color));
            if let Some(bg) = background {
                let bg_rect = Rect::from_min_size(
                    pos - Vec2::new(4.0, 3.0),
                    galley.size() + Vec2::new(8.0, 6.0),
                );
                painter.rect_filled(bg_rect, CornerRadius::same(3), to_egui_color(*bg));
            }
            painter.galley(pos, galley, to_egui_color(*color));
        }
        AnnotationKind::StepNumber {
            center,
            number,
            color,
            radius,
        } => {
            let c = to_screen(*center);
            let r = (*radius * zoom).max(8.0);
            painter.circle_filled(c, r + 2.0, Color32::WHITE);
            painter.circle_filled(c, r, to_egui_color(*color));
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                FontId::proportional((r * 1.05).max(10.0)),
                Color32::WHITE,
            );
        }
        AnnotationKind::Pixelate { rect, block_size } => {
            let sr = rect_to_screen(*rect, to_screen);
            painter.rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(30, 41, 59, 175));
            painter.rect_stroke(
                sr,
                0.0,
                Stroke::new(1.5_f32, Color32::from_rgb(148, 163, 184)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                sr.center(),
                egui::Align2::CENTER_CENTER,
                format!("PIXELATE ({block_size}px)"),
                FontId::monospace(11.0),
                Color32::WHITE,
            );
        }
        AnnotationKind::Blur { rect, radius } => {
            let sr = rect_to_screen(*rect, to_screen);
            painter.rect_filled(sr, 0.0, Color32::from_rgba_unmultiplied(30, 58, 138, 135));
            painter.rect_stroke(
                sr,
                0.0,
                Stroke::new(1.5_f32, Color32::from_rgb(147, 197, 253)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                sr.center(),
                egui::Align2::CENTER_CENTER,
                format!("BLUR (r={radius})"),
                FontId::monospace(11.0),
                Color32::WHITE,
            );
        }
        AnnotationKind::SpeechBalloon {
            rect,
            tail,
            text,
            fill,
            border,
            text_color,
            font_size,
        } => {
            let body = rect_to_screen(*rect, to_screen);
            let (outline, tail_tip) = speech_balloon_outline(*rect, *tail);
            let points: Vec<_> = outline.into_iter().map(to_screen).collect();
            let fill = to_egui_color(*fill);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(body.center(), fill);
            for point in &points {
                mesh.colored_vertex(*point, fill);
            }
            for index in 0..points.len() {
                if tail_tip == Some(index) {
                    continue;
                }
                let mut next = (index + 1) % points.len();
                if tail_tip == Some(next) {
                    mesh.add_triangle(index as u32 + 1, next as u32 + 1, next as u32 + 2);
                    next += 1;
                }
                mesh.add_triangle(0, index as u32 + 1, next as u32 + 1);
            }
            painter.add(mesh);
            painter.add(egui::Shape::closed_line(
                points,
                Stroke::new(2.0 * zoom, to_egui_color(*border)),
            ));
            let text_rect = body.shrink(6.0 * zoom);
            if text_rect.is_positive() {
                painter.with_clip_rect(text_rect).text(
                    text_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    text,
                    FontId::proportional(
                        (*font_size * zoom).clamp(1.0, text_rect.height().max(1.0)),
                    ),
                    to_egui_color(*text_color),
                );
            }
        }
        AnnotationKind::Magnifier {
            center,
            radius,
            border_color,
            ..
        } => {
            let outer_radius = *radius * zoom;
            if outer_radius <= 0.0 || !outer_radius.is_finite() {
                return;
            }
            let border_width = (2.0 * zoom).min(outer_radius);
            painter.circle_stroke(
                to_screen(*center),
                outer_radius - border_width * 0.5,
                Stroke::new(border_width, to_egui_color(*border_color)),
            );
        }
        AnnotationKind::Spotlight { rect, dim_alpha } => {
            let opening = rect_to_screen(*rect, to_screen);
            let visible = painter.clip_rect();
            if !visible.is_positive() {
                return;
            }
            let left = opening.left().clamp(visible.left(), visible.right());
            let right = opening.right().clamp(visible.left(), visible.right());
            let top = opening.top().clamp(visible.top(), visible.bottom());
            let bottom = opening.bottom().clamp(visible.top(), visible.bottom());
            let dim = Color32::from_black_alpha(*dim_alpha);
            for rect in [
                Rect::from_min_max(visible.min, Pos2::new(visible.right(), top)),
                Rect::from_min_max(Pos2::new(visible.left(), bottom), visible.max),
                Rect::from_min_max(Pos2::new(visible.left(), top), Pos2::new(left, bottom)),
                Rect::from_min_max(Pos2::new(right, top), Pos2::new(visible.right(), bottom)),
            ] {
                painter.rect_filled(rect, 0.0, dim);
            }
        }
        AnnotationKind::SmartEraser {
            points,
            stroke_width,
            sample_color: Some(color),
        } => {
            paint_eraser_stroke(painter, points, to_screen, zoom, *stroke_width, *color);
        }
        AnnotationKind::ImageOverlay { .. } | AnnotationKind::SmartEraser { .. } => {}
    }

    if selected {
        let b = rect_to_screen(annotation.bounds(), to_screen).expand(4.0);
        painter.rect_stroke(
            b,
            0.0,
            Stroke::new(1.5_f32, accent),
            egui::StrokeKind::Outside,
        );
    }
}

fn rect_to_screen(r: RectF, to_screen: &dyn Fn(Point) -> Pos2) -> Rect {
    Rect::from_min_max(
        to_screen(Point::new(r.x, r.y)),
        to_screen(Point::new(r.right(), r.bottom())),
    )
}

pub(crate) fn paint_arrow(
    painter: &egui::Painter,
    start: Pos2,
    end: Pos2,
    color: Color32,
    width: f32,
) {
    let delta = end - start;
    let len = delta.length();
    if len < 1.0 {
        return;
    }
    let dir = delta / len;
    let head_len = (width * 3.5).max(12.0).min(len * 0.65);
    let head_half = head_len * 0.55;
    let base = end - dir * head_len;
    let perp = Vec2::new(-dir.y, dir.x);
    let left = base + perp * head_half;
    let right = base - perp * head_half;

    painter.line_segment([start, base], Stroke::new(width, color));
    painter.add(egui::Shape::convex_polygon(
        vec![end, left, right],
        color,
        Stroke::NONE,
    ));
}

pub(crate) fn paint_ellipse_stroke(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let center = rect.center();
    let rx = rect.width() * 0.5;
    let ry = rect.height() * 0.5;
    let segments = 48;
    let mut pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
        pts.push(Pos2::new(
            center.x + rx * angle.cos(),
            center.y + ry * angle.sin(),
        ));
    }
    painter.add(egui::Shape::line(pts, stroke));
}

pub(crate) fn to_egui_color(c: RgbaColor) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn load_editor_texture(
    ctx: &egui::Context,
    image: &CapturedImage,
    version: u64,
) -> egui::TextureHandle {
    let color =
        crate::image_texture::bounded_color_image(image, ctx.input(|input| input.max_texture_side));
    ctx.load_texture(
        format!("voltshot_editor_canvas_{version}"),
        color,
        egui::TextureOptions::LINEAR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> CapturedImage {
        CapturedImage::from_rgba(24, 20, [12, 34, 56, 255].repeat(24 * 20)).unwrap()
    }

    fn test_rectangle() -> AnnotationKind {
        AnnotationKind::Rectangle {
            rect: RectF::from_points(Point::new(4.0, 5.0), Point::new(12.0, 14.0)),
            color: RgbaColor::RED,
            stroke_width: 2.0,
            filled: true,
        }
    }

    #[test]
    fn sxed_project_round_trips_resized_canvas_and_layers() {
        let mut source = EditorDocument::new(test_image());
        source.push_annotation(test_rectangle());
        let stamp = sample_badge().unwrap();
        source.push_annotation(AnnotationKind::ImageOverlay {
            rect: RectF::from_points(Point::new(5.0, 2.0), Point::new(17.0, 14.0)),
            width: stamp.width(),
            height: stamp.height(),
            rgba: stamp.rgba().to_vec(),
            opacity: 0.6,
        });
        let resized = resize_canvas(&source, 28, 17, 64).unwrap().unwrap();
        assert_eq!(
            sample_pixel(resized.base_image(), Point::new(0.0, 0.0)),
            RgbaColor::rgb(12, 34, 56)
        );
        assert_eq!(&resized.base_image().rgba()[27 * 4..28 * 4], &[0; 4]);

        let dir = tempfile::tempdir().unwrap();
        let path = project_path(Some(&dir.path().join("capture.png"))).unwrap();
        assert_eq!(path.file_name().unwrap(), "capture.sxed");
        assert!(project_path(None).is_err());
        save_project(&resized, &path).unwrap();
        let loaded = load_project(&path, source.base_image()).unwrap();
        assert_eq!(loaded.base_image(), resized.base_image());
        assert_eq!(loaded.annotations(), source.annotations());
        assert_eq!(loaded.flatten().unwrap(), resized.flatten().unwrap());

        std::fs::write(&path, source.export_annotations_json().unwrap()).unwrap();
        let legacy = load_project(&path, source.base_image()).unwrap();
        assert_eq!(legacy.base_image(), source.base_image());
        assert_eq!(legacy.annotations(), source.annotations());
        std::fs::write(
            &path,
            "{\"base_png_hex_v1\":\"not a png\",\"annotations\":[]}",
        )
        .unwrap();
        assert!(load_project(&path, source.base_image()).is_err());
    }

    #[test]
    fn corner_drag_previews_from_the_original_and_commits_once() {
        let mut document = EditorDocument::new(test_image());
        let id = document.push_annotation(test_rectangle());
        let original = document.annotations()[0].clone();
        let near_corner = Point::new(13.0, 15.0);
        let mut drag = ResizeDrag::start(&document, near_corner, 2.0).unwrap();
        assert_eq!(drag.handle, ResizeHandle::BottomRight);
        assert!(!drag.commit(&mut document));
        drag.update(Point::new(15.0, 16.0));
        drag.update(near_corner);
        assert_eq!(drag.annotation().kind, original.kind);
        assert!(!drag.commit(&mut document));
        drag.update(Point::new(18.0, 19.0));
        assert_eq!(
            drag.annotation().bounds(),
            RectF::from_points(Point::new(4.0, 5.0), Point::new(17.0, 18.0))
        );
        assert_eq!(document.annotations(), std::slice::from_ref(&original));

        assert!(drag.commit(&mut document));
        assert_eq!(document.annotations()[0].id, id);
        let final_kind = document.annotations()[0].kind.clone();
        assert!(document.undo());
        assert_eq!(document.annotations(), [original]);
        assert!(document.redo());
        assert_eq!(document.annotations()[0].kind, final_kind);
    }

    #[test]
    fn canvas_replacement_keeps_history_on_both_sides() {
        let ctx = egui::Context::default();
        let mut editor = EditorViewState::open(&ctx, test_image(), None, ImageFormat::Png);
        editor.push_annotation(test_rectangle());
        let resized = resize_canvas(&editor.document, 28, 25, 64)
            .unwrap()
            .unwrap();
        editor.replace_document(&ctx, resized);
        editor.push_annotation(test_rectangle());
        editor.step_history(&ctx, false);
        assert_eq!(editor.document.base_image().width(), 28);
        assert_eq!(editor.document.annotations().len(), 1);
        editor.step_history(&ctx, false);
        assert_eq!(editor.document.base_image().width(), 24);
        assert_eq!(editor.document.annotations().len(), 1);
        editor.step_history(&ctx, false);
        assert!(editor.document.annotations().is_empty());
        for _ in 0..3 {
            editor.step_history(&ctx, true);
        }
        assert_eq!(editor.document.annotations().len(), 2);
        assert_eq!(editor.document.base_image().width(), 28);
    }

    #[test]
    fn editor_pointer_gestures_preview_and_commit_eraser_stamp_and_resize() {
        let ctx = egui::Context::default();
        let mut editor = EditorViewState::open(&ctx, test_image(), None, ImageFormat::Png);
        editor.zoom = 6.0;
        editor.push_annotation(test_rectangle());
        let frame = |editor: &mut EditorViewState, events| {
            ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.interact(
                            Rect::from_min_size(Pos2::new(12.0, 12.0), Vec2::new(144.0, 120.0)),
                            ui.id().with("canvas"),
                            Sense::click_and_drag(),
                        );
                        editor.handle_canvas_pointer(ctx, &response, &|position| {
                            Point::new((position.x - 12.0) / 6.0, (position.y - 12.0) / 6.0)
                        });
                        if let Some(draft) = &editor.drag {
                            editor.paint_draft(
                                &ui.painter_at(response.rect),
                                draft,
                                &|point| Pos2::new(12.0 + point.x * 6.0, 12.0 + point.y * 6.0),
                                response.rect,
                                editor.zoom,
                                Color32::BLUE,
                            );
                        }
                    });
                },
            )
        };
        let moved = |column, row| {
            egui::Event::PointerMoved(Pos2::new(12.0 + column * 6.0_f32, 12.0 + row * 6.0_f32))
        };
        let click = |column, row, pressed| egui::Event::PointerButton {
            pos: Pos2::new(12.0 + column * 6.0_f32, 12.0 + row * 6.0_f32),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut editor, vec![]);

        editor.active_tool = EditorTool::Select;
        frame(
            &mut editor,
            vec![moved(12.0, 14.0), click(12.0, 14.0, true)],
        );
        frame(&mut editor, vec![moved(17.0, 16.0)]);
        assert!(matches!(editor.drag, Some(DragDraft::ResizeSelected(_))));
        assert_eq!(editor.undo.len(), 1);
        frame(
            &mut editor,
            vec![moved(18.0, 17.0), click(18.0, 17.0, false)],
        );
        assert_eq!(editor.document.annotations()[0].bounds().right(), 18.0);
        assert_eq!(editor.undo.len(), 2);

        editor.active_tool = EditorTool::SmartEraser;
        frame(&mut editor, vec![moved(6.0, 7.0), click(6.0, 7.0, true)]);
        let preview = frame(&mut editor, vec![moved(21.0, 7.0)]);
        assert!(preview.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::LineSegment { stroke, .. } if stroke.color == to_egui_color(RgbaColor::RED)
        )));
        assert!(matches!(
            editor.drag,
            Some(DragDraft::SmartEraser {
                color: RgbaColor::RED,
                ..
            })
        ));
        frame(&mut editor, vec![click(21.0, 7.0, false)]);
        assert_eq!(
            sample_pixel(&editor.document.flatten().unwrap(), Point::new(21.0, 7.0)),
            RgbaColor::RED
        );

        let image = sample_badge().unwrap();
        let texture = load_editor_texture(&ctx, &image, 0);
        editor.stamp = Some(OverlayStamp { image, texture });
        editor.stamp_opacity = 0.25;
        editor.active_tool = EditorTool::ImageOverlay;
        frame(&mut editor, vec![moved(2.0, 2.0), click(2.0, 2.0, true)]);
        frame(&mut editor, vec![moved(9.0, 8.0)]);
        assert!(matches!(editor.drag, Some(DragDraft::Shape { .. })));
        frame(&mut editor, vec![click(9.0, 8.0, false)]);
        assert!(
            matches!(editor.document.annotations()[2].kind, AnnotationKind::ImageOverlay { rect, opacity: 0.25, .. } if rect.width == 7.0 && rect.height == 6.0)
        );
    }
}
