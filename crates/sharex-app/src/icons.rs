use crate::theme::SharexPalette;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Vec2};

/// Vector UI icons painted crisply at any DPI without external bitmap assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Region,
    Fullscreen,
    Window,
    LastRegion,
    Captures,
    Editor,
    Effects,
    Tools,
    Recording,
    Upload,
    Workflow,
    Hotkeys,
    Settings,
    Folder,
    Refresh,
    Search,
    Pin,
    Copy,
    Link,
    Trash,
    Check,
    Close,
    Undo,
    Redo,
    SelectCursor,
    RectOutline,
    RectFilled,
    Ellipse,
    Arrow,
    Line,
    Pen,
    Highlighter,
    TextT,
    StepNumber,
    Pixelate,
    Blur,
    SnapMagnet,
    Loupe,
    EyeDropper,
    OcrText,
}

/// Renders a sleek square icon-only toolbar button with hover highlight and tooltip.
pub fn toolbar_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    selected: bool,
    accent: Color32,
) -> Response {
    let size = Vec2::splat(28.0_f32);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let bg = if selected {
        accent
    } else if response.hovered() {
        Color32::from_rgba_unmultiplied(255, 255, 255, 24)
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    }
    let fg = if selected {
        Color32::WHITE
    } else if response.hovered() {
        Color32::from_rgb(245, 248, 252)
    } else {
        Color32::from_rgb(184, 194, 210)
    };
    paint_icon(ui.painter(), rect.shrink(6.0_f32), icon, fg);
    response.on_hover_text(tooltip)
}

/// Renders a compact icon button styled with the active application palette.
pub fn palette_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    danger: bool,
    p: &SharexPalette,
) -> Response {
    let size = Vec2::splat(26.0_f32);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let bg = if hovered {
        if danger {
            Color32::from_rgba_unmultiplied(239, 68, 68, 36)
        } else {
            p.hover_bg
        }
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    }
    let fg = if hovered && danger {
        p.danger
    } else if hovered {
        p.text_primary
    } else {
        p.text_muted
    };
    paint_icon(ui.painter(), rect.shrink(5.5_f32), icon, fg);
    response.on_hover_text(tooltip)
}

/// Paints a crisp geometric vector icon inside `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(1.5_f32, color);
    let thin = Stroke::new(1.25_f32, color);
    let c = rect.center();
    let w = rect.width();
    let h = rect.height();
    let p = |nx: f32, ny: f32| Pos2::new(rect.min.x + w * nx, rect.min.y + h * ny);

    match icon {
        Icon::Region => {
            let corner = 0.28_f32;
            painter.line_segment([p(0.08, 0.08 + corner), p(0.08, 0.08)], s);
            painter.line_segment([p(0.08, 0.08), p(0.08 + corner, 0.08)], s);
            painter.line_segment([p(0.92 - corner, 0.08), p(0.92, 0.08)], s);
            painter.line_segment([p(0.92, 0.08), p(0.92, 0.08 + corner)], s);
            painter.line_segment([p(0.08, 0.92 - corner), p(0.08, 0.92)], s);
            painter.line_segment([p(0.08, 0.92), p(0.08 + corner, 0.92)], s);
            painter.line_segment([p(0.92 - corner, 0.92), p(0.92, 0.92)], s);
            painter.line_segment([p(0.92, 0.92), p(0.92, 0.92 - corner)], s);
            painter.line_segment([p(0.5, 0.30), p(0.5, 0.70)], thin);
            painter.line_segment([p(0.30, 0.5), p(0.70, 0.5)], thin);
        }
        Icon::Fullscreen => {
            let box_r = Rect::from_min_max(p(0.08, 0.14), p(0.92, 0.74));
            painter.rect_stroke(box_r, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            painter.line_segment([p(0.38, 0.88), p(0.62, 0.88)], s);
            painter.line_segment([p(0.50, 0.74), p(0.50, 0.88)], s);
        }
        Icon::Window => {
            let box_r = Rect::from_min_max(p(0.10, 0.14), p(0.90, 0.86));
            painter.rect_stroke(box_r, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            painter.line_segment([p(0.10, 0.36), p(0.90, 0.36)], thin);
            painter.circle_filled(p(0.22, 0.25), 1.2_f32, color);
            painter.circle_filled(p(0.34, 0.25), 1.2_f32, color);
        }
        Icon::LastRegion => {
            let box_r = Rect::from_min_max(p(0.12, 0.18), p(0.88, 0.82));
            painter.rect_stroke(box_r, CornerRadius::same(2), thin, egui::StrokeKind::Inside);
            painter.line_segment([p(0.34, 0.50), p(0.66, 0.50)], s);
            painter.line_segment([p(0.54, 0.38), p(0.66, 0.50)], s);
            painter.line_segment([p(0.54, 0.62), p(0.66, 0.50)], s);
        }
        Icon::Captures => {
            let box_r = Rect::from_min_max(p(0.08, 0.14), p(0.92, 0.86));
            painter.rect_stroke(box_r, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            painter.circle_filled(p(0.32, 0.36), 1.6_f32, color);
            painter.line_segment([p(0.16, 0.76), p(0.44, 0.50)], thin);
            painter.line_segment([p(0.44, 0.50), p(0.62, 0.66)], thin);
            painter.line_segment([p(0.56, 0.60), p(0.74, 0.44)], thin);
            painter.line_segment([p(0.74, 0.44), p(0.88, 0.76)], thin);
        }
        Icon::Editor | Icon::Pen => {
            painter.line_segment([p(0.18, 0.82), p(0.74, 0.26)], s);
            painter.line_segment([p(0.64, 0.16), p(0.84, 0.36)], s);
            painter.line_segment([p(0.14, 0.86), p(0.32, 0.82)], thin);
            painter.line_segment([p(0.14, 0.86), p(0.18, 0.68)], thin);
        }
        Icon::Effects => {
            painter.line_segment([p(0.50, 0.10), p(0.50, 0.90)], thin);
            painter.line_segment([p(0.10, 0.50), p(0.90, 0.50)], thin);
            painter.line_segment([p(0.24, 0.24), p(0.76, 0.76)], thin);
            painter.line_segment([p(0.76, 0.24), p(0.24, 0.76)], thin);
            painter.circle_filled(c, 2.2_f32, color);
        }
        Icon::Tools => {
            painter.line_segment([p(0.20, 0.80), p(0.66, 0.34)], s);
            painter.circle_stroke(p(0.72, 0.28), w * 0.18_f32, s);
            painter.circle_filled(p(0.20, 0.80), 1.6_f32, color);
        }
        Icon::Recording => {
            painter.circle_stroke(c, w * 0.42_f32, s);
            painter.circle_filled(c, w * 0.20_f32, color);
        }
        Icon::Upload => {
            painter.line_segment([p(0.50, 0.72), p(0.50, 0.18)], s);
            painter.line_segment([p(0.30, 0.38), p(0.50, 0.18)], s);
            painter.line_segment([p(0.70, 0.38), p(0.50, 0.18)], s);
            painter.line_segment([p(0.16, 0.72), p(0.16, 0.86)], s);
            painter.line_segment([p(0.16, 0.86), p(0.84, 0.86)], s);
            painter.line_segment([p(0.84, 0.86), p(0.84, 0.72)], s);
        }
        Icon::Workflow => {
            painter.circle_stroke(p(0.24, 0.26), w * 0.14_f32, s);
            painter.circle_stroke(p(0.76, 0.26), w * 0.14_f32, s);
            painter.circle_stroke(p(0.50, 0.76), w * 0.14_f32, s);
            painter.line_segment([p(0.38, 0.26), p(0.62, 0.26)], thin);
            painter.line_segment([p(0.32, 0.38), p(0.44, 0.64)], thin);
            painter.line_segment([p(0.68, 0.38), p(0.56, 0.64)], thin);
        }
        Icon::Hotkeys => {
            let box_r = Rect::from_min_max(p(0.08, 0.22), p(0.92, 0.78));
            painter.rect_stroke(box_r, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            for nx in [0.24_f32, 0.42, 0.60, 0.76] {
                painter.circle_filled(p(nx, 0.42), 1.1_f32, color);
            }
            painter.line_segment([p(0.28, 0.62), p(0.72, 0.62)], thin);
        }
        Icon::Settings => {
            painter.circle_stroke(c, w * 0.20_f32, s);
            painter.circle_stroke(c, w * 0.38_f32, thin);
            for i in 0..4 {
                let angle = (i as f32) * std::f32::consts::FRAC_PI_2;
                let (sin, cos) = angle.sin_cos();
                let a = Pos2::new(c.x + cos * w * 0.34, c.y + sin * h * 0.34);
                let b = Pos2::new(c.x + cos * w * 0.48, c.y + sin * h * 0.48);
                painter.line_segment([a, b], s);
            }
        }
        Icon::Folder => {
            painter.line_segment([p(0.10, 0.24), p(0.40, 0.24)], s);
            painter.line_segment([p(0.40, 0.24), p(0.50, 0.34)], s);
            painter.line_segment([p(0.50, 0.34), p(0.90, 0.34)], s);
            painter.line_segment([p(0.90, 0.34), p(0.90, 0.82)], s);
            painter.line_segment([p(0.90, 0.82), p(0.10, 0.82)], s);
            painter.line_segment([p(0.10, 0.82), p(0.10, 0.24)], s);
        }
        Icon::Refresh => {
            painter.circle_stroke(c, w * 0.34_f32, s);
            painter.line_segment([p(0.74, 0.16), p(0.84, 0.38)], s);
            painter.line_segment([p(0.60, 0.34), p(0.84, 0.38)], s);
        }
        Icon::Search | Icon::Loupe => {
            painter.circle_stroke(p(0.44, 0.44), w * 0.28_f32, s);
            painter.line_segment([p(0.64, 0.64), p(0.88, 0.88)], s);
        }
        Icon::Pin => {
            painter.line_segment([p(0.32, 0.20), p(0.68, 0.20)], s);
            painter.line_segment([p(0.38, 0.20), p(0.32, 0.56)], thin);
            painter.line_segment([p(0.62, 0.20), p(0.68, 0.56)], thin);
            painter.line_segment([p(0.24, 0.56), p(0.76, 0.56)], s);
            painter.line_segment([p(0.50, 0.56), p(0.50, 0.88)], s);
        }
        Icon::Copy => {
            let back = Rect::from_min_max(p(0.14, 0.14), p(0.68, 0.68));
            let front = Rect::from_min_max(p(0.32, 0.32), p(0.86, 0.86));
            painter.rect_stroke(back, CornerRadius::same(2), thin, egui::StrokeKind::Inside);
            painter.rect_stroke(front, CornerRadius::same(2), s, egui::StrokeKind::Inside);
        }
        Icon::Link => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.10, 0.34), p(0.56, 0.66)),
                CornerRadius::same(4),
                thin,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_max(p(0.44, 0.34), p(0.90, 0.66)),
                CornerRadius::same(4),
                thin,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Trash => {
            painter.line_segment([p(0.18, 0.26), p(0.82, 0.26)], s);
            painter.line_segment([p(0.38, 0.16), p(0.62, 0.16)], s);
            let body = Rect::from_min_max(p(0.24, 0.26), p(0.76, 0.86));
            painter.rect_stroke(body, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            painter.line_segment([p(0.42, 0.42), p(0.42, 0.72)], thin);
            painter.line_segment([p(0.58, 0.42), p(0.58, 0.72)], thin);
        }
        Icon::Check => {
            painter.line_segment([p(0.18, 0.54), p(0.42, 0.78)], s);
            painter.line_segment([p(0.42, 0.78), p(0.84, 0.26)], s);
        }
        Icon::Close => {
            painter.line_segment([p(0.22, 0.22), p(0.78, 0.78)], s);
            painter.line_segment([p(0.78, 0.22), p(0.22, 0.78)], s);
        }
        Icon::Undo => {
            painter.line_segment([p(0.20, 0.42), p(0.70, 0.42)], s);
            painter.line_segment([p(0.20, 0.42), p(0.38, 0.24)], s);
            painter.line_segment([p(0.20, 0.42), p(0.38, 0.60)], s);
            painter.line_segment([p(0.70, 0.42), p(0.80, 0.56)], thin);
            painter.line_segment([p(0.80, 0.56), p(0.70, 0.74)], thin);
            painter.line_segment([p(0.70, 0.74), p(0.42, 0.74)], thin);
        }
        Icon::Redo => {
            painter.line_segment([p(0.80, 0.42), p(0.30, 0.42)], s);
            painter.line_segment([p(0.80, 0.42), p(0.62, 0.24)], s);
            painter.line_segment([p(0.80, 0.42), p(0.62, 0.60)], s);
            painter.line_segment([p(0.30, 0.42), p(0.20, 0.56)], thin);
            painter.line_segment([p(0.20, 0.56), p(0.30, 0.74)], thin);
            painter.line_segment([p(0.30, 0.74), p(0.58, 0.74)], thin);
        }
        Icon::SelectCursor => {
            painter.line_segment([p(0.22, 0.14), p(0.22, 0.80)], s);
            painter.line_segment([p(0.22, 0.14), p(0.76, 0.54)], s);
            painter.line_segment([p(0.76, 0.54), p(0.46, 0.58)], s);
            painter.line_segment([p(0.46, 0.58), p(0.22, 0.80)], s);
            painter.line_segment([p(0.48, 0.60), p(0.68, 0.88)], s);
        }
        Icon::RectOutline => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.14, 0.20), p(0.86, 0.80)),
                CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
        }
        Icon::RectFilled => {
            painter.rect_filled(
                Rect::from_min_max(p(0.16, 0.22), p(0.84, 0.78)),
                CornerRadius::same(2),
                color,
            );
        }
        Icon::Ellipse => {
            painter.circle_stroke(c, w * 0.36_f32, s);
        }
        Icon::Arrow => {
            painter.line_segment([p(0.18, 0.82), p(0.82, 0.18)], s);
            painter.line_segment([p(0.46, 0.18), p(0.82, 0.18)], s);
            painter.line_segment([p(0.82, 0.18), p(0.82, 0.54)], s);
        }
        Icon::Line => {
            painter.line_segment([p(0.16, 0.84), p(0.84, 0.16)], s);
        }
        Icon::Highlighter => {
            let bar = Stroke::new(3.2_f32, color.gamma_multiply(0.75));
            painter.line_segment([p(0.16, 0.68), p(0.72, 0.24)], bar);
            painter.line_segment([p(0.14, 0.84), p(0.86, 0.84)], thin);
        }
        Icon::TextT => {
            painter.line_segment([p(0.18, 0.22), p(0.82, 0.22)], s);
            painter.line_segment([p(0.50, 0.22), p(0.50, 0.84)], s);
            painter.line_segment([p(0.36, 0.84), p(0.64, 0.84)], thin);
        }
        Icon::StepNumber => {
            painter.circle_stroke(c, w * 0.40_f32, s);
            painter.line_segment([p(0.50, 0.30), p(0.50, 0.70)], s);
            painter.line_segment([p(0.40, 0.38), p(0.50, 0.30)], thin);
        }
        Icon::Pixelate => {
            let cell = w * 0.26_f32;
            for (ix, iy) in [(0, 0), (2, 0), (1, 1), (0, 2), (2, 2)] {
                let x = rect.min.x + w * 0.11 + (ix as f32) * cell;
                let y = rect.min.y + h * 0.11 + (iy as f32) * cell;
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::splat(cell - 1.0_f32)),
                    CornerRadius::ZERO,
                    color,
                );
            }
        }
        Icon::Blur => {
            painter.circle_stroke(c, w * 0.38_f32, thin);
            painter.circle_filled(c, w * 0.22_f32, color.gamma_multiply(0.55));
            painter.circle_filled(c, w * 0.10_f32, color);
        }
        Icon::SnapMagnet => {
            painter.line_segment([p(0.24, 0.20), p(0.24, 0.56)], s);
            painter.line_segment([p(0.76, 0.20), p(0.76, 0.56)], s);
            painter.line_segment([p(0.24, 0.56), p(0.38, 0.78)], s);
            painter.line_segment([p(0.38, 0.78), p(0.62, 0.78)], s);
            painter.line_segment([p(0.62, 0.78), p(0.76, 0.56)], s);
            painter.line_segment([p(0.18, 0.32), p(0.30, 0.32)], thin);
            painter.line_segment([p(0.70, 0.32), p(0.82, 0.32)], thin);
        }
        Icon::EyeDropper => {
            painter.line_segment([p(0.20, 0.80), p(0.66, 0.34)], s);
            painter.line_segment([p(0.54, 0.22), p(0.78, 0.46)], s);
            painter.circle_filled(p(0.74, 0.26), 2.2_f32, color);
            painter.circle_filled(p(0.18, 0.82), 1.4_f32, color);
        }
        Icon::OcrText => {
            painter.line_segment([p(0.14, 0.28), p(0.14, 0.14)], s);
            painter.line_segment([p(0.14, 0.14), p(0.28, 0.14)], s);
            painter.line_segment([p(0.72, 0.14), p(0.86, 0.14)], s);
            painter.line_segment([p(0.86, 0.14), p(0.86, 0.28)], s);
            painter.line_segment([p(0.14, 0.72), p(0.14, 0.86)], s);
            painter.line_segment([p(0.14, 0.86), p(0.28, 0.86)], s);
            painter.line_segment([p(0.72, 0.86), p(0.86, 0.86)], s);
            painter.line_segment([p(0.86, 0.86), p(0.86, 0.72)], s);
            painter.line_segment([p(0.30, 0.36), p(0.70, 0.36)], thin);
            painter.line_segment([p(0.30, 0.52), p(0.64, 0.52)], thin);
            painter.line_segment([p(0.30, 0.68), p(0.54, 0.68)], thin);
        }
    }
}
