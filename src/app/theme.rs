//! Courier's design system: colour tokens, typography, spacing and egui
//! visuals for the light and dark themes.

use crate::settings::Theme;
use egui::epaint::Shadow;
use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Semantic colour tokens for one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Window background behind everything (rail, sidebar, title areas).
    pub chrome: Color32,
    /// Main content background.
    pub canvas: Color32,
    /// Raised surfaces: cards, popovers, active tab.
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    /// Text input fields and code blocks.
    pub input: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on accent-filled buttons.
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
}

pub const DARK: Palette = Palette {
    dark: true,
    chrome: rgb(0x0b0d10),
    canvas: rgb(0x111317),
    surface: rgb(0x181b20),
    surface_hover: rgb(0x1f2329),
    surface_active: rgb(0x262b32),
    input: rgb(0x0d0f12),
    border: rgb(0x23272e),
    border_strong: rgb(0x31363f),
    text: rgb(0xe8eaed),
    text_muted: rgb(0x949ba7),
    text_faint: rgb(0x5f6672),
    accent: rgb(0x2dd4bf),
    accent_hover: rgb(0x5eead4),
    on_accent: rgb(0x032421),
    accent_soft: Color32::from_rgba_premultiplied(0x08, 0x2b, 0x28, 0xff),
    success: rgb(0x4ade80),
    warning: rgb(0xfbbf24),
    danger: rgb(0xf87171),
    info: rgb(0x60a5fa),
};

pub const LIGHT: Palette = Palette {
    dark: false,
    chrome: rgb(0xf3f4f6),
    canvas: rgb(0xffffff),
    surface: rgb(0xffffff),
    surface_hover: rgb(0xf1f3f5),
    surface_active: rgb(0xe7eaee),
    input: rgb(0xf8f9fb),
    border: rgb(0xe3e6ea),
    border_strong: rgb(0xd0d5dc),
    text: rgb(0x111827),
    text_muted: rgb(0x5b6472),
    text_faint: rgb(0x9aa2ae),
    accent: rgb(0x0d9488),
    accent_hover: rgb(0x0f766e),
    on_accent: rgb(0xffffff),
    accent_soft: rgb(0xe6f6f4),
    success: rgb(0x16a34a),
    warning: rgb(0xd97706),
    danger: rgb(0xdc2626),
    info: rgb(0x2563eb),
};

pub fn palette_for(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

/// Palette for the theme the UI is currently drawn with.
pub fn palette(ui: &egui::Ui) -> Palette {
    palette_for(ui.visuals().dark_mode)
}

pub fn accent(dark: bool) -> Color32 {
    palette_for(dark).accent
}

pub fn method_color(method: &str, dark: bool) -> Color32 {
    let (light, dark_c) = match method {
        "GET" => (rgb(0x15803d), rgb(0x4ade80)),
        "POST" => (rgb(0xb45309), rgb(0xfbbf24)),
        "PUT" => (rgb(0x1d4ed8), rgb(0x60a5fa)),
        "PATCH" => (rgb(0x7e22ce), rgb(0xc084fc)),
        "DELETE" => (rgb(0xb91c1c), rgb(0xf87171)),
        "HEAD" | "OPTIONS" => (rgb(0x475569), rgb(0x94a3b8)),
        _ => (rgb(0x9a3412), rgb(0xfb923c)),
    };
    if dark { dark_c } else { light }
}

pub fn status_color(code: u16, dark: bool) -> Color32 {
    let p = palette_for(dark);
    match code {
        200..=299 => p.success,
        300..=399 => p.info,
        400..=499 => p.warning,
        _ => p.danger,
    }
}

pub fn ok_color(dark: bool) -> Color32 {
    palette_for(dark).success
}

pub fn err_color(dark: bool) -> Color32 {
    palette_for(dark).danger
}

pub fn warn_color(dark: bool) -> Color32 {
    palette_for(dark).warning
}

/// Short label used in the sidebar for long method names.
pub fn method_short(method: &str) -> String {
    match method {
        "DELETE" => "DEL".into(),
        "OPTIONS" => "OPT".into(),
        m if m.len() > 5 => m[..4].to_string(),
        m => m.to_string(),
    }
}

/// Named text styles beyond egui's defaults.
pub fn title_style() -> TextStyle {
    TextStyle::Name("title".into())
}

pub fn caption_style() -> TextStyle {
    TextStyle::Name("caption".into())
}

fn visuals(p: Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    let r6 = CornerRadius::same(6);
    v.override_text_color = None;
    v.panel_fill = p.canvas;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.surface_hover;
    v.code_bg_color = p.input;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    let shadow_alpha = if p.dark { 140 } else { 40 };
    v.window_shadow = Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
    v.selection.bg_fill = if p.dark {
        Color32::from_rgba_unmultiplied(0x2d, 0xd4, 0xbf, 0x40)
    } else {
        Color32::from_rgba_unmultiplied(0x0d, 0x94, 0x88, 0x30)
    };
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.accent);
    v.striped = false;
    v.indent_has_left_vline = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.corner_radius = r6;

    w.inactive.bg_fill = p.surface_active;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.corner_radius = r6;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.surface_active;
    w.hovered.weak_bg_fill = p.surface_hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.corner_radius = r6;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.surface_active;
    w.active.weak_bg_fill = p.surface_active;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.active.corner_radius = r6;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.surface_hover;
    w.open.weak_bg_fill = p.surface_hover;
    w.open.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.open.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.open.corner_radius = r6;
    v
}

pub fn light() -> Visuals {
    visuals(LIGHT)
}

pub fn dark() -> Visuals {
    visuals(DARK)
}

fn style(theme: egui::Theme) -> egui::Style {
    let p = palette_for(theme == egui::Theme::Dark);
    let mut style = egui::Style { visuals: visuals(p), ..Default::default() };
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (title_style(), FontId::new(20.0, FontFamily::Proportional)),
        (caption_style(), FontId::new(10.5, FontFamily::Proportional)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 5.0);
    s.interact_size = egui::vec2(36.0, 26.0);
    s.indent = 14.0;
    s.window_margin = Margin::same(18);
    s.menu_margin = Margin::same(6);
    s.combo_width = 120.0;
    s.text_edit_width = 240.0;
    s.icon_width = 15.0;
    s.icon_width_inner = 9.0;
    s.icon_spacing = 6.0;
    s.scroll = egui::style::ScrollStyle::thin();
    s.scroll.bar_width = 8.0;
    s.scroll.floating_allocated_width = 0.0;
    s.menu_spacing = 2.0;
    style.interaction.selectable_labels = false;
    style.animation_time = 0.12;
    style
}

pub fn apply(ctx: &egui::Context, theme: Theme) {
    ctx.set_style_of(egui::Theme::Dark, style(egui::Theme::Dark));
    ctx.set_style_of(egui::Theme::Light, style(egui::Theme::Light));
    ctx.options_mut(|o| {
        o.theme_preference = match theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        }
    });
}

/// Window icon: Courier's own mark, a rounded teal square with a white
/// chevron, drawn procedurally so no image assets are needed.
pub fn app_icon() -> egui::IconData {
    let size = 256;
    egui::IconData { rgba: icon_rgba(size), width: size, height: size }
}

/// RGBA pixels of the Courier icon at `size`×`size` (anti-aliased).
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let n = size as usize;
    let mut rgba = vec![0u8; n * n * 4];
    // 4×4 supersampling for smooth edges.
    const SS: usize = 4;
    for y in 0..n {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n as f32;
                    let fy = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n as f32;
                    let px = icon_sample(fx, fy);
                    for c in 0..4 {
                        acc[c] += px[c];
                    }
                }
            }
            let i = (y * n + x) * 4;
            let samples = (SS * SS) as f32;
            let a = acc[3] / samples;
            for c in 0..3 {
                // Un-premultiply the averaged colour.
                rgba[i + c] = if a > 0.0 { (acc[c] / samples / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            }
            rgba[i + 3] = (a * 255.0).round() as u8;
        }
    }
    rgba
}

/// One premultiplied sample of the icon at normalised coordinates.
fn icon_sample(fx: f32, fy: f32) -> [f32; 4] {
    // macOS-style margins: the tile occupies ~80% of the canvas.
    let margin = 0.1;
    let (u, v) = ((fx - margin) / (1.0 - 2.0 * margin), (fy - margin) / (1.0 - 2.0 * margin));
    let r = 0.225;
    let dx = (u - 0.5).abs() - (0.5 - r);
    let dy = (v - 0.5).abs() - (0.5 - r);
    if dx.max(0.0).hypot(dy.max(0.0)) > r || !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return [0.0; 4];
    }
    // Diagonal gradient from bright teal to deep teal.
    let t = ((u + v) / 2.0).clamp(0.0, 1.0);
    let (r0, g0, b0) = (0x3d as f32, 0xd6 as f32, 0xc4 as f32);
    let (r1, g1, b1) = (0x0a as f32, 0x5f as f32, 0x66 as f32);
    let mut c = [(r0 + (r1 - r0) * t) / 255.0, (g0 + (g1 - g0) * t) / 255.0, (b0 + (b1 - b0) * t) / 255.0];
    let on_stroke = |ax: f32, ay: f32, bx: f32, by: f32, w: f32| {
        let (vx, vy) = (bx - ax, by - ay);
        let h = (((u - ax) * vx + (v - ay) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
        (ax + vx * h - u).hypot(ay + vy * h - v) < w
    };
    // A chevron ">" plus a short "speed" line: a message on its way.
    if on_stroke(0.36, 0.28, 0.66, 0.5, 0.065)
        || on_stroke(0.66, 0.5, 0.36, 0.72, 0.065)
        || on_stroke(0.2, 0.5, 0.36, 0.5, 0.045)
    {
        c = [1.0, 1.0, 1.0];
    }
    [c[0], c[1], c[2], 1.0]
}

/// Fonts: the platform's UI and code fonts (SF Pro / SF Mono on macOS,
/// Ubuntu / DejaVu / Noto on Linux, Segoe UI / Consolas on Windows), Phosphor
/// for icons and system symbol fonts as a fallback. egui's bundled fonts are
/// used when none of the files exist.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let mut add = |name: &str, candidates: &[&str], families: &[FontFamily], primary: bool| {
        let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else { return };
        fonts.font_data.insert(name.to_string(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        for family in families {
            let list = fonts.families.entry(family.clone()).or_default();
            if primary {
                list.insert(0, name.to_string());
            } else {
                list.push(name.to_string());
            }
        }
    };
    let (ui_fonts, mono_fonts, symbol_fonts) = super::platform::font_candidates();
    add("system-ui", ui_fonts, &[FontFamily::Proportional], true);
    add("system-mono", mono_fonts, &[FontFamily::Monospace], true);
    add("system-symbols", symbol_fonts, &[FontFamily::Proportional, FontFamily::Monospace], false);
    ctx.set_fonts(fonts);
}
