use crate::editor_ui::{paint_annotation, paint_arrow, paint_ellipse_stroke, to_egui_color};
use crate::icons::{Icon, paint_icon, toolbar_icon_button};
use crate::win_focus::OverlayFocusGuard;
use eframe::egui::emath::GuiRounding;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Shadow, Stroke, Vec2,
};
use sharex_capture::{DesktopSnapshot, XcapCapturer, find_window_at_point};
use sharex_core::{CapturedImage, Region, ScreenCapturer, WindowInfo};
use sharex_editor::{AnnotationKind, EditorDocument, Point, RectF, RgbaColor, SHAREX_PALETTE};
use sharex_tools::{ColorSample, sample_image};

const OVERLAY_WINDOW_TITLE: &str = "ShareX Region Capture";
const MIN_REGION_DRAG: f32 = 4.0;
const MAGNIFIER_HALF_EXTENT: i32 = 5;
const MIN_LOUPE_HALF_EXTENT: i32 = 3;
const MAX_LOUPE_HALF_EXTENT: i32 = 9;

fn glass_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(15, 18, 24, 236)
}

fn glass_border() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 28)
}

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

/// Determines what the overlay returns when the user makes a selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    #[default]
    Capture,
    Ocr,
    SelectRecordingRegion,
    ColorPicker,
}

impl OverlayMode {
    const fn hint(self) -> &'static str {
        match self {
            Self::Capture => "Capture Region",
            Self::Ocr => "OCR Region: drag over text or click a window. Esc to cancel.",
            Self::SelectRecordingRegion => {
                "Select Recording Region: drag an area or click a window. Esc to cancel."
            }
            Self::ColorPicker => "Color Picker: click any pixel to copy HEX. Esc to cancel.",
        }
    }

    fn finish_region(
        self,
        document: &EditorDocument,
        selection: Option<RectF>,
        origin: (i32, i32),
        open_in_editor: bool,
    ) -> Option<OverlayModeOutcome> {
        let base = document.base_image();
        let (width, height) = (base.width(), base.height());
        let local = match selection {
            Some(selection) => {
                let x = (selection.x.floor() as u32).min(width - 1);
                let y = (selection.y.floor() as u32).min(height - 1);
                let w = (selection.width.round() as u32).min(width - x);
                let h = (selection.height.round() as u32).min(height - y);
                Region::new(x as i32, y as i32, w, h).ok()?
            }
            None if self == Self::Capture => Region::new(0, 0, width, height).ok()?,
            None => return None,
        };
        let global_region = Region::new(
            origin.0.checked_add(local.x())?,
            origin.1.checked_add(local.y())?,
            local.width(),
            local.height(),
        )
        .ok()?;

        match self {
            Self::Capture => {
                let flat = document.flatten().ok()?;
                let image = if selection.is_some() {
                    flat.crop(local).ok()?
                } else {
                    flat
                };
                Some(OverlayModeOutcome::Captured {
                    image,
                    global_region,
                    open_in_editor,
                })
            }
            Self::Ocr => Some(OverlayModeOutcome::OcrRegion {
                image: base.crop(local).ok()?,
                global_region,
            }),
            Self::SelectRecordingRegion => Some(OverlayModeOutcome::RecordingRegion(global_region)),
            Self::ColorPicker => None,
        }
    }
}

/// Result produced by one frame of a mode-aware region overlay.
pub enum OverlayModeOutcome {
    Pending,
    Cancelled,
    Captured {
        image: CapturedImage,
        global_region: Region,
        open_in_editor: bool,
    },
    OcrRegion {
        image: CapturedImage,
        global_region: Region,
    },
    RecordingRegion(Region),
    ColorPicked {
        hex: String,
        r: u8,
        g: u8,
        b: u8,
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

    const fn icon(self) -> Icon {
        match self {
            Self::Region => Icon::Region,
            Self::Select => Icon::SelectCursor,
            Self::Rectangle => Icon::RectOutline,
            Self::FilledRectangle => Icon::RectFilled,
            Self::Ellipse => Icon::Ellipse,
            Self::Arrow => Icon::Arrow,
            Self::Line => Icon::Line,
            Self::Freehand => Icon::Pen,
            Self::Highlighter => Icon::Highlighter,
            Self::Text => Icon::TextT,
            Self::StepNumber => Icon::StepNumber,
            Self::Pixelate => Icon::Pixelate,
            Self::Blur => Icon::Blur,
        }
    }

    const fn tooltip(self) -> &'static str {
        match self {
            Self::Region => "Capture Region (Space)",
            Self::Select => "Select / Move (V)",
            Self::Rectangle => "Rectangle (R)",
            Self::FilledRectangle => "Filled Rectangle (F)",
            Self::Ellipse => "Ellipse (E)",
            Self::Arrow => "Arrow (A)",
            Self::Line => "Line (L)",
            Self::Freehand => "Freehand Pen (D)",
            Self::Highlighter => "Highlighter (H)",
            Self::Text => "Text (T)",
            Self::StepNumber => "Step Number (N)",
            Self::Pixelate => "Pixelate (P)",
            Self::Blur => "Blur (B)",
        }
    }
}

enum OverlayDrag {
    RegionPress { start: Point },
    RegionSelect { start: Point, current: Point },
    Shape { start: Point, current: Point },
    Freehand { points: Vec<Point> },
    MoveSelected { id: u64, last: Point },
}

/// ShareX-style borderless fullscreen region capture overlay with inline annotation toolbar.
pub struct RegionOverlayState {
    origin_x: i32,
    origin_y: i32,
    primary_monitor: Option<RectF>,
    mode: OverlayMode,
    document: EditorDocument,
    texture: egui::TextureHandle,
    focus_guard: OverlayFocusGuard,
    focus_acquired: bool,
    overlay_revealed: bool,
    viewport_fitted: bool,
    viewport_inner_size: Option<Vec2>,
    active_tool: OverlayTool,
    selection: Option<RectF>,
    auto_capture_on_drag: bool,
    windows: Vec<WindowInfo>,
    snap_to_windows: bool,
    show_magnifier: bool,
    loupe_half_extent: i32,
    show_style_bar: bool,
    color: RgbaColor,
    stroke_width: f32,
    font_size: f32,
    text_draft: String,
    effect_strength: u32,
    drag: Option<OverlayDrag>,
}

impl RegionOverlayState {
    /// Opens the overlay in capture mode.
    pub fn open(ctx: &egui::Context, snapshot: DesktopSnapshot) -> Self {
        Self::open_with_mode(ctx, snapshot, OverlayMode::Capture)
    }

    /// Opens the overlay over the supplied desktop snapshot in the requested mode.
    pub fn open_with_mode(
        ctx: &egui::Context,
        snapshot: DesktopSnapshot,
        mode: OverlayMode,
    ) -> Self {
        let focus_guard = OverlayFocusGuard::capture_current_foreground();
        let capturer = XcapCapturer::new();
        let mut windows = if mode == OverlayMode::ColorPicker {
            Vec::new()
        } else {
            capturer.list_snap_windows().unwrap_or_default()
        };
        windows.retain(|window| {
            !window.title.contains("ShareX") && !window.title.contains("VoltShot")
        });
        let primary_monitor = capturer.list_monitors().ok().and_then(|monitors| {
            let primary = monitors.into_iter().find(|monitor| monitor.is_primary)?;
            Some(RectF {
                x: (i64::from(primary.x) - i64::from(snapshot.origin_x)) as f32,
                y: (i64::from(primary.y) - i64::from(snapshot.origin_y)) as f32,
                width: primary.width as f32,
                height: primary.height as f32,
            })
        });
        let color_img = crate::image_texture::bounded_color_image(
            &snapshot.image,
            ctx.input(|input| input.max_texture_side),
        );
        let texture = ctx.load_texture(
            "sharex_region_overlay_backdrop",
            color_img,
            egui::TextureOptions::NEAREST,
        );
        Self {
            origin_x: snapshot.origin_x,
            origin_y: snapshot.origin_y,
            primary_monitor,
            mode,
            document: EditorDocument::new(snapshot.image),
            texture,
            focus_guard,
            focus_acquired: false,
            overlay_revealed: false,
            viewport_fitted: false,
            viewport_inner_size: None,
            active_tool: OverlayTool::Region,
            selection: None,
            auto_capture_on_drag: true,
            windows,
            snap_to_windows: true,
            show_magnifier: true,
            loupe_half_extent: MAGNIFIER_HALF_EXTENT,
            show_style_bar: false,
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

    /// Shows the capture overlay with the legacy capture-only result.
    pub fn show(&mut self, ctx: &egui::Context) -> OverlayOutcome {
        match self.show_mode(ctx) {
            OverlayModeOutcome::Pending => OverlayOutcome::Pending,
            OverlayModeOutcome::Captured {
                image,
                global_region,
                open_in_editor,
            } => OverlayOutcome::Captured {
                image,
                global_region,
                open_in_editor,
            },
            OverlayModeOutcome::Cancelled
            | OverlayModeOutcome::OcrRegion { .. }
            | OverlayModeOutcome::RecordingRegion(_)
            | OverlayModeOutcome::ColorPicked { .. } => OverlayOutcome::Cancelled,
        }
    }

    /// Shows one overlay frame and returns the result for the active mode.
    pub fn show_mode(&mut self, ctx: &egui::Context) -> OverlayModeOutcome {
        let viewport_id = egui::ViewportId::from_hash_of("sharex_region_capture_overlay");
        let ppp = ctx.pixels_per_point();
        let img_w = self.document.base_image().width() as f32;
        let img_h = self.document.base_image().height() as f32;

        let reveal_now = self.overlay_revealed;
        let builder = egui::ViewportBuilder::default()
            .with_title(OVERLAY_WINDOW_TITLE)
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_taskbar(false)
            .with_transparent(true)
            .with_visible(reveal_now)
            .with_clamp_size_to_monitor_size(false)
            .with_position([self.origin_x as f32 / ppp, self.origin_y as f32 / ppp])
            .with_inner_size([img_w / ppp, img_h / ppp])
            .with_active(reveal_now);

        let mut outcome = OverlayModeOutcome::Pending;

        ctx.show_viewport_immediate(viewport_id, builder, |overlay_ctx, class| {
            if class != egui::ViewportClass::Embedded {
                if !self.overlay_revealed {
                    let prepared = crate::win_focus::prepare_overlay_window(
                        OVERLAY_WINDOW_TITLE,
                        self.origin_x,
                        self.origin_y,
                        img_w as i32,
                        img_h as i32,
                    );
                    if prepared {
                        self.viewport_fitted = true;
                    }
                } else {
                    let ppp = overlay_ctx.pixels_per_point();
                    let size = Vec2::new(img_w, img_h) / ppp;
                    let (inner, outer, focused) = overlay_ctx.input(|input| {
                        let viewport = input.viewport();
                        (viewport.inner_rect, viewport.outer_rect, viewport.focused)
                    });
                    let inner_size = inner.map(|rect| rect.size());
                    if !self.viewport_fitted
                        || inner_size != self.viewport_inner_size
                        || inner_size.is_some_and(|current| exceeds_half_pixel(size - current, ppp))
                    {
                        self.viewport_inner_size = inner_size;
                        self.fit_viewport_to_snapshot(overlay_ctx, size, inner, outer);
                    }
                    if !self.focus_acquired {
                        self.focus_acquired = focused == Some(true);
                        if !self.focus_acquired {
                            self.focus_guard.enforce_overlay_focus(OVERLAY_WINDOW_TITLE);
                        }
                    }
                }
            }
            let typing = overlay_ctx.wants_keyboard_input();
            let capture_mode = self.mode == OverlayMode::Capture;
            let color_picker = self.mode == OverlayMode::ColorPicker;

            if overlay_ctx.input(|i| i.viewport().close_requested())
                || overlay_ctx.input(|i| i.key_pressed(egui::Key::Escape))
            {
                outcome = OverlayModeOutcome::Cancelled;
                close_overlay_viewport(overlay_ctx);
                return;
            }

            if capture_mode
                && overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z))
            {
                self.document.undo();
            }
            if capture_mode
                && overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y))
            {
                self.document.redo();
            }
            let delete_requested = capture_mode
                && !typing
                && overlay_ctx.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                });
            if capture_mode && !typing && overlay_ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(captured) = self.finalize_selection(false) {
                    outcome = captured;
                    close_overlay_viewport(overlay_ctx);
                }
                return;
            }

            if capture_mode && !typing {
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

            let mut toolbar_action = None;

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
                    let to_image_unclamped = |pos: Pos2| {
                        Point::new(
                            (pos.x - screen_rect.min.x) / scale_x,
                            (pos.y - screen_rect.min.y) / scale_y,
                        )
                    };
                    let to_image = |pos: Pos2| {
                        let point = to_image_unclamped(pos);
                        Point::new(point.x.clamp(0.0, img_w), point.y.clamp(0.0, img_h))
                    };
                    let to_screen_rect = |rect: RectF| {
                        Rect::from_min_max(
                            to_screen(Point::new(rect.x, rect.y)),
                            to_screen(Point::new(rect.right(), rect.bottom())),
                        )
                        .intersect(screen_rect)
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
                    let primary_bounds = self
                        .primary_monitor
                        .map(to_screen_rect)
                        .filter(|rect| rect.is_positive());
                    let toolbar_anchor = if let Some(bounds) = primary_bounds {
                        toolbar_anchor_in_overlay(0, 0, bounds, scale_x, scale_y)
                    } else {
                        toolbar_anchor_in_overlay(
                            self.origin_x,
                            self.origin_y,
                            screen_rect,
                            scale_x,
                            scale_y,
                        )
                    };
                    let toolbar_rect = egui::Area::new(egui::Id::new("sharex_region_floating_bar"))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::CENTER_TOP)
                        .fixed_pos(toolbar_anchor)
                        .constrain_to(primary_bounds.unwrap_or(screen_rect))
                        .interactable(capture_mode)
                        .show(overlay_ctx, |ui| {
                            egui::Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(15, 18, 24, 236))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 28),
                                ))
                                .corner_radius(CornerRadius::same(10))
                                .shadow(Shadow {
                                    offset: [0, 6],
                                    blur: 18,
                                    spread: 0,
                                    color: Color32::from_black_alpha(120),
                                })
                                .inner_margin(Margin::symmetric(8, 5))
                                .show(ui, |ui| {
                                    if capture_mode {
                                        self.render_floating_toolbar(
                                            ui,
                                            accent,
                                            &mut toolbar_action,
                                        );
                                    } else {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 7.0;
                                            let mode_icon = match self.mode {
                                                OverlayMode::Ocr => Icon::OcrText,
                                                OverlayMode::SelectRecordingRegion => {
                                                    Icon::Recording
                                                }
                                                OverlayMode::ColorPicker => Icon::EyeDropper,
                                                OverlayMode::Capture => Icon::Region,
                                            };
                                            let (icon_rect, _) = ui.allocate_exact_size(
                                                Vec2::splat(15.0_f32),
                                                Sense::hover(),
                                            );
                                            paint_icon(ui.painter(), icon_rect, mode_icon, accent);
                                            ui.label(
                                                RichText::new(self.mode.hint())
                                                    .size(12.0)
                                                    .color(Color32::from_rgb(236, 240, 246)),
                                            );
                                        });
                                    }
                                });
                        })
                        .response
                        .rect;

                    let is_canvas = |pos: Pos2| {
                        screen_rect.contains(pos) && (!capture_mode || !toolbar_rect.contains(pos))
                    };
                    let pointer_pos = overlay_ctx
                        .input(|i| i.pointer.hover_pos())
                        .filter(|&pos| is_canvas(pos));

                    if pointer_pos.is_some() {
                        overlay_ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                        let scroll_y = overlay_ctx.input(|i| i.raw_scroll_delta.y);
                        if scroll_y.abs() >= 1.0_f32 {
                            let step = if scroll_y > 0.0_f32 { 1 } else { -1 };
                            self.apply_scroll_step(step, color_picker);
                        }
                        if response.secondary_clicked() {
                            if self.active_tool == OverlayTool::Region {
                                outcome = OverlayModeOutcome::Cancelled;
                                close_overlay_viewport(overlay_ctx);
                                return;
                            }
                            self.active_tool = OverlayTool::Region;
                            self.drag = None;
                        }
                    }

                    if delete_requested {
                        delete_hovered_or_selected(&mut self.document, pointer_pos.map(&to_image));
                    }

                    if color_picker {
                        let click = overlay_ctx.input(|input| {
                            input.events.iter().find_map(|event| match *event {
                                egui::Event::PointerButton {
                                    pos,
                                    button: egui::PointerButton::Primary,
                                    pressed: true,
                                    ..
                                } if is_canvas(pos) => Some(to_image(pos)),
                                _ => None,
                            })
                        });
                        let image = self.document.base_image();
                        if let Some(ColorSample { r, g, b, .. }) = click.and_then(|point| {
                            let x = (point.x.floor() as u32).min(image.width() - 1);
                            let y = (point.y.floor() as u32).min(image.height() - 1);
                            sample_image(image, x, y).ok()
                        }) {
                            let hex = format_picked_color(r, g, b);
                            overlay_ctx.copy_text(hex.clone());
                            outcome = OverlayModeOutcome::ColorPicked { hex, r, g, b };
                            close_overlay_viewport(overlay_ctx);
                            return;
                        }
                    } else if self.active_tool == OverlayTool::Region {
                        let selection = overlay_ctx.input(|input| {
                            self.handle_region_input(&input.events, &is_canvas, &to_image_unclamped)
                        });
                        if let Some(selection) = selection {
                            self.selection = Some(selection);
                            if (!capture_mode || self.auto_capture_on_drag)
                                && let Some(captured) = self.finalize_selection(false)
                            {
                                outcome = captured;
                                close_overlay_viewport(overlay_ctx);
                                return;
                            }
                        }
                    } else if pointer_pos.is_some() {
                        self.handle_annotation_input(&response, &to_image);
                    }

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
                    let snapped_window = if active_region.is_some() {
                        None
                    } else {
                        pointer_pos.and_then(|pos| self.snapped_window_at(to_image(pos)))
                    };
                    let target_snap_rect = snapped_window.map(|(_, rect)| rect);

                    let scrim = Color32::from_black_alpha(112);
                    if let Some(sel_img) = active_region.or(target_snap_rect) {
                        let sel = to_screen_rect(sel_img);

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

                        if active_region.is_some() {
                            paint_dashed_border(
                                &painter,
                                sel,
                                Color32::WHITE,
                                Color32::from_black_alpha(230),
                                6.0_f32,
                            );
                            if !matches!(self.drag, Some(OverlayDrag::RegionSelect { .. })) {
                                paint_selection_handles(&painter, sel);
                            }
                        }
                    } else if !color_picker && self.active_tool == OverlayTool::Region {
                        painter.rect_filled(screen_rect, 0.0, scrim);
                    }

                    if let Some(draft) = &self.drag {
                        self.paint_draft(&painter, draft, &to_screen, zoom, accent);
                    }

                    if let Some((window, rect)) = snapped_window {
                        paint_snapped_window(
                            &painter,
                            to_screen_rect(rect),
                            screen_rect,
                            toolbar_rect,
                            window,
                        );
                    }

                    if let Some(pos) = pointer_pos {
                        if self.active_tool == OverlayTool::Region {
                            let px = 1.0_f32 / painter.pixels_per_point();
                            let snapped_pos = pos.round_to_pixels(painter.pixels_per_point());
                            let shadow_guide = Stroke::new(px, Color32::from_black_alpha(80));
                            let light_guide = Stroke::new(
                                px,
                                Color32::from_rgba_unmultiplied(255, 255, 255, 155),
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(screen_rect.min.x, snapped_pos.y + px),
                                    Pos2::new(screen_rect.max.x, snapped_pos.y + px),
                                ],
                                shadow_guide,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(snapped_pos.x + px, screen_rect.min.y),
                                    Pos2::new(snapped_pos.x + px, screen_rect.max.y),
                                ],
                                shadow_guide,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(screen_rect.min.x, snapped_pos.y),
                                    Pos2::new(screen_rect.max.x, snapped_pos.y),
                                ],
                                light_guide,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(snapped_pos.x, screen_rect.min.y),
                                    Pos2::new(snapped_pos.x, screen_rect.max.y),
                                ],
                                light_guide,
                            );
                        }

                        let img_pt = to_image(pos);
                        let hud = if let Some(sel) = active_region {
                            format!(
                                "{} × {}",
                                sel.width.round() as u32,
                                sel.height.round() as u32,
                            )
                        } else if let Some(snap_rect) = target_snap_rect {
                            format!(
                                "{} × {}",
                                snap_rect.width.round() as u32,
                                snap_rect.height.round() as u32,
                            )
                        } else if !capture_mode || self.active_tool == OverlayTool::Region {
                            format!(
                                "{}, {}",
                                self.origin_x + img_pt.x.floor() as i32,
                                self.origin_y + img_pt.y.floor() as i32,
                            )
                        } else {
                            format!(
                                "{} · {}, {}",
                                self.active_tool.label(),
                                self.origin_x + img_pt.x.round() as i32,
                                self.origin_y + img_pt.y.round() as i32,
                            )
                        };

                        let galley = painter.layout_no_wrap(
                            hud,
                            FontId::monospace(11.5_f32),
                            Color32::from_rgb(242, 245, 250),
                        );
                        let badge_size = galley.size() + Vec2::new(14.0, 7.0);
                        let clamp_badge = |top_left: Pos2| {
                            top_left
                                .min(screen_rect.max - badge_size)
                                .max(screen_rect.min)
                        };
                        let mut badge_pos = clamp_badge(pos + Vec2::new(14.0, 16.0));
                        if Rect::from_min_size(badge_pos, badge_size).intersects(toolbar_rect) {
                            let below = toolbar_rect.max.y + 8.0;
                            let above = toolbar_rect.min.y - badge_size.y - 8.0;
                            badge_pos = clamp_badge(Pos2::new(
                                badge_pos.x,
                                if below + badge_size.y <= screen_rect.max.y {
                                    below
                                } else {
                                    above
                                },
                            ));
                        }
                        let badge_rect = Rect::from_min_size(badge_pos, badge_size);
                        painter.rect_filled(badge_rect, CornerRadius::same(6), glass_bg());
                        painter.rect_stroke(
                            badge_rect,
                            CornerRadius::same(6),
                            Stroke::new(1.0_f32, glass_border()),
                            egui::StrokeKind::Inside,
                        );
                        painter.galley(badge_pos + Vec2::new(7.0, 3.5), galley, Color32::WHITE);

                        if color_picker || self.show_magnifier {
                            let image = self.document.base_image();
                            let x = (img_pt.x.floor() as u32).min(image.width() - 1);
                            let y = (img_pt.y.floor() as u32).min(image.height() - 1);
                            let Ok(ColorSample { r, g, b, .. }) = sample_image(image, x, y) else {
                                return;
                            };
                            let loupe = compute_loupe_uv_rect_with_extent(
                                [x, y],
                                [image.width(), image.height()],
                                self.loupe_half_extent,
                            );
                            let hex = format_picked_color(r, g, b);
                            if !typing
                                && overlay_ctx
                                    .input(|i| !i.modifiers.any() && i.key_pressed(egui::Key::C))
                            {
                                overlay_ctx.copy_text(hex.clone());
                            }
                            let mut readout = format!("{hex}\n{r}, {g}, {b}");
                            if color_picker {
                                let (hue, saturation, lightness) =
                                    ColorSample::rgb(r, g, b).to_hsl();
                                readout.push_str(&format!(
                                    "\n{hue:.0}° {saturation:.0}% {lightness:.0}%"
                                ));
                            }
                            paint_magnifier(
                                &painter,
                                pos,
                                screen_rect,
                                [toolbar_rect, badge_rect],
                                self.texture.id(),
                                LoupeView {
                                    region: loupe,
                                    half_extent: self.loupe_half_extent,
                                    swatch: Color32::from_rgb(r, g, b),
                                    footer: &readout,
                                },
                            );
                        }
                    }
                });

            if let Some(act) = toolbar_action {
                outcome = act;
                close_overlay_viewport(overlay_ctx);
            }
        });

        if !matches!(outcome, OverlayModeOutcome::Pending) {
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Visible(false));
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Close);
            ctx.request_repaint();
        } else if !self.overlay_revealed {
            self.overlay_revealed = true;
            ctx.send_viewport_cmd_to(viewport_id, egui::ViewportCommand::Visible(true));
            ctx.request_repaint();
        }

        outcome
    }

    fn fit_viewport_to_snapshot(
        &mut self,
        ctx: &egui::Context,
        desired_size: Vec2,
        inner: Option<Rect>,
        outer: Option<Rect>,
    ) {
        let ppp = ctx.pixels_per_point();
        let desired_origin = Pos2::new(self.origin_x as f32 / ppp, self.origin_y as f32 / ppp);
        let Some(inner) = inner else {
            return;
        };
        let offset = desired_origin - inner.min;
        let needs_move = exceeds_half_pixel(offset, ppp);
        let needs_resize = exceeds_half_pixel(desired_size - inner.size(), ppp);
        if needs_move && let Some(outer) = outer {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(outer.min + offset));
        }
        if needs_resize {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired_size));
        }
        self.viewport_fitted = !needs_move && !needs_resize;
    }

    fn render_floating_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        accent: Color32,
        toolbar_action: &mut Option<OverlayModeOutcome>,
    ) {
        let show_options = self.show_style_bar || self.active_tool != OverlayTool::Region;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(2.0_f32, 6.0_f32);
            ui.horizontal(|ui| {
                for (index, tool) in OverlayTool::ALL.into_iter().enumerate() {
                    if index == 2 {
                        paint_toolbar_divider(ui);
                    }
                    let selected = self.active_tool == tool;
                    if toolbar_icon_button(ui, tool.icon(), tool.tooltip(), selected, accent)
                        .clicked()
                    {
                        self.active_tool = tool;
                        self.drag = None;
                    }
                }

                paint_toolbar_divider(ui);

                let (swatch_rect, swatch_resp) =
                    ui.allocate_exact_size(Vec2::splat(28.0_f32), Sense::click());
                if swatch_resp.hovered() || self.show_style_bar {
                    ui.painter().rect_filled(
                        swatch_rect,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 22),
                    );
                }
                ui.painter().circle_filled(
                    swatch_rect.center(),
                    7.0_f32,
                    to_egui_color(self.color),
                );
                ui.painter().circle_stroke(
                    swatch_rect.center(),
                    7.5_f32,
                    Stroke::new(
                        1.25_f32,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 210),
                    ),
                );
                if swatch_resp.on_hover_text("Color & tool options").clicked() {
                    self.show_style_bar = !self.show_style_bar;
                }

                paint_toolbar_divider(ui);

                let toggle_bg = Color32::from_rgba_unmultiplied(255, 255, 255, 34);
                if toolbar_icon_button(
                    ui,
                    Icon::SnapMagnet,
                    "Snap to visible windows",
                    self.snap_to_windows,
                    toggle_bg,
                )
                .clicked()
                {
                    self.snap_to_windows = !self.snap_to_windows;
                }
                if toolbar_icon_button(
                    ui,
                    Icon::Loupe,
                    "Pixel magnifier (press C to copy HEX)",
                    self.show_magnifier,
                    toggle_bg,
                )
                .clicked()
                {
                    self.show_magnifier = !self.show_magnifier;
                }

                paint_toolbar_divider(ui);

                if overlay_action_icon_button(
                    ui,
                    Icon::Undo,
                    "Undo (Ctrl+Z)",
                    self.document.can_undo(),
                    None,
                )
                .clicked()
                {
                    self.document.undo();
                }
                if overlay_action_icon_button(
                    ui,
                    Icon::Redo,
                    "Redo (Ctrl+Y)",
                    self.document.can_redo(),
                    None,
                )
                .clicked()
                {
                    self.document.redo();
                }
                if self.document.selected_id().is_some()
                    && overlay_action_icon_button(
                        ui,
                        Icon::Trash,
                        "Delete selected / hovered (Del / Backspace)",
                        true,
                        Some(Color32::from_rgb(239, 68, 68)),
                    )
                    .clicked()
                {
                    self.document.delete_selected();
                }

                paint_toolbar_divider(ui);

                if overlay_action_icon_button(
                    ui,
                    Icon::Check,
                    "Capture now (Enter)",
                    true,
                    Some(Color32::from_rgb(34, 197, 94)),
                )
                .clicked()
                {
                    *toolbar_action = self.finalize_selection(false);
                }
                if overlay_action_icon_button(ui, Icon::Editor, "Open in Full Editor", true, None)
                    .clicked()
                {
                    *toolbar_action = self.finalize_selection(true);
                }
                if overlay_action_icon_button(
                    ui,
                    Icon::Close,
                    "Cancel (Esc / Right-click)",
                    true,
                    Some(Color32::from_rgb(239, 68, 68)),
                )
                .clicked()
                {
                    *toolbar_action = Some(OverlayModeOutcome::Cancelled);
                }
            });

            if show_options {
                let (sep_rect, _) = ui
                    .allocate_exact_size(Vec2::new(ui.available_width(), 1.0_f32), Sense::hover());
                ui.painter().line_segment(
                    [sep_rect.left_center(), sep_rect.right_center()],
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0_f32;
                    for swatch in SHAREX_PALETTE {
                        let is_current = self.color == swatch;
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::splat(16.0_f32), Sense::click());
                        ui.painter()
                            .circle_filled(rect.center(), 6.5_f32, to_egui_color(swatch));
                        if is_current {
                            ui.painter().circle_stroke(
                                rect.center(),
                                7.5_f32,
                                Stroke::new(1.5_f32, Color32::WHITE),
                            );
                        } else if resp.hovered() {
                            ui.painter().circle_stroke(
                                rect.center(),
                                7.5_f32,
                                Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(255, 255, 255, 130),
                                ),
                            );
                        }
                        if resp.clicked() {
                            self.color = swatch;
                        }
                    }

                    paint_toolbar_divider(ui);

                    match self.active_tool {
                        OverlayTool::Text => {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.text_draft)
                                    .desired_width(110.0_f32),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.font_size)
                                    .range(12.0_f32..=56.0_f32)
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
                                    .size(12.0_f32)
                                    .monospace()
                                    .color(Color32::WHITE),
                            );
                        }
                        _ => {
                            ui.add(
                                egui::DragValue::new(&mut self.stroke_width)
                                    .range(1.0_f32..=24.0_f32)
                                    .suffix(" px"),
                            );
                        }
                    }

                    paint_toolbar_divider(ui);

                    ui.checkbox(&mut self.auto_capture_on_drag, "Auto-capture")
                        .on_hover_text("Finish immediately when releasing a region drag");
                });
            }
        });
    }

    fn snapped_window_at(&self, point: Point) -> Option<(&WindowInfo, RectF)> {
        if self.active_tool != OverlayTool::Region || !self.snap_to_windows {
            return None;
        }

        let image = self.document.base_image();
        let window = find_window_at_point(
            self.origin_x + (point.x.floor() as i32).min(image.width() as i32 - 1),
            self.origin_y + (point.y.floor() as i32).min(image.height() as i32 - 1),
            &self.windows,
        )?;
        let rect = clipped_window_rect(
            window,
            self.origin_x,
            self.origin_y,
            image.width(),
            image.height(),
        )?;
        Some((window, rect))
    }

    fn handle_region_input(
        &mut self,
        events: &[egui::Event],
        is_canvas: &dyn Fn(Pos2) -> bool,
        to_image: &dyn Fn(Pos2) -> Point,
    ) -> Option<RectF> {
        for event in events {
            match *event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if is_canvas(pos) => {
                    self.drag = Some(OverlayDrag::RegionPress {
                        start: to_image(pos),
                    });
                }
                egui::Event::PointerMoved(pos) => self.move_region_pointer(to_image(pos)),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    let point = to_image(pos);
                    self.move_region_pointer(point);
                    match self.drag.take() {
                        Some(OverlayDrag::RegionPress { .. }) if is_canvas(pos) => {
                            if let Some((_, rect)) = self.snapped_window_at(point) {
                                return Some(rect);
                            }
                        }
                        Some(OverlayDrag::RegionSelect { start, current }) => {
                            let rect = RectF::from_points(start, current);
                            if rect.width >= MIN_REGION_DRAG && rect.height >= MIN_REGION_DRAG {
                                return Some(rect);
                            }
                        }
                        _ => {}
                    }
                }
                egui::Event::WindowFocused(false) => self.drag = None,
                _ => {}
            }
        }
        None
    }

    fn move_region_pointer(&mut self, point: Point) {
        let image = self.document.base_image();
        let clipped = Point::new(
            point.x.clamp(0.0, image.width() as f32),
            point.y.clamp(0.0, image.height() as f32),
        );
        match &mut self.drag {
            Some(OverlayDrag::RegionPress { start })
                if (start.x - point.x).hypot(start.y - point.y) >= MIN_REGION_DRAG =>
            {
                self.drag = Some(OverlayDrag::RegionSelect {
                    start: *start,
                    current: clipped,
                });
            }
            Some(OverlayDrag::RegionSelect { current, .. }) => *current = clipped,
            _ => {}
        }
    }

    fn handle_annotation_input(
        &mut self,
        response: &egui::Response,
        to_image: &dyn Fn(Pos2) -> Point,
    ) {
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
                OverlayTool::Region | OverlayTool::Text | OverlayTool::StepNumber => {}
            }
        }

        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pos) = response.interact_pointer_pos()
        {
            let pt = to_image(pos);
            match &mut self.drag {
                Some(OverlayDrag::Shape { current, .. }) => {
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
                _ => {}
            }
        }

        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(draft) = self.drag.take()
        {
            match draft {
                OverlayDrag::RegionPress { .. }
                | OverlayDrag::RegionSelect { .. }
                | OverlayDrag::MoveSelected { .. } => {}
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
                        return;
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
            OverlayDrag::RegionPress { .. }
            | OverlayDrag::RegionSelect { .. }
            | OverlayDrag::MoveSelected { .. } => {}
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
                        let _ = accent;
                        painter.rect_filled(sr, 0.0, Color32::from_black_alpha(68));
                        paint_dashed_border(
                            painter,
                            sr,
                            Color32::WHITE,
                            Color32::from_black_alpha(220),
                            5.0_f32,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn apply_scroll_step(&mut self, step: i32, color_picker: bool) {
        if color_picker || self.active_tool == OverlayTool::Region {
            self.loupe_half_extent = adjust_loupe_half_extent(self.loupe_half_extent, step);
            return;
        }
        match self.active_tool {
            OverlayTool::Pixelate | OverlayTool::Blur => {
                self.effect_strength = adjust_effect_strength(self.effect_strength, step);
            }
            OverlayTool::Text | OverlayTool::StepNumber => {
                self.font_size = (self.font_size + step as f32 * 2.0_f32).clamp(12.0_f32, 56.0_f32);
            }
            _ => {
                self.stroke_width = (self.stroke_width + step as f32).clamp(1.0_f32, 24.0_f32);
            }
        }
    }

    fn finalize_selection(&self, open_in_editor: bool) -> Option<OverlayModeOutcome> {
        self.mode.finish_region(
            &self.document,
            self.selection,
            (self.origin_x, self.origin_y),
            open_in_editor,
        )
    }
}

fn toolbar_anchor_in_overlay(
    origin_x: i32,
    origin_y: i32,
    screen_rect: Rect,
    scale_x: f32,
    scale_y: f32,
) -> Pos2 {
    let zero_x = screen_rect.min.x - origin_x as f32 * scale_x;
    let zero_y = screen_rect.min.y - origin_y as f32 * scale_y;
    let left = if zero_x < screen_rect.max.x {
        zero_x.max(screen_rect.min.x)
    } else {
        screen_rect.min.x
    };
    let top = if zero_y + 10.0 < screen_rect.max.y {
        zero_y.max(screen_rect.min.y)
    } else {
        screen_rect.min.y
    };
    Pos2::new((left + screen_rect.max.x) * 0.5, top + 10.0)
}

fn close_overlay_viewport(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

fn delete_hovered_or_selected(document: &mut EditorDocument, cursor: Option<Point>) -> bool {
    if let Some(point) = cursor
        && document.select_at(point, 14.0).is_some()
    {
        return document.delete_selected();
    }
    document.delete_selected()
}

fn format_picked_color(r: u8, g: u8, b: u8) -> String {
    ColorSample::rgb(r, g, b).to_hex()
}

fn exceeds_half_pixel(offset: Vec2, pixels_per_point: f32) -> bool {
    let tolerance = 0.5 / pixels_per_point;
    offset.x.abs() > tolerance || offset.y.abs() > tolerance
}

fn clipped_window_rect(
    window: &WindowInfo,
    origin_x: i32,
    origin_y: i32,
    image_width: u32,
    image_height: u32,
) -> Option<RectF> {
    let x = i64::from(window.x) - i64::from(origin_x);
    let y = i64::from(window.y) - i64::from(origin_y);
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + i64::from(window.width)).min(i64::from(image_width));
    let bottom = (y + i64::from(window.height)).min(i64::from(image_height));
    (left < right && top < bottom).then(|| {
        RectF::from_points(
            Point::new(left as f32, top as f32),
            Point::new(right as f32, bottom as f32),
        )
    })
}

fn paint_toolbar_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(7.0_f32, 20.0_f32), Sense::hover());
    ui.painter().line_segment(
        [
            Pos2::new(rect.center().x, rect.top() + 2.0_f32),
            Pos2::new(rect.center().x, rect.bottom() - 2.0_f32),
        ],
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 24)),
    );
}

fn overlay_action_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    enabled: bool,
    tint: Option<Color32>,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0_f32), sense);
    let hovered = enabled && response.hovered();
    let bg = if hovered {
        tint.map_or(Color32::from_rgba_unmultiplied(255, 255, 255, 24), |c| {
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 44)
        })
    } else if enabled && icon == Icon::Check {
        Color32::from_rgba_unmultiplied(34, 197, 94, 34)
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }
    let fg = if !enabled {
        Color32::from_rgba_unmultiplied(148, 163, 184, 72)
    } else if hovered {
        tint.unwrap_or(Color32::WHITE)
    } else if icon == Icon::Check {
        Color32::from_rgb(74, 222, 128)
    } else {
        Color32::from_rgb(184, 194, 210)
    };
    paint_icon(ui.painter(), rect.shrink(6.0_f32), icon, fg);
    response.on_hover_text(tooltip)
}

fn paint_selection_handles(painter: &egui::Painter, bounds: Rect) {
    let half = 3.5_f32;
    let points = [
        bounds.left_top(),
        bounds.right_top(),
        bounds.left_bottom(),
        bounds.right_bottom(),
        Pos2::new(bounds.center().x, bounds.top()),
        Pos2::new(bounds.center().x, bounds.bottom()),
        Pos2::new(bounds.left(), bounds.center().y),
        Pos2::new(bounds.right(), bounds.center().y),
    ];
    for center in points {
        let handle = Rect::from_center_size(center, Vec2::splat(half * 2.0_f32));
        painter.rect_filled(handle, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(
            handle,
            CornerRadius::same(1),
            Stroke::new(1.0_f32, Color32::from_black_alpha(210)),
            egui::StrokeKind::Outside,
        );
    }
}

fn paint_snapped_window(
    painter: &egui::Painter,
    bounds: Rect,
    screen_rect: Rect,
    toolbar_rect: Rect,
    window: &WindowInfo,
) {
    paint_dashed_border(
        painter,
        bounds,
        Color32::WHITE,
        Color32::from_black_alpha(225),
        5.0_f32,
    );

    let text = match (window.app_name.as_str(), window.title.as_str()) {
        ("", "") => return,
        ("", title) => title.into(),
        (app, title) if title.is_empty() || app == title => app.into(),
        (app, title) => format!("{app} · {title}"),
    };
    let padding = Vec2::new(8.0_f32, 4.0_f32);
    let max_width = (bounds.width().min(420.0_f32) - 2.0_f32 * padding.x).max(60.0_f32);
    let mut label = egui::text::LayoutJob::simple(
        text,
        FontId::proportional(11.5_f32),
        Color32::from_rgb(238, 242, 248),
        max_width,
    );
    label.wrap.max_rows = 1;
    let galley = painter.layout_job(label);
    let size = galley.size() + 2.0_f32 * padding;
    let mut pos = (bounds.min + Vec2::splat(8.0_f32))
        .min(screen_rect.max - size - Vec2::splat(4.0_f32))
        .max(screen_rect.min + Vec2::splat(4.0_f32));
    if Rect::from_min_size(pos, size).intersects(toolbar_rect) {
        pos.y = (toolbar_rect.max.y + 8.0_f32).min(screen_rect.max.y - size.y - 4.0_f32);
    }
    let chip = Rect::from_min_size(pos, size);
    painter.rect_filled(chip, CornerRadius::same(6), glass_bg());
    painter.rect_stroke(
        chip,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, glass_border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(pos + padding, galley, Color32::WHITE);
}

fn paint_dashed_border(
    painter: &egui::Painter,
    bounds: Rect,
    light_color: Color32,
    dark_color: Color32,
    dash_pixels: f32,
) {
    let pixels_per_point = painter.pixels_per_point();
    let pixel = 1.0_f32 / pixels_per_point;
    let bounds = Rect::from_min_max(
        bounds.min.round_to_pixels(pixels_per_point),
        bounds.max.round_to_pixels(pixels_per_point),
    );
    let mut border = egui::Mesh::default();
    for horizontal in [true, false] {
        let (mut start, end) = if horizontal {
            (bounds.left(), bounds.right())
        } else {
            (bounds.top() + pixel, bounds.bottom() - pixel)
        };
        let mut light = true;
        while start < end {
            let stop = (start + dash_pixels * pixel).min(end);
            let color = if light { light_color } else { dark_color };
            let strips = if horizontal {
                [bounds.top(), bounds.bottom() - pixel]
                    .map(|y| Rect::from_min_max(Pos2::new(start, y), Pos2::new(stop, y + pixel)))
            } else {
                [bounds.left(), bounds.right() - pixel]
                    .map(|x| Rect::from_min_max(Pos2::new(x, start), Pos2::new(x + pixel, stop)))
            };
            for strip in strips {
                border.add_colored_rect(strip, color);
            }
            start = stop;
            light = !light;
        }
    }
    painter.add(border);
}

#[cfg(test)]
fn sample_magnifier_grid(
    image: &CapturedImage,
    center_x: i32,
    center_y: i32,
    half_extent: i32,
) -> (Vec<[u8; 4]>, [u8; 4]) {
    let half_extent = half_extent.max(0);
    let sample = |dx, dy| {
        let x = (i64::from(center_x) + i64::from(dx)).clamp(0, i64::from(image.width() - 1));
        let y = (i64::from(center_y) + i64::from(dy)).clamp(0, i64::from(image.height() - 1));
        let offset = (y as usize * image.width() as usize + x as usize) * 4;
        image.rgba()[offset..offset + 4].try_into().unwrap()
    };
    let side = half_extent as usize * 2 + 1;
    let mut grid = Vec::with_capacity(side * side);
    for dy in -half_extent..=half_extent {
        for dx in -half_extent..=half_extent {
            grid.push(sample(dx, dy));
        }
    }
    (grid, sample(0, 0))
}

struct LoupeRegion {
    uv_rect: Rect,
    grid_cells: Rect,
}

fn adjust_loupe_half_extent(current: i32, step: i32) -> i32 {
    (current + step).clamp(MIN_LOUPE_HALF_EXTENT, MAX_LOUPE_HALF_EXTENT)
}

fn adjust_effect_strength(current: u32, step: i32) -> u32 {
    (current as i32 + step * 2).clamp(4, 32) as u32
}

#[cfg(test)]
fn compute_loupe_uv_rect(center: [u32; 2], size: [u32; 2]) -> LoupeRegion {
    compute_loupe_uv_rect_with_extent(center, size, MAGNIFIER_HALF_EXTENT)
}

fn compute_loupe_uv_rect_with_extent(
    center: [u32; 2],
    size: [u32; 2],
    half_extent: i32,
) -> LoupeRegion {
    let half = half_extent.clamp(1, MAX_LOUPE_HALF_EXTENT) as u32;
    let axis = |center: u32, size: u32| {
        let center = center.min(size - 1);
        let min = center.saturating_sub(half);
        let max = center.saturating_add(half + 1).min(size);
        let grid_start = (half - (center - min)) as f32;
        (
            min as f32 / size as f32,
            max as f32 / size as f32,
            grid_start,
            grid_start + (max - min) as f32,
        )
    };
    let (u_min, u_max, x_min, x_max) = axis(center[0], size[0]);
    let (v_min, v_max, y_min, y_max) = axis(center[1], size[1]);
    LoupeRegion {
        uv_rect: Rect::from_min_max(Pos2::new(u_min, v_min), Pos2::new(u_max, v_max)),
        grid_cells: Rect::from_min_max(Pos2::new(x_min, y_min), Pos2::new(x_max, y_max)),
    }
}

struct LoupeView<'a> {
    region: LoupeRegion,
    half_extent: i32,
    swatch: Color32,
    footer: &'a str,
}

fn paint_magnifier(
    painter: &egui::Painter,
    cursor: Pos2,
    screen_rect: Rect,
    obstacles: [Rect; 2],
    texture: egui::TextureId,
    view: LoupeView<'_>,
) {
    let half_extent = view
        .half_extent
        .clamp(MIN_LOUPE_HALF_EXTENT, MAX_LOUPE_HALF_EXTENT);
    let side = (half_extent * 2 + 1) as usize;
    let pixels_per_point = painter.pixels_per_point();
    let physical_pixel = 1.0_f32 / pixels_per_point;
    let cell_size = 10.0 * physical_pixel;
    let grid_size = side as f32 * cell_size;
    let inset = 4.0_f32;
    let swatch_side = 10.0_f32;
    let swatch_gap = 6.0_f32;
    let galley = painter.layout_no_wrap(
        view.footer.into(),
        FontId::monospace(10.5_f32),
        Color32::from_rgb(232, 238, 246),
    );
    let footer_row_w = swatch_side + swatch_gap + galley.size().x;
    let padding = Vec2::new(6.0_f32, 5.0_f32);
    let size = Vec2::new(
        (grid_size + 2.0_f32 * inset).max(footer_row_w + 2.0_f32 * padding.x),
        inset + grid_size + galley.size().y + 2.0_f32 * padding.y,
    );
    let right = cursor.x + 16.0;
    let left = cursor.x - 16.0 - size.x;
    let below = cursor.y + 16.0;
    let above = cursor.y - 16.0 - size.y;
    let candidates = [
        Pos2::new(left, above),
        Pos2::new(right, above),
        Pos2::new(left, below),
        Pos2::new(right, below),
    ];
    let positions = candidates.map(|pos| {
        pos.round_to_pixels(pixels_per_point)
            .min(screen_rect.max - size)
            .max(screen_rect.min)
    });
    let pos = positions
        .into_iter()
        .find(|&pos| {
            let panel = Rect::from_min_size(pos, size);
            !panel.contains(cursor) && !obstacles.iter().any(|rect| panel.intersects(*rect))
        })
        .or_else(|| {
            positions
                .into_iter()
                .find(|&pos| !Rect::from_min_size(pos, size).contains(cursor))
        })
        .unwrap_or(positions[0]);
    let panel = Rect::from_min_size(pos, size);
    painter.rect_filled(panel, CornerRadius::same(8), glass_bg());

    let grid_origin =
        (pos + Vec2::new((size.x - grid_size) * 0.5, inset)).round_to_pixels(pixels_per_point);
    let full_grid_rect = Rect::from_min_size(grid_origin, Vec2::splat(grid_size));
    let loupe_grid_rect = Rect::from_min_max(
        grid_origin + view.region.grid_cells.min.to_vec2() * cell_size,
        grid_origin + view.region.grid_cells.max.to_vec2() * cell_size,
    );
    painter.rect_filled(
        full_grid_rect,
        CornerRadius::same(3),
        Color32::from_rgb(10, 12, 16),
    );
    painter.image(
        texture,
        loupe_grid_rect,
        view.region.uv_rect,
        Color32::WHITE,
    );
    painter.rect_stroke(
        full_grid_rect,
        CornerRadius::same(2),
        Stroke::new(
            physical_pixel,
            Color32::from_rgba_unmultiplied(255, 255, 255, 36),
        ),
        egui::StrokeKind::Inside,
    );
    let center = Rect::from_min_size(
        grid_origin + Vec2::splat(half_extent as f32 * cell_size),
        Vec2::splat(cell_size),
    );
    painter.rect_stroke(
        center,
        0.0,
        Stroke::new(physical_pixel, Color32::from_black_alpha(230)),
        egui::StrokeKind::Outside,
    );
    painter.rect_stroke(
        center,
        0.0,
        Stroke::new(physical_pixel, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        panel,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, glass_border()),
        egui::StrokeKind::Inside,
    );
    let footer_top = pos.y + inset + grid_size + padding.y;
    let footer_left = panel.center().x - footer_row_w * 0.5;
    let swatch_rect = Rect::from_min_size(
        Pos2::new(footer_left, footer_top + 2.0_f32),
        Vec2::splat(swatch_side),
    );
    painter.rect_filled(swatch_rect, CornerRadius::same(3), view.swatch);
    painter.rect_stroke(
        swatch_rect,
        CornerRadius::same(3),
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 175)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        Pos2::new(footer_left + swatch_side + swatch_gap, footer_top),
        galley,
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sharex_core::WindowId;

    const PIXELS: [[u8; 4]; 4] = [
        [1, 2, 3, 4],
        [5, 6, 7, 8],
        [9, 10, 11, 12],
        [13, 14, 15, 16],
    ];

    fn sample_image() -> CapturedImage {
        CapturedImage::from_rgba(2, 2, PIXELS.concat()).unwrap()
    }

    #[test]
    fn toolbar_centers_on_a_single_monitor() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
        assert_eq!(
            toolbar_anchor_in_overlay(0, 0, screen, 1.0, 1.0),
            Pos2::new(960.0, 10.0),
        );
    }

    #[test]
    fn toolbar_skips_the_bezel_on_a_left_extended_desktop() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(3840.0, 1080.0));
        assert_eq!(
            toolbar_anchor_in_overlay(-1920, 0, screen, 1.0, 1.0),
            Pos2::new(2880.0, 10.0),
        );
    }

    #[test]
    fn toolbar_maps_negative_origins_into_scaled_overlay_points() {
        let screen = Rect::from_min_size(Pos2::new(5.0, 15.0), Vec2::new(1920.0, 1080.0));
        assert_eq!(
            toolbar_anchor_in_overlay(-1920, -1080, screen, 0.5, 0.5),
            Pos2::new(1445.0, 565.0),
        );
    }

    #[test]
    fn modes_default_to_capture_and_describe_the_requested_action() {
        assert_eq!(OverlayMode::default(), OverlayMode::Capture);
        assert_eq!(OverlayMode::Capture.hint(), "Capture Region");
        assert!(OverlayMode::Ocr.hint().starts_with("OCR Region:"));
        assert!(
            OverlayMode::SelectRecordingRegion
                .hint()
                .starts_with("Select Recording Region:")
        );
        assert!(OverlayMode::ColorPicker.hint().starts_with("Color Picker:"));
    }

    #[test]
    fn mode_outcomes_crop_ocr_and_return_global_recording_bounds() {
        let document = EditorDocument::new(sample_image());
        let selection = Some(RectF::from_points(
            Point::new(0.0, 1.0),
            Point::new(2.0, 2.0),
        ));
        let origin = (-1, -100);
        let expected_region = Region::new(-1, -99, 2, 1).unwrap();

        let Some(OverlayModeOutcome::OcrRegion {
            image,
            global_region,
        }) = OverlayMode::Ocr.finish_region(&document, selection, origin, false)
        else {
            panic!("expected a cropped OCR region");
        };
        assert_eq!(global_region, expected_region);
        assert_eq!(image.rgba(), PIXELS[2..].concat());

        assert!(matches!(
            OverlayMode::SelectRecordingRegion.finish_region(&document, selection, origin, false),
            Some(OverlayModeOutcome::RecordingRegion(region)) if region == expected_region
        ));
        for mode in [
            OverlayMode::Ocr,
            OverlayMode::SelectRecordingRegion,
            OverlayMode::ColorPicker,
        ] {
            assert!(mode.finish_region(&document, None, origin, false).is_none());
        }
        assert!(
            OverlayMode::ColorPicker
                .finish_region(&document, selection, origin, false)
                .is_none()
        );

        let Some(OverlayModeOutcome::Captured {
            image,
            global_region,
            open_in_editor: true,
        }) = OverlayMode::Capture.finish_region(&document, None, origin, true)
        else {
            panic!("expected a full capture to open in the editor");
        };
        assert_eq!(global_region, Region::new(-1, -100, 2, 2).unwrap());
        assert_eq!(image.rgba(), document.base_image().rgba());
    }

    #[test]
    fn picked_color_formats_uppercase_zero_padded_hex() {
        assert_eq!(format_picked_color(0, 10, 255), "#000AFF");
        assert_eq!(format_picked_color(171, 205, 239), "#ABCDEF");
    }

    #[test]
    fn magnifier_returns_exact_center_pixel_and_grid_size() {
        let image = sample_image();
        let (grid, center) = sample_magnifier_grid(&image, 1, 0, 5);
        assert_eq!(grid.len(), 11 * 11);
        assert_eq!(center, PIXELS[1]);
        assert_eq!(grid[5 * 11 + 5], center);
        assert_eq!(
            sample_magnifier_grid(&image, 0, 1, 0),
            (vec![PIXELS[2]], PIXELS[2])
        );
    }

    #[test]
    fn loupe_uvs_clamp_to_edges_without_displacing_the_center_cell() {
        let top_left = compute_loupe_uv_rect([0, 0], [20, 40]);
        assert_eq!(
            top_left.uv_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(0.3, 0.15))
        );
        assert_eq!(
            top_left.grid_cells,
            Rect::from_min_max(Pos2::new(5.0, 5.0), Pos2::new(11.0, 11.0))
        );

        let bottom_right = compute_loupe_uv_rect([19, 39], [20, 40]);
        assert_eq!(
            bottom_right.uv_rect,
            Rect::from_min_max(Pos2::new(0.7, 0.85), Pos2::new(1.0, 1.0))
        );
        assert_eq!(
            bottom_right.grid_cells,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(6.0, 6.0))
        );

        let single_pixel = compute_loupe_uv_rect([0, 0], [1, 1]);
        assert_eq!(
            single_pixel.uv_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
        );
        assert_eq!(
            single_pixel.grid_cells,
            Rect::from_min_max(Pos2::new(5.0, 5.0), Pos2::new(6.0, 6.0))
        );
    }

    #[test]
    fn magnifier_repeats_edge_pixels_in_row_order() {
        let (grid, center) = sample_magnifier_grid(&sample_image(), 0, 0, 1);
        let [a, b, c, d] = PIXELS;
        assert_eq!(grid, [a, a, b, a, a, b, c, c, d]);
        assert_eq!(center, a);
        let (grid, center) = sample_magnifier_grid(&sample_image(), 2, 2, 1);
        assert_eq!(grid, [d; 9]);
        assert_eq!(center, d);
    }

    #[test]
    fn snapped_window_rect_clips_to_overlay_and_rejects_nonoverlap() {
        let mut window = WindowInfo {
            id: WindowId::new(1),
            title: "Document".into(),
            app_name: "Editor".into(),
            x: -120,
            y: -40,
            width: 90,
            height: 150,
            is_minimized: false,
            is_focused: true,
        };
        assert_eq!(
            clipped_window_rect(&window, -100, -50, 60, 120),
            Some(RectF {
                x: 0.0,
                y: 10.0,
                width: 60.0,
                height: 110.0
            })
        );
        window.y = -70;
        window.height = 20;
        assert_eq!(clipped_window_rect(&window, -100, -50, 60, 120), None);
    }

    #[test]
    fn delete_hovered_or_selected_removes_hovered_arrow_without_prior_selection() {
        let mut document = EditorDocument::new(sample_image());
        let first_id = document.push_annotation(AnnotationKind::Arrow {
            start: Point::new(0.0, 0.0),
            end: Point::new(100.0, 0.0),
            color: RgbaColor::RED,
            stroke_width: 4.0,
        });
        let second_id = document.push_annotation(AnnotationKind::Arrow {
            start: Point::new(0.0, 80.0),
            end: Point::new(100.0, 80.0),
            color: RgbaColor::RED,
            stroke_width: 4.0,
        });
        document.select_at(Point::new(50.0, 80.0), 10.0);
        assert_eq!(document.selected_id(), Some(second_id));

        assert!(delete_hovered_or_selected(
            &mut document,
            Some(Point::new(50.0, 2.0))
        ));
        assert_eq!(document.annotations().len(), 1);
        assert_eq!(document.annotations()[0].id, second_id);
        assert_ne!(document.annotations()[0].id, first_id);
    }

    #[test]
    fn scroll_steps_resize_loupe_and_effect_strength_within_bounds() {
        assert_eq!(adjust_loupe_half_extent(5, 1), 6);
        assert_eq!(adjust_loupe_half_extent(9, 1), MAX_LOUPE_HALF_EXTENT);
        assert_eq!(adjust_loupe_half_extent(3, -1), MIN_LOUPE_HALF_EXTENT);
        assert_eq!(adjust_effect_strength(12, 1), 14);
        assert_eq!(adjust_effect_strength(32, 1), 32);
        assert_eq!(adjust_effect_strength(4, -1), 4);
    }
}
