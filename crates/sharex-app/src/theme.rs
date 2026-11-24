use eframe::egui::{self, Color32, CornerRadius, Margin, Stroke, Visuals};

/// ShareX dark theme palette constants.
pub struct SharexPalette {
    pub sidebar_bg: Color32,
    pub panel_bg: Color32,
    pub card_bg: Color32,
    pub hover_bg: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub warning: Color32,
}

pub fn palette(dark: bool) -> SharexPalette {
    if dark {
        SharexPalette {
            sidebar_bg: Color32::from_rgb(27, 31, 36),
            panel_bg: Color32::from_rgb(35, 39, 46),
            card_bg: Color32::from_rgb(44, 49, 58),
            hover_bg: Color32::from_rgb(54, 61, 72),
            border: Color32::from_rgb(60, 68, 80),
            accent: Color32::from_rgb(59, 130, 246),
            accent_hover: Color32::from_rgb(79, 148, 255),
            text_primary: Color32::from_rgb(232, 237, 243),
            text_muted: Color32::from_rgb(148, 160, 176),
            success: Color32::from_rgb(52, 199, 89),
            danger: Color32::from_rgb(235, 87, 87),
            warning: Color32::from_rgb(245, 166, 35),
        }
    } else {
        SharexPalette {
            sidebar_bg: Color32::from_rgb(238, 241, 246),
            panel_bg: Color32::from_rgb(247, 249, 252),
            card_bg: Color32::from_rgb(255, 255, 255),
            hover_bg: Color32::from_rgb(229, 235, 244),
            border: Color32::from_rgb(208, 215, 224),
            accent: Color32::from_rgb(37, 99, 235),
            accent_hover: Color32::from_rgb(59, 130, 246),
            text_primary: Color32::from_rgb(24, 30, 38),
            text_muted: Color32::from_rgb(95, 108, 124),
            success: Color32::from_rgb(22, 163, 74),
            danger: Color32::from_rgb(220, 38, 38),
            warning: Color32::from_rgb(217, 119, 6),
        }
    }
}

/// Applies the authentic ShareX visual theme to an `egui::Context`.
pub fn apply_sharex_theme(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.window_fill = p.panel_bg;
    visuals.panel_fill = p.panel_bg;
    visuals.faint_bg_color = p.card_bg;
    visuals.extreme_bg_color = p.sidebar_bg;
    visuals.override_text_color = Some(p.text_primary);
    visuals.selection.bg_fill = p.accent;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);

    let radius = CornerRadius::same(4);
    visuals.widgets.noninteractive.bg_fill = p.card_bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.bg_fill = p.card_bg;
    visuals.widgets.inactive.weak_bg_fill = p.card_bg;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.bg_fill = p.hover_bg;
    visuals.widgets.hovered.weak_bg_fill = p.hover_bg;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent_hover);
    visuals.widgets.active.corner_radius = radius;

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = Margin::same(10);
    ctx.set_style(style);
}
