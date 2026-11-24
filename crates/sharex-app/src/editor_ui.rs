use crate::theme::SharexPalette;
use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use sharex_core::{CapturedImage, ImageFormat, Region};
use sharex_editor::{
    Annotation, AnnotationKind, EditorDocument, EditorTool, Point, RectF, RgbaColor, SHAREX_PALETTE,
};
use std::path::PathBuf;

/// Actions emitted by the ShareX image editor back to the parent application shell.
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
    Shape { start: Point, current: Point },
    Freehand { points: Vec<Point> },
    MoveSelected { id: u64, last: Point },
}

/// Interactive ShareX-style screenshot annotation editor state.
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
            zoom: 1.0,
            fit_to_view: true,
            drag: None,
            status,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.document.is_dirty()
    }

    pub fn set_saved(&mut self, path: PathBuf) {
        self.source_path = Some(path.clone());
        self.document.mark_saved();
        self.status = format!("Saved to {}", path.display());
    }

    fn reload_texture(&mut self, ctx: &egui::Context) {
        self.texture_version = self.texture_version.saturating_add(1);
        self.texture = load_editor_texture(ctx, self.document.base_image(), self.texture_version);
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        palette: &SharexPalette,
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

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
            if self.document.undo() {
                self.reload_texture(ctx);
                self.status = "Undid last change.".into();
            }
            return;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
            if self.document.redo() {
                self.reload_texture(ctx);
                self.status = "Redid change.".into();
            }
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
            (egui::Key::C, EditorTool::Crop),
        ];
        for (key, tool) in shortcut_map {
            if ctx.input(|i| !i.modifiers.any() && i.key_pressed(key)) {
                self.active_tool = tool;
                self.drag = None;
            }
        }
    }

    fn render_tool_row(&mut self, ui: &mut egui::Ui, palette: &SharexPalette) {
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
        palette: &SharexPalette,
        action: &mut EditorAction,
    ) {
        ui.horizontal_wrapped(|ui| {
            for swatch in SHAREX_PALETTE {
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
                EditorTool::Text => {
                    ui.label("Text:");
                    ui.add(egui::TextEdit::singleline(&mut self.text_draft).desired_width(130.0));
                    ui.add(
                        egui::DragValue::new(&mut self.font_size)
                            .range(12.0..=64.0)
                            .prefix("Size "),
                    );
                    ui.checkbox(&mut self.text_background, "Bg");
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
                .add_enabled(self.document.can_undo(), egui::Button::new("Undo"))
                .clicked()
                && self.document.undo()
            {
                self.reload_texture(ctx);
                self.status = "Undid last change.".into();
            }
            if ui
                .add_enabled(self.document.can_redo(), egui::Button::new("Redo"))
                .clicked()
                && self.document.redo()
            {
                self.reload_texture(ctx);
                self.status = "Redid change.".into();
            }
            if ui
                .add_enabled(
                    self.document.selected_id().is_some(),
                    egui::Button::new("Delete"),
                )
                .clicked()
            {
                self.document.delete_selected();
            }
            if ui
                .add_enabled(
                    !self.document.annotations().is_empty(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.document.clear_annotations();
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

    fn render_canvas(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, palette: &SharexPalette) {
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

                painter.image(
                    self.texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );

                let to_screen =
                    |pt: Point| Pos2::new(rect.min.x + pt.x * zoom, rect.min.y + pt.y * zoom);
                let to_image = |pos: Pos2| {
                    Point::new(
                        ((pos.x - rect.min.x) / zoom).clamp(0.0, img_w),
                        ((pos.y - rect.min.y) / zoom).clamp(0.0, img_h),
                    )
                };

                for item in self.document.annotations() {
                    let selected = self.document.selected_id() == Some(item.id);
                    paint_annotation(&painter, item, &to_screen, zoom, selected, palette.accent);
                }

                self.handle_canvas_pointer(ctx, &response, &to_image);

                if let Some(draft) = &self.drag {
                    self.paint_draft(&painter, draft, &to_screen, rect, zoom, palette.accent);
                }
            });
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
                    self.document.push_annotation(AnnotationKind::StepNumber {
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
                        self.document.push_annotation(AnnotationKind::Text {
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
                _ => {}
            }
        }

        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match self.active_tool {
                EditorTool::Select => {
                    if let Some(id) = self.document.select_at(pt, 10.0 / self.zoom.max(0.25)) {
                        self.document.begin_move_undo();
                        self.drag = Some(DragDraft::MoveSelected { id, last: pt });
                    }
                }
                EditorTool::Freehand | EditorTool::Highlighter => {
                    self.drag = Some(DragDraft::Freehand { points: vec![pt] });
                }
                EditorTool::Rectangle
                | EditorTool::FilledRectangle
                | EditorTool::Ellipse
                | EditorTool::Arrow
                | EditorTool::Line
                | EditorTool::Pixelate
                | EditorTool::Blur
                | EditorTool::Crop => {
                    self.drag = Some(DragDraft::Shape {
                        start: pt,
                        current: pt,
                    });
                }
                EditorTool::Text | EditorTool::StepNumber => {}
            }
        }

        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match &mut self.drag {
                Some(DragDraft::Shape { current, .. }) => {
                    *current = pt;
                }
                Some(DragDraft::Freehand { points }) => {
                    if points
                        .last()
                        .is_none_or(|prev| (prev.x - pt.x).hypot(prev.y - pt.y) >= 1.5)
                    {
                        points.push(pt);
                    }
                }
                Some(DragDraft::MoveSelected { id, last }) => {
                    let dx = pt.x - last.x;
                    let dy = pt.y - last.y;
                    let target_id = *id;
                    *last = pt;
                    self.document.translate_annotation(target_id, dx, dy);
                }
                None => {}
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(draft) = self.drag.take()
        {
            match draft {
                DragDraft::MoveSelected { .. } => {
                    self.status = "Moved annotation.".into();
                }
                DragDraft::Freehand { points } => {
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
                        self.document.push_annotation(kind);
                    }
                }
                DragDraft::Shape { start, current } => {
                    let rect = RectF::from_points(start, current);
                    if rect.width < 3.0 && rect.height < 3.0 {
                        return;
                    }
                    match self.active_tool {
                        EditorTool::Rectangle => {
                            self.document.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: false,
                            });
                        }
                        EditorTool::FilledRectangle => {
                            self.document.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: true,
                            });
                        }
                        EditorTool::Ellipse => {
                            self.document.push_annotation(AnnotationKind::Ellipse {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Line => {
                            self.document.push_annotation(AnnotationKind::Line {
                                start,
                                end: current,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Arrow => {
                            self.document.push_annotation(AnnotationKind::Arrow {
                                start,
                                end: current,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        EditorTool::Pixelate => {
                            self.document.push_annotation(AnnotationKind::Pixelate {
                                rect,
                                block_size: self.effect_strength,
                            });
                        }
                        EditorTool::Blur => {
                            self.document.push_annotation(AnnotationKind::Blur {
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
            DragDraft::MoveSelected { .. } => {}
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
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.rgba(),
    );
    ctx.load_texture(
        format!("sharex_editor_canvas_{version}"),
        color,
        egui::TextureOptions::LINEAR,
    )
}
