use crate::editor_ui::{paint_annotation, paint_arrow, paint_ellipse_stroke, to_egui_color};
use crate::win_focus::OverlayFocusGuard;
use eframe::egui::emath::GuiRounding;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use sharex_capture::{DesktopSnapshot, XcapCapturer, find_window_at_point};
use sharex_core::{CapturedImage, Region, ScreenCapturer, WindowInfo};
use sharex_editor::{AnnotationKind, EditorDocument, Point, RectF, RgbaColor, SHAREX_PALETTE};
use sharex_tools::{ColorSample, sample_image};

const OVERLAY_WINDOW_TITLE: &str = "ShareX Region Capture";
const SNAP_COLOR: Color32 = Color32::from_rgb(34, 197, 94);
const MIN_REGION_DRAG: f32 = 4.0;
const MAGNIFIER_HALF_EXTENT: i32 = 5;

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
    active_tool: OverlayTool,
    selection: Option<RectF>,
    auto_capture_on_drag: bool,
    windows: Vec<WindowInfo>,
    snap_to_windows: bool,
    show_magnifier: bool,
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
            capturer.list_windows().unwrap_or_default()
        };
        windows.retain(|window| !window.title.contains("ShareX"));
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
            active_tool: OverlayTool::Region,
            selection: None,
            auto_capture_on_drag: true,
            windows,
            snap_to_windows: true,
            show_magnifier: true,
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

        let builder = egui::ViewportBuilder::default()
            .with_title(OVERLAY_WINDOW_TITLE)
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_clamp_size_to_monitor_size(false)
            .with_position([self.origin_x as f32 / ppp, self.origin_y as f32 / ppp])
            .with_inner_size([img_w / ppp, img_h / ppp])
            .with_active(true);

        let mut outcome = OverlayModeOutcome::Pending;

        ctx.show_viewport_immediate(viewport_id, builder, |overlay_ctx, class| {
            if class != egui::ViewportClass::Embedded {
                self.fit_viewport_to_snapshot(overlay_ctx, Vec2::new(img_w, img_h));
            }
            self.focus_guard.enforce_overlay_focus(OVERLAY_WINDOW_TITLE);
            let typing = overlay_ctx.wants_keyboard_input();
            let capture_mode = self.mode == OverlayMode::Capture;
            let color_picker = self.mode == OverlayMode::ColorPicker;

            if overlay_ctx.input(|i| i.viewport().close_requested())
                || overlay_ctx.input(|i| i.key_pressed(egui::Key::Escape))
            {
                outcome = OverlayModeOutcome::Cancelled;
                return;
            }

            if capture_mode && overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
                self.document.undo();
            }
            if capture_mode && overlay_ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
                self.document.redo();
            }
            if capture_mode && !typing && overlay_ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.document.delete_selected();
            }
            if capture_mode && !typing && overlay_ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(captured) = self.finalize_selection(false) {
                    outcome = captured;
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
                    let primary_bounds = self.primary_monitor.map(to_screen_rect).filter(|rect| rect.is_positive());
                    let toolbar_anchor = if let Some(bounds) = primary_bounds {
                        toolbar_anchor_in_overlay(0, 0, bounds, scale_x, scale_y)
                    } else {
                        toolbar_anchor_in_overlay(self.origin_x, self.origin_y, screen_rect, scale_x, scale_y)
                    };
                    let toolbar_rect = egui::Area::new(egui::Id::new("sharex_region_floating_bar"))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::CENTER_TOP)
                        .fixed_pos(toolbar_anchor)
                        .constrain_to(primary_bounds.unwrap_or(screen_rect))
                        .interactable(capture_mode)
                        .show(overlay_ctx, |ui| {
                            egui::Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(24, 28, 34, 242))
                                .stroke(Stroke::new(1.0_f32, accent))
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    if capture_mode {
                                        self.render_floating_toolbar(ui, accent, &mut toolbar_action);
                                    } else {
                                        ui.label(RichText::new(self.mode.hint()).color(Color32::WHITE));
                                    }
                                });
                        })
                        .response
                        .rect;

                    let is_canvas = |pos: Pos2| {
                        screen_rect.contains(pos) && (!capture_mode || !toolbar_rect.contains(pos))
                    };
                    let pointer_pos = overlay_ctx.input(|i| i.pointer.hover_pos()).filter(|&pos| is_canvas(pos));

                    if pointer_pos.is_some() {
                        overlay_ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                        if response.secondary_clicked() {
                            if self.active_tool == OverlayTool::Region {
                                outcome = OverlayModeOutcome::Cancelled;
                                return;
                            }
                            self.active_tool = OverlayTool::Region;
                            self.drag = None;
                        }
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
                    let snapped_window = if matches!(self.drag, Some(OverlayDrag::RegionSelect { .. })) {
                        None
                    } else {
                        pointer_pos.and_then(|pos| self.snapped_window_at(to_image(pos)))
                    };

                    let scrim = Color32::from_black_alpha(125);
                    if let Some(sel_img) = active_region.or(snapped_window.map(|(_, rect)| rect)) {
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
                        }
                    } else if !color_picker && self.active_tool == OverlayTool::Region {
                        painter.rect_filled(screen_rect, 0.0, scrim);
                    }

                    if let Some(draft) = &self.drag {
                        self.paint_draft(&painter, draft, &to_screen, zoom, accent);
                    }

                    if let Some(pos) = pointer_pos {
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

                        if let Some((window, rect)) = snapped_window {
                            paint_snapped_window(&painter, to_screen_rect(rect), screen_rect, window);
                        }

                        let img_pt = to_image(pos);
                        let hud = if !capture_mode {
                            format!(
                                "X: {}  Y: {}",
                                self.origin_x + img_pt.x.floor() as i32,
                                self.origin_y + img_pt.y.floor() as i32,
                            )
                        } else if let Some(sel) = active_region {
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
                        let badge_size = galley.size() + Vec2::new(14.0, 8.0);
                        let clamp_badge = |top_left: Pos2| {
                            top_left.min(screen_rect.max - badge_size).max(screen_rect.min)
                        };
                        let mut badge_pos = clamp_badge(pos + Vec2::new(16.0, 18.0));
                        if Rect::from_min_size(badge_pos, badge_size).intersects(toolbar_rect) {
                            let below = toolbar_rect.max.y + 8.0;
                            let above = toolbar_rect.min.y - badge_size.y - 8.0;
                            badge_pos = clamp_badge(Pos2::new(
                                badge_pos.x,
                                if below + badge_size.y <= screen_rect.max.y { below } else { above },
                            ));
                        }
                        let badge_rect = Rect::from_min_size(badge_pos, badge_size);
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

                        if color_picker || self.show_magnifier {
                            let image = self.document.base_image();
                            let (grid, rgba) = sample_magnifier_grid(
                                image,
                                img_pt.x.floor() as i32,
                                img_pt.y.floor() as i32,
                                MAGNIFIER_HALF_EXTENT,
                            );
                            let [r, g, b, _] = rgba;
                            let hex = format_picked_color(r, g, b);
                            if !typing && overlay_ctx.input(|i| !i.modifiers.any() && i.key_pressed(egui::Key::C)) {
                                overlay_ctx.copy_text(hex.clone());
                            }
                            let mut readout = format!("HEX {hex}\nRGB({r}, {g}, {b})");
                            if color_picker {
                                let (hue, saturation, lightness) = ColorSample::rgb(r, g, b).to_hsl();
                                readout.push_str(&format!("\nHSL({hue:.0}°, {saturation:.0}%, {lightness:.0}%)"));
                            }
                            paint_magnifier(
                                &painter,
                                pos,
                                screen_rect,
                                [toolbar_rect, badge_rect],
                                &grid,
                                &readout,
                            );
                        }
                    }
                });

            if let Some(act) = toolbar_action {
                outcome = act;
            }
        });

        outcome
    }

    fn fit_viewport_to_snapshot(&self, ctx: &egui::Context, image_size: Vec2) {
        let ppp = ctx.pixels_per_point();
        let desired_origin = Pos2::new(self.origin_x as f32 / ppp, self.origin_y as f32 / ppp);
        let desired_size = image_size / ppp;
        let (inner, outer) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.inner_rect, viewport.outer_rect)
        });
        let Some(inner) = inner else {
            return;
        };
        let offset = desired_origin - inner.min;
        let tolerance = 0.5 / ppp;
        if (offset.x.abs() > tolerance || offset.y.abs() > tolerance)
            && let Some(outer) = outer
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(outer.min + offset));
        }
        let size_error = desired_size - inner.size();
        if size_error.x.abs() > tolerance || size_error.y.abs() > tolerance {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired_size));
        }
    }

    fn render_floating_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        accent: Color32,
        toolbar_action: &mut Option<OverlayModeOutcome>,
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
                ui.checkbox(&mut self.snap_to_windows, "Snap windows");
                ui.checkbox(&mut self.show_magnifier, "Loupe")
                    .on_hover_text("Press C on the canvas to copy the hex color");
            });
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
                    *toolbar_action = self.finalize_selection(false);
                }
                if ui.button("Full Editor").clicked() {
                    *toolbar_action = self.finalize_selection(true);
                }
                if ui.button("Cancel [Esc]").clicked() {
                    *toolbar_action = Some(OverlayModeOutcome::Cancelled);
                }
            });
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

fn format_picked_color(r: u8, g: u8, b: u8) -> String {
    ColorSample::rgb(r, g, b).to_hex()
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

fn paint_snapped_window(
    painter: &egui::Painter,
    bounds: Rect,
    screen_rect: Rect,
    window: &WindowInfo,
) {
    painter.rect_stroke(
        bounds,
        0.0,
        Stroke::new(2.0_f32, SNAP_COLOR),
        egui::StrokeKind::Inside,
    );

    let text = match (window.app_name.as_str(), window.title.as_str()) {
        ("", "") => "Window".into(),
        ("", title) => title.into(),
        (app, title) if title.is_empty() || app == title => app.into(),
        (app, title) => format!("{app} / {title}"),
    };
    let padding = Vec2::new(6.0, 3.0);
    let mut label = egui::text::LayoutJob::simple(
        text,
        FontId::proportional(12.0),
        SNAP_COLOR,
        (screen_rect.width() - 2.0 * padding.x).max(0.0),
    );
    label.wrap.max_rows = 1;
    let galley = painter.layout_job(label);
    let size = galley.size() + 2.0 * padding;
    let pos = (bounds.min + Vec2::splat(3.0))
        .min(screen_rect.max - size)
        .max(screen_rect.min);
    painter.rect_filled(
        Rect::from_min_size(pos, size),
        CornerRadius::same(3),
        Color32::from_black_alpha(232),
    );
    painter.galley(pos + padding, galley, SNAP_COLOR);
}

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

fn paint_magnifier(
    painter: &egui::Painter,
    cursor: Pos2,
    screen_rect: Rect,
    obstacles: [Rect; 2],
    grid: &[[u8; 4]],
    footer: &str,
) {
    const SIDE: usize = (MAGNIFIER_HALF_EXTENT * 2 + 1) as usize;
    let pixels_per_point = painter.pixels_per_point();
    let physical_pixel = 1.0_f32 / pixels_per_point;
    let cell_size = 10.0 * physical_pixel;
    let grid_size = SIDE as f32 * cell_size;
    let galley = painter.layout_no_wrap(footer.into(), FontId::monospace(10.0), Color32::WHITE);
    let padding = Vec2::new(5.0, 3.0);
    let size = Vec2::new(
        grid_size.max(galley.size().x + 2.0 * padding.x),
        grid_size + galley.size().y + 2.0 * padding.y,
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
    painter.rect_filled(panel, 0.0, Color32::from_rgb(20, 24, 30));

    let grid_origin =
        (pos + Vec2::new((size.x - grid_size) * 0.5, 0.0)).round_to_pixels(pixels_per_point);
    for (index, &[r, g, b, a]) in grid.iter().enumerate() {
        let offset = Vec2::new((index % SIDE) as f32, (index / SIDE) as f32) * cell_size;
        let cell = Rect::from_min_size(grid_origin + offset, Vec2::splat(cell_size));
        painter.rect_filled(cell, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
    }

    let grid_color = Color32::from_black_alpha(72);
    for index in 1..SIDE {
        let offset = index as f32 * cell_size;
        painter.rect_filled(
            Rect::from_min_size(
                grid_origin + Vec2::new(offset, 0.0),
                Vec2::new(physical_pixel, grid_size),
            ),
            0.0,
            grid_color,
        );
        painter.rect_filled(
            Rect::from_min_size(
                grid_origin + Vec2::new(0.0, offset),
                Vec2::new(grid_size, physical_pixel),
            ),
            0.0,
            grid_color,
        );
    }
    let center = Rect::from_min_size(
        grid_origin + Vec2::splat(MAGNIFIER_HALF_EXTENT as f32 * cell_size),
        Vec2::splat(cell_size),
    );
    painter.rect_stroke(
        center,
        0.0,
        Stroke::new(physical_pixel, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
    painter.rect_stroke(
        center,
        0.0,
        Stroke::new(physical_pixel, Color32::RED),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        panel,
        0.0,
        Stroke::new(physical_pixel, Color32::from_gray(135)),
        egui::StrokeKind::Inside,
    );
    let footer_pos = Pos2::new(
        panel.center().x - galley.size().x * 0.5,
        pos.y + grid_size + padding.y,
    );
    painter.galley(footer_pos, galley, Color32::WHITE);
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
}
