use crate::editor_ui::{paint_annotation, paint_arrow, paint_ellipse_stroke, to_egui_color};
use crate::win_focus::OverlayFocusGuard;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use sharex_capture::DesktopSnapshot;
use sharex_core::{CapturedImage, Region};
use sharex_editor::{AnnotationKind, EditorDocument, Point, RectF, RgbaColor, SHAREX_PALETTE};

const OVERLAY_WINDOW_TITLE: &str = "ShareX Region Capture";

/// Result produced by one frame of the interactive ShareX region capture overlay.
pub enum OverlayOutcome {
    Pending,
    Cancelled,
    Captured {
        image: CapturedImage,
        global_region: Region,
        open_in_editor: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayTool {
    Region,
    Select,
    Rectangle,
    FilledRectangle,
    Ellipse,
    Arrow,
    Line,
    Freehand,
    Highlighter,
    Text,
    StepNumber,
    Pixelate,
    Blur,
}

impl OverlayTool {
    const ALL: [Self; 13] = [
        Self::Region,
        Self::Select,
        Self::Rectangle,
        Self::FilledRectangle,
        Self::Ellipse,
        Self::Arrow,
        Self::Line,
        Self::Freehand,
        Self::Highlighter,
        Self::Text,
        Self::StepNumber,
        Self::Pixelate,
        Self::Blur,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Region => "Region",
            Self::Select => "Select",
            Self::Rectangle => "Rect",
            Self::FilledRectangle => "Fill",
            Self::Ellipse => "Ellipse",
            Self::Arrow => "Arrow",
            Self::Line => "Line",
            Self::Freehand => "Pen",
            Self::Highlighter => "Highlight",
            Self::Text => "Text",
            Self::StepNumber => "Step",
            Self::Pixelate => "Pixelate",
            Self::Blur => "Blur",
        }
    }
}

enum OverlayDrag {
    RegionSelect { start: Point, current: Point },
    Shape { start: Point, current: Point },
    Freehand { points: Vec<Point> },
    MoveSelected { id: u64, last: Point },
}

/// ShareX-style borderless fullscreen region capture overlay with inline annotation toolbar.
pub struct RegionOverlayState {
    origin_x: i32,
    origin_y: i32,
    document: EditorDocument,
    texture: egui::TextureHandle,
    focus_guard: OverlayFocusGuard,
    active_tool: OverlayTool,
    selection: Option<RectF>,
    auto_capture_on_drag: bool,
    color: RgbaColor,
    stroke_width: f32,
    font_size: f32,
    text_draft: String,
    effect_strength: u32,
    drag: Option<OverlayDrag>,
}

impl RegionOverlayState {
    pub fn open(ctx: &egui::Context, snapshot: DesktopSnapshot) -> Self {
        let focus_guard = OverlayFocusGuard::capture_current_foreground();
        let color_img = egui::ColorImage::from_rgba_unmultiplied(
            [
                snapshot.image.width() as usize,
                snapshot.image.height() as usize,
            ],
            snapshot.image.rgba(),
        );
        let texture = ctx.load_texture(
            "sharex_region_overlay_backdrop",
            color_img,
            egui::TextureOptions::NEAREST,
        );
        Self {
            origin_x: snapshot.origin_x,
            origin_y: snapshot.origin_y,
            document: EditorDocument::new(snapshot.image),
            texture,
            focus_guard,
            active_tool: OverlayTool::Region,
            selection: None,
            auto_capture_on_drag: true,
            color: RgbaColor::RED,
            stroke_width: 4.0,
            font_size: 20.0,
            text_draft: "Note".into(),
            effect_strength: 12,
            drag: None,
        }
    }

    pub fn restore_previous_focus(&self) {
        self.focus_guard.restore_previous();
    }

    pub fn show(&mut self, ctx: &egui::Context) -> OverlayOutcome {
        let viewport_id = egui::ViewportId::from_hash_of("sharex_region_capture_overlay");
        let ppp = ctx.pixels_per_point().max(1.0);
        let img_w = self.document.base_image().width() as f32;
        let img_h = self.document.base_image().height() as f32;

        let builder = egui::ViewportBuilder::default()
            .with_title(OVERLAY_WINDOW_TITLE)
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_position([self.origin_x as f32 / ppp, self.origin_y as f32 / ppp])
            .with_inner_size([img_w / ppp, img_h / ppp])
            .with_active(true);

        let mut outcome = OverlayOutcome::Pending;

        ctx.show_viewport_immediate(viewport_id, builder, |overlay_ctx, _class| {
            self.focus_guard.enforce_overlay_focus(OVERLAY_WINDOW_TITLE);
            let typing = overlay_ctx.wants_keyboard_input();

            if overlay_ctx.input(|i| i.viewport().close_requested())
                || overlay_ctx.input(|i| i.key_pressed(egui::Key::Escape))
            {
                outcome = OverlayOutcome::Cancelled;
                return;
            }

            if overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
                self.document.undo();
            }
            if overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
                self.document.redo();
            }
            if !typing && overlay_ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.document.delete_selected();
            }
            if !typing && overlay_ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(captured) = self.finalize_capture(false) {
                    outcome = captured;
                }
                return;
            }

            if !typing {
                let shortcuts = [
                    (egui::Key::Space, OverlayTool::Region),
                    (egui::Key::V, OverlayTool::Select),
                    (egui::Key::R, OverlayTool::Rectangle),
                    (egui::Key::F, OverlayTool::FilledRectangle),
                    (egui::Key::E, OverlayTool::Ellipse),
                    (egui::Key::A, OverlayTool::Arrow),
                    (egui::Key::L, OverlayTool::Line),
                    (egui::Key::D, OverlayTool::Freehand),
                    (egui::Key::H, OverlayTool::Highlighter),
                    (egui::Key::T, OverlayTool::Text),
                    (egui::Key::N, OverlayTool::StepNumber),
                    (egui::Key::P, OverlayTool::Pixelate),
                    (egui::Key::B, OverlayTool::Blur),
                ];
                for (key, tool) in shortcuts {
                    if overlay_ctx.input(|i| !i.modifiers.any() && i.key_pressed(key)) {
                        self.active_tool = tool;
                        self.drag = None;
                    }
                }
            }

            let mut toolbar_action: Option<OverlayOutcome> = None;
            let mut toolbar_rect = Rect::NOTHING;

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(overlay_ctx, |ui| {
                    let screen_rect = ui.max_rect();
                    let scale_x = screen_rect.width().max(1.0) / img_w.max(1.0);
                    let scale_y = screen_rect.height().max(1.0) / img_h.max(1.0);
                    let zoom = scale_x.min(scale_y);

                    let to_screen = |pt: Point| {
                        Pos2::new(
                            screen_rect.min.x + pt.x * scale_x,
                            screen_rect.min.y + pt.y * scale_y,
                        )
                    };
                    let to_image = |pos: Pos2| {
                        Point::new(
                            ((pos.x - screen_rect.min.x) / scale_x).clamp(0.0, img_w),
                            ((pos.y - screen_rect.min.y) / scale_y).clamp(0.0, img_h),
                        )
                    };

                    let response = ui.allocate_rect(screen_rect, Sense::click_and_drag());
                    let painter = ui.painter_at(screen_rect);

                    painter.image(
                        self.texture.id(),
                        screen_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    let accent = Color32::from_rgb(59, 130, 246);
                    for item in self.document.annotations() {
                        let selected = self.document.selected_id() == Some(item.id);
                        paint_annotation(&painter, item, &to_screen, zoom, selected, accent);
                    }

                    let active_region = match &self.drag {
                        Some(OverlayDrag::RegionSelect { start, current }) => {
                            Some(RectF::from_points(*start, *current))
                        }
                        _ => self.selection,
                    };

                    let scrim = Color32::from_black_alpha(125);
                    if let Some(sel_img) = active_region {
                        let sel = Rect::from_min_max(
                            to_screen(Point::new(sel_img.x, sel_img.y)),
                            to_screen(Point::new(sel_img.right(), sel_img.bottom())),
                        )
                        .intersect(screen_rect);

                        painter.rect_filled(
                            Rect::from_min_max(
                                screen_rect.min,
                                Pos2::new(screen_rect.max.x, sel.min.y),
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(screen_rect.min.x, sel.max.y),
                                screen_rect.max,
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(screen_rect.min.x, sel.min.y),
                                Pos2::new(sel.min.x, sel.max.y),
                            ),
                            0.0,
                            scrim,
                        );
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(sel.max.x, sel.min.y),
                                Pos2::new(screen_rect.max.x, sel.max.y),
                            ),
                            0.0,
                            scrim,
                        );

                        painter.rect_stroke(
                            sel,
                            0.0,
                            Stroke::new(2.0_f32, accent),
                            egui::StrokeKind::Outside,
                        );
                        painter.rect_stroke(
                            sel,
                            0.0,
                            Stroke::new(1.0_f32, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    } else if self.active_tool == OverlayTool::Region {
                        painter.rect_filled(screen_rect, 0.0, scrim);
                    }

                    if let Some(draft) = &self.drag {
                        self.paint_draft(&painter, draft, &to_screen, zoom, accent);
                    }

                    let area_resp = egui::Area::new(egui::Id::new("sharex_region_floating_bar"))
                        .order(egui::Order::Foreground)
                        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 10.0))
                        .show(overlay_ctx, |ui| {
                            egui::Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(24, 28, 34, 242))
                                .stroke(Stroke::new(1.0_f32, accent))
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    self.render_floating_toolbar(ui, accent, &mut toolbar_action);
                                });
                        });
                    toolbar_rect = area_resp.response.rect;

                    let pointer_pos = overlay_ctx.input(|i| i.pointer.hover_pos());
                    let over_toolbar = pointer_pos.is_some_and(|p| toolbar_rect.contains(p))
                        || overlay_ctx.is_pointer_over_area();

                    if !over_toolbar {
                        overlay_ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

                        if response.secondary_clicked() {
                            if self.active_tool != OverlayTool::Region {
                                self.active_tool = OverlayTool::Region;
                                self.drag = None;
                            } else {
                                outcome = OverlayOutcome::Cancelled;
                                return;
                            }
                        }

                        if let Some(finished) =
                            self.handle_canvas_input(&response, &to_image)
                        {
                            outcome = finished;
                            return;
                        }
                    }

                    if let Some(pos) = pointer_pos
                        && !over_toolbar
                    {
                        if self.active_tool == OverlayTool::Region {
                            let guide = Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(96, 165, 250, 165),
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(screen_rect.min.x, pos.y),
                                    Pos2::new(screen_rect.max.x, pos.y),
                                ],
                                guide,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(pos.x, screen_rect.min.y),
                                    Pos2::new(pos.x, screen_rect.max.y),
                                ],
                                guide,
                            );
                        }

                        let img_pt = to_image(pos);
                        let hud = if let Some(sel) = active_region {
                            format!(
                                "X: {}  Y: {}  W: {}  H: {}  |  Tool: {}  |  Enter: save  |  Esc: cancel",
                                self.origin_x + sel.x.round() as i32,
                                self.origin_y + sel.y.round() as i32,
                                sel.width.round() as u32,
                                sel.height.round() as u32,
                                self.active_tool.label()
                            )
                        } else {
                            format!(
                                "X: {}  Y: {}  |  Tool: {}  |  Drag to {}  |  Esc: cancel",
                                self.origin_x + img_pt.x.round() as i32,
                                self.origin_y + img_pt.y.round() as i32,
                                self.active_tool.label(),
                                if self.active_tool == OverlayTool::Region {
                                    "select region"
                                } else {
                                    "annotate"
                                }
                            )
                        };

                        let galley =
                            painter.layout_no_wrap(hud, FontId::monospace(12.5), Color32::WHITE);
                        let badge_pos = Pos2::new(
                            (pos.x + 16.0)
                                .min(screen_rect.max.x - galley.size().x - 24.0)
                                .max(12.0),
                            (pos.y + 18.0)
                                .min(screen_rect.max.y - 36.0)
                                .max(toolbar_rect.max.y + 8.0),
                        );
                        let badge_rect =
                            Rect::from_min_size(badge_pos, galley.size() + Vec2::new(14.0, 8.0));
                        painter.rect_filled(
                            badge_rect,
                            CornerRadius::same(4),
                            Color32::from_rgba_unmultiplied(20, 24, 30, 232),
                        );
                        painter.rect_stroke(
                            badge_rect,
                            CornerRadius::same(4),
                            Stroke::new(1.0_f32, accent),
                            egui::StrokeKind::Inside,
                        );
                        painter.galley(badge_pos + Vec2::new(7.0, 4.0), galley, Color32::WHITE);
                    }
                });

            if let Some(act) = toolbar_action {
                outcome = act;
            }
        });

        outcome
    }

    fn render_floating_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        accent: Color32,
        toolbar_action: &mut Option<OverlayOutcome>,
    ) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                for tool in OverlayTool::ALL {
                    let selected = self.active_tool == tool;
                    let btn = egui::Button::new(RichText::new(tool.label()).size(12.0).color(
                        if selected {
                            Color32::WHITE
                        } else {
                            Color32::from_rgb(220, 226, 234)
                        },
                    ))
                    .fill(if selected {
                        accent
                    } else {
                        Color32::from_rgb(38, 43, 52)
                    });
                    if ui.add(btn).clicked() {
                        self.active_tool = tool;
                        self.drag = None;
                    }
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for swatch in SHAREX_PALETTE {
                    let is_current = self.color == swatch;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(3), to_egui_color(swatch));
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(3),
                        Stroke::new(
                            if is_current { 2.0_f32 } else { 1.0_f32 },
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(70, 78, 90)
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if resp.clicked() {
                        self.color = swatch;
                    }
                }

                ui.separator();
                match self.active_tool {
                    OverlayTool::Text => {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.text_draft).desired_width(110.0),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.font_size)
                                .range(12.0..=56.0)
                                .prefix("Sz "),
                        );
                    }
                    OverlayTool::Pixelate | OverlayTool::Blur => {
                        ui.add(
                            egui::DragValue::new(&mut self.effect_strength)
                                .range(4..=32)
                                .prefix("Str "),
                        );
                    }
                    OverlayTool::StepNumber => {
                        ui.label(
                            RichText::new(format!("#{}", self.document.next_step_number()))
                                .color(Color32::WHITE),
                        );
                    }
                    _ => {
                        ui.add(
                            egui::DragValue::new(&mut self.stroke_width)
                                .range(1.0..=24.0)
                                .suffix("px"),
                        );
                    }
                }

                ui.separator();
                if ui
                    .add_enabled(self.document.can_undo(), egui::Button::new("Undo"))
                    .clicked()
                {
                    self.document.undo();
                }
                if ui
                    .add_enabled(self.document.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    self.document.redo();
                }
                if ui
                    .add_enabled(
                        self.document.selected_id().is_some(),
                        egui::Button::new("Del"),
                    )
                    .clicked()
                {
                    self.document.delete_selected();
                }

                ui.separator();
                ui.checkbox(&mut self.auto_capture_on_drag, "Auto-capture");

                let cap_btn = egui::Button::new(
                    RichText::new("Capture [Enter]")
                        .size(12.0)
                        .color(Color32::WHITE),
                )
                .fill(accent);
                if ui.add(cap_btn).clicked() {
                    *toolbar_action = self.finalize_capture(false);
                }
                if ui.button("Full Editor").clicked() {
                    *toolbar_action = self.finalize_capture(true);
                }
                if ui.button("Cancel [Esc]").clicked() {
                    *toolbar_action = Some(OverlayOutcome::Cancelled);
                }
            });
        });
    }

    fn handle_canvas_input(
        &mut self,
        response: &egui::Response,
        to_image: &dyn Fn(Pos2) -> Point,
    ) -> Option<OverlayOutcome> {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match self.active_tool {
                OverlayTool::Select => {
                    self.document.select_at(pt, 12.0);
                }
                OverlayTool::StepNumber => {
                    let num = self.document.next_step_number();
                    self.document.push_annotation(AnnotationKind::StepNumber {
                        center: pt,
                        number: num,
                        color: self.color,
                        radius: (self.font_size * 0.85).clamp(12.0, 34.0),
                    });
                }
                OverlayTool::Text => {
                    let text = self.text_draft.trim();
                    if !text.is_empty() {
                        self.document.push_annotation(AnnotationKind::Text {
                            position: pt,
                            text: text.to_owned(),
                            color: self.color,
                            font_size: self.font_size,
                            background: Some(RgbaColor::rgba(20, 24, 30, 210)),
                        });
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
                OverlayTool::Region => {
                    self.drag = Some(OverlayDrag::RegionSelect {
                        start: pt,
                        current: pt,
                    });
                }
                OverlayTool::Select => {
                    if let Some(id) = self.document.select_at(pt, 12.0) {
                        self.document.begin_move_undo();
                        self.drag = Some(OverlayDrag::MoveSelected { id, last: pt });
                    }
                }
                OverlayTool::Freehand | OverlayTool::Highlighter => {
                    self.drag = Some(OverlayDrag::Freehand { points: vec![pt] });
                }
                OverlayTool::Rectangle
                | OverlayTool::FilledRectangle
                | OverlayTool::Ellipse
                | OverlayTool::Arrow
                | OverlayTool::Line
                | OverlayTool::Pixelate
                | OverlayTool::Blur => {
                    self.drag = Some(OverlayDrag::Shape {
                        start: pt,
                        current: pt,
                    });
                }
                OverlayTool::Text | OverlayTool::StepNumber => {}
            }
        }

        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match &mut self.drag {
                Some(OverlayDrag::RegionSelect { current, .. })
                | Some(OverlayDrag::Shape { current, .. }) => {
                    *current = pt;
                }
                Some(OverlayDrag::Freehand { points }) => {
                    if points
                        .last()
                        .is_none_or(|prev| (prev.x - pt.x).hypot(prev.y - pt.y) >= 1.5)
                    {
                        points.push(pt);
                    }
                }
                Some(OverlayDrag::MoveSelected { id, last }) => {
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
                OverlayDrag::RegionSelect { start, current } => {
                    let rect = RectF::from_points(start, current);
                    if rect.width >= 4.0 && rect.height >= 4.0 {
                        self.selection = Some(rect);
                        if self.auto_capture_on_drag {
                            return self.finalize_capture(false);
                        }
                    }
                }
                OverlayDrag::MoveSelected { .. } => {}
                OverlayDrag::Freehand { points } => {
                    if points.len() >= 2 {
                        let kind = if self.active_tool == OverlayTool::Highlighter {
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
                OverlayDrag::Shape { start, current } => {
                    let rect = RectF::from_points(start, current);
                    if rect.width < 3.0 && rect.height < 3.0 {
                        return None;
                    }
                    match self.active_tool {
                        OverlayTool::Rectangle => {
                            self.document.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: false,
                            });
                        }
                        OverlayTool::FilledRectangle => {
                            self.document.push_annotation(AnnotationKind::Rectangle {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                                filled: true,
                            });
                        }
                        OverlayTool::Ellipse => {
                            self.document.push_annotation(AnnotationKind::Ellipse {
                                rect,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        OverlayTool::Line => {
                            self.document.push_annotation(AnnotationKind::Line {
                                start,
                                end: current,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        OverlayTool::Arrow => {
                            self.document.push_annotation(AnnotationKind::Arrow {
                                start,
                                end: current,
                                color: self.color,
                                stroke_width: self.stroke_width,
                            });
                        }
                        OverlayTool::Pixelate => {
                            self.document.push_annotation(AnnotationKind::Pixelate {
                                rect,
                                block_size: self.effect_strength,
                            });
                        }
                        OverlayTool::Blur => {
                            self.document.push_annotation(AnnotationKind::Blur {
                                rect,
                                radius: (self.effect_strength / 2).max(2),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        None
    }

    fn paint_draft(
        &self,
        painter: &egui::Painter,
        draft: &OverlayDrag,
        to_screen: &dyn Fn(Point) -> Pos2,
        zoom: f32,
        accent: Color32,
    ) {
        match draft {
            OverlayDrag::RegionSelect { .. } | OverlayDrag::MoveSelected { .. } => {}
            OverlayDrag::Freehand { points } => {
                if points.len() >= 2 {
                    let pts: Vec<Pos2> = points.iter().copied().map(to_screen).collect();
                    let (col, width) = if self.active_tool == OverlayTool::Highlighter {
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
                    painter.add(egui::Shape::line(pts, Stroke::new(width, col)));
                }
            }
            OverlayDrag::Shape { start, current } => {
                let r = RectF::from_points(*start, *current);
                let sr = Rect::from_min_max(
                    to_screen(Point::new(r.x, r.y)),
                    to_screen(Point::new(r.right(), r.bottom())),
                );
                let col = to_egui_color(self.color);
                let sw = (self.stroke_width * zoom).max(1.5);
                match self.active_tool {
                    OverlayTool::Rectangle => {
                        painter.rect_stroke(
                            sr,
                            0.0,
                            Stroke::new(sw, col),
                            egui::StrokeKind::Inside,
                        );
                    }
                    OverlayTool::FilledRectangle => {
                        painter.rect_filled(sr, 0.0, col);
                    }
                    OverlayTool::Ellipse => {
                        paint_ellipse_stroke(painter, sr, Stroke::new(sw, col));
                    }
                    OverlayTool::Line => {
                        painter.line_segment(
                            [to_screen(*start), to_screen(*current)],
                            Stroke::new(sw, col),
                        );
                    }
                    OverlayTool::Arrow => {
                        paint_arrow(painter, to_screen(*start), to_screen(*current), col, sw);
                    }
                    OverlayTool::Pixelate | OverlayTool::Blur => {
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
                    _ => {}
                }
            }
        }
    }

    fn finalize_capture(&self, open_in_editor: bool) -> Option<OverlayOutcome> {
        let flat = self.document.flatten().ok()?;
        let max_w = flat.width();
        let max_h = flat.height();

        let (cropped, global_region) = if let Some(sel) = self.selection {
            let x = (sel.x.floor().max(0.0) as u32).min(max_w.saturating_sub(1));
            let y = (sel.y.floor().max(0.0) as u32).min(max_h.saturating_sub(1));
            let w = (sel.width.round() as u32).min(max_w.saturating_sub(x));
            let h = (sel.height.round() as u32).min(max_h.saturating_sub(y));
            if w < 4 || h < 4 {
                return None;
            }
            let local = Region::new(x as i32, y as i32, w, h).ok()?;
            let image = flat.crop(local).ok()?;
            let global =
                Region::new(self.origin_x + x as i32, self.origin_y + y as i32, w, h).ok()?;
            (image, global)
        } else {
            let global = Region::new(self.origin_x, self.origin_y, max_w, max_h).ok()?;
            (flat, global)
        };

        Some(OverlayOutcome::Captured {
            image: cropped,
            global_region,
            open_in_editor,
        })
    }
}
