//! Look and feel: colours, rounding, spacing and text sizes for both themes,
//! plus small animation helpers.

use egui::{Color32, CornerRadius, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2};

pub const ACCENT_DARK: Color32 = Color32::from_rgb(122, 162, 247);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(52, 104, 214);

pub fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode { ACCENT_DARK } else { ACCENT_LIGHT }
}

/// Applies the mindwiki style to both the dark and the light theme.
pub fn apply(ctx: &egui::Context, dark: Option<bool>) {
    ctx.set_theme(match dark {
        Some(true) => egui::ThemePreference::Dark,
        Some(false) => egui::ThemePreference::Light,
        None => egui::ThemePreference::System,
    });

    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(14.5)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.window_margin = Margin::same(14);
        s.spacing.interact_size.y = 26.0;
        s.animation_time = 0.14;

        let r = CornerRadius::same(7);
        let v = &mut s.visuals;
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = r;
        }
        v.widgets.hovered.expansion = 1.0;
        v.widgets.active.expansion = 0.0;
    });

    ctx.style_mut_of(Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(24, 26, 32);
        v.window_fill = Color32::from_rgb(32, 35, 43);
        v.extreme_bg_color = Color32::from_rgb(17, 18, 23);
        v.faint_bg_color = Color32::from_rgb(30, 33, 40);
        v.code_bg_color = Color32::from_rgb(38, 42, 52);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(52, 57, 70));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 48, 59));
        v.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(205, 210, 222);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(40, 44, 54);
        v.widgets.inactive.bg_fill = Color32::from_rgb(40, 44, 54);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 58, 72);
        v.widgets.hovered.bg_fill = Color32::from_rgb(52, 58, 72);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DARK.gamma_multiply(0.6));
        v.selection.bg_fill = ACCENT_DARK.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(225, 232, 250));
        v.hyperlink_color = ACCENT_DARK;
        v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(120) };
        v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(100) };
    });

    ctx.style_mut_of(Theme::Light, |s| {
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(248, 247, 244);
        v.window_fill = Color32::from_rgb(255, 255, 255);
        v.extreme_bg_color = Color32::from_rgb(255, 255, 255);
        v.faint_bg_color = Color32::from_rgb(240, 239, 235);
        v.code_bg_color = Color32::from_rgb(234, 233, 228);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(214, 212, 205));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(222, 220, 213));
        v.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(48, 50, 58);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(234, 233, 228);
        v.widgets.inactive.bg_fill = Color32::from_rgb(234, 233, 228);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(222, 226, 238);
        v.widgets.hovered.bg_fill = Color32::from_rgb(222, 226, 238);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_LIGHT.gamma_multiply(0.6));
        v.selection.bg_fill = ACCENT_LIGHT.gamma_multiply(0.22);
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(20, 40, 90));
        v.hyperlink_color = ACCENT_LIGHT;
        v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(40) };
        v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(30) };
    });
}

/// Smooth ease-out curve for 0..=1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Progress (0..=1, eased) of an animation that started at `start` and lasts
/// `secs`. Keeps the UI repainting until it is finished.
pub fn progress(ctx: &egui::Context, start: f64, secs: f32) -> f32 {
    let t = ((ctx.input(|i| i.time) - start) as f32 / secs).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}
