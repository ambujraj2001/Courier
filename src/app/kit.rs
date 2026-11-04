//! Courier's UI kit: custom-painted controls built on the design tokens in
//! [`theme`](super::theme).

use super::theme::{self, Palette};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, vec2,
};
pub use egui_phosphor::regular as icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

fn galley(ui: &Ui, text: &str, font: FontId, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(text.to_string(), font, color)
}

/// A button with an optional leading icon. Height 30 (or 26 when `small`).
pub fn button_ext(ui: &mut Ui, kind: Kind, icon: Option<&str>, label: &str, small: bool, enabled: bool) -> Response {
    let p = theme::palette(ui);
    let height = if small { 26.0 } else { 30.0 };
    let font = FontId::proportional(if small { 12.5 } else { 13.0 });
    let icon_font = FontId::proportional(if small { 14.0 } else { 15.0 });
    let text_color = match kind {
        Kind::Primary => p.on_accent,
        Kind::Danger => p.danger,
        _ => p.text,
    };
    let text_color = if enabled { text_color } else { p.text_faint };
    let label_g = (!label.is_empty()).then(|| galley(ui, label, font, text_color));
    let icon_g =
        icon.map(|i| galley(ui, i, icon_font, if kind == Kind::Ghost && enabled { p.text_muted } else { text_color }));
    let pad = if small { 9.0 } else { 12.0 };
    let gap = if label_g.is_some() && icon_g.is_some() { 6.0 } else { 0.0 };
    let content_w = label_g.as_ref().map_or(0.0, |g| g.size().x) + icon_g.as_ref().map_or(0.0, |g| g.size().x) + gap;
    let width = if label_g.is_none() { height } else { content_w + pad * 2.0 };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), sense);
    let a11y = if label.is_empty() { icon.unwrap_or_default().to_string() } else { label.to_string() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &a11y));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let pressed = enabled && resp.is_pointer_button_down_on();
        let radius = CornerRadius::same(7);
        let (fill, stroke) = match kind {
            Kind::Primary => {
                let f = if !enabled {
                    p.surface_active
                } else if pressed || hovered {
                    p.accent_hover
                } else {
                    p.accent
                };
                (f, Stroke::NONE)
            }
            Kind::Secondary => (
                if pressed {
                    p.surface_active
                } else if hovered {
                    p.surface_hover
                } else {
                    p.surface
                },
                Stroke::new(1.0_f32, if hovered { p.border_strong } else { p.border }),
            ),
            Kind::Ghost => (
                if pressed {
                    p.surface_active
                } else if hovered {
                    p.surface_hover
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::NONE,
            ),
            Kind::Danger => (
                if hovered { p.danger.linear_multiply(0.15) } else { Color32::TRANSPARENT },
                Stroke::new(1.0_f32, p.danger.linear_multiply(0.6)),
            ),
        };
        ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
        let mut x = rect.center().x - content_w / 2.0;
        if let Some(g) = icon_g {
            let mut c = if kind == Kind::Ghost && hovered {
                p.text
            } else if kind == Kind::Ghost && enabled {
                p.text_muted
            } else {
                text_color
            };
            if !enabled {
                c = p.text_faint;
            }
            let w = g.size().x;
            ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, c);
            x += w + gap;
        }
        if let Some(g) = label_g {
            ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, text_color);
        }
    }
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub fn button(ui: &mut Ui, kind: Kind, icon: Option<&str>, label: &str) -> Response {
    button_ext(ui, kind, icon, label, false, true)
}

pub fn small_button(ui: &mut Ui, kind: Kind, icon: Option<&str>, label: &str) -> Response {
    button_ext(ui, kind, icon, label, true, true)
}

/// Square ghost button holding a single icon.
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str) -> Response {
    let r = button_ext(ui, Kind::Ghost, Some(glyph), "", true, true);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip));
    r.on_hover_text(tooltip)
}

/// Icon button that shows a selected state (accent tint).
pub fn icon_toggle(ui: &mut Ui, glyph: &str, selected: bool, size: f32, tooltip: &str) -> Response {
    let p = theme::palette(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let fill = if selected {
            p.accent_soft
        } else if hovered {
            p.surface_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 8.0, fill);
        let color = if selected {
            p.accent
        } else if hovered {
            p.text
        } else {
            p.text_muted
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(size * 0.52), color);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Underlined section tabs. Each item is (value, label, optional count).
/// A count is shown as "(n)" after the label; "•" shows a small dot meaning
/// "this tab has content". Returns true when the selection changed.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str, Option<String>)]) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    let border_idx = ui.painter().add(egui::Shape::Noop);
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (value, label, badge) in items {
            let selected = *current == *value;
            let font = FontId::proportional(12.5);
            let text_g = galley(ui, label, font.clone(), p.text);
            let dot = badge.as_deref() == Some("•");
            let count_g = badge.as_ref().filter(|_| !dot).map(|b| {
                galley(
                    ui,
                    &format!("({b})"),
                    FontId::proportional(12.0),
                    if selected { p.accent } else { p.text_faint },
                )
            });
            let extra_w = if dot { 9.0 } else { count_g.as_ref().map_or(0.0, |g| g.size().x + 4.0) };
            let width = text_g.size().x + 22.0 + extra_w;
            let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
            let a11y = match badge {
                Some(b) => format!("{label} ({b})"),
                None => label.to_string(),
            };
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &a11y));
            let hovered = resp.hovered();
            let color = if selected || hovered { p.text } else { p.text_muted };
            let x0 = rect.left() + 11.0;
            let ty = rect.center().y - text_g.size().y / 2.0;
            ui.painter().galley(egui::pos2(x0, ty), text_g.clone(), color);
            if dot {
                ui.painter().circle_filled(
                    egui::pos2(x0 + text_g.size().x + 6.0, rect.center().y - 3.0),
                    2.6,
                    p.success,
                );
            }
            if let Some(g) = count_g {
                let gx = x0 + text_g.size().x + 4.0;
                ui.painter().galley(egui::pos2(gx, rect.center().y - g.size().y / 2.0), g, p.text_faint);
            }
            if selected {
                let line = Rect::from_min_max(
                    egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
                    egui::pos2(rect.right() - 4.0, rect.bottom()),
                );
                ui.painter().rect_filled(line, 0.0, p.accent);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected {
                *current = *value;
                changed = true;
            }
        }
    });
    let r = row.response.rect;
    let full = Rect::from_min_max(
        egui::pos2(ui.min_rect().left(), r.bottom() - 1.0),
        egui::pos2(ui.max_rect().right(), r.bottom()),
    );
    ui.painter().set(border_idx, egui::Shape::rect_filled(full, 0.0, p.border));
    changed
}

/// Plain text toggles where the selected one sits on a light pill
/// (e.g. Pretty / Raw / Preview).
pub fn pill_tabs<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (value, label) in items {
            let selected = *current == *value;
            let g = galley(ui, label, FontId::proportional(12.0), p.text);
            let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 24.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, *label));
            if selected {
                ui.painter().rect_filled(rect, 4.0, p.surface_active);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 4.0, p.surface_hover);
            }
            let color = if selected || resp.hovered() { p.text } else { p.text_muted };
            ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected {
                *current = *value;
                changed = true;
            }
        }
    });
    changed
}

/// A radio button with its label, laid out inline.
pub fn radio(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let p = theme::palette(ui);
    let g = galley(ui, label, FontId::proportional(12.5), p.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(18.0 + g.size().x + 10.0, 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label));
    let c = egui::pos2(rect.left() + 7.0, rect.center().y);
    let ring = if selected {
        p.accent
    } else if resp.hovered() {
        p.text_muted
    } else {
        p.border_strong
    };
    ui.painter().circle(c, 6.5, p.input, Stroke::new(1.2_f32, ring));
    if selected {
        ui.painter().circle_filled(c, 3.5, p.accent);
    }
    let color = if selected { p.text } else { p.text_muted };
    ui.painter().galley(egui::pos2(rect.left() + 18.0, rect.center().y - g.size().y / 2.0), g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Pill-shaped segmented control.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = items.iter().map(|(_, l)| galley(ui, l, font.clone(), p.text).size().x + 20.0).collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (outer, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect(outer, 7.0, p.input, Stroke::new(1.0_f32, p.border), StrokeKind::Inside);
    let mut x = outer.left() + 2.0;
    for ((value, label), w) in items.iter().zip(widths) {
        let rect = Rect::from_min_size(egui::pos2(x, outer.top() + 2.0), vec2(w, 24.0));
        let resp = ui.interact(rect, ui.id().with(("seg", label)), Sense::click());
        let selected = *current == *value;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, *label));
        if selected {
            ui.painter().rect(rect, 5.0, p.surface_active, Stroke::new(1.0_f32, p.border_strong), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 5.0, p.surface_hover);
        }
        let color = if selected || resp.hovered() { p.text } else { p.text_muted };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !selected {
            *current = *value;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Small rounded label with a tinted background.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let g = galley(ui, text, FontId::proportional(11.5), color);
    let size = vec2(g.size().x + 14.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().rect_filled(rect, 10.0, color.gamma_multiply(0.14));
    ui.painter().galley(egui::pos2(rect.left() + 7.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

/// Coloured monospace method tag of a fixed width.
pub fn method_tag(ui: &mut Ui, method: &str, width: f32) {
    let color = theme::method_color(method, ui.visuals().dark_mode);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        theme::method_short(method),
        FontId::monospace(10.5),
        color,
    );
}

/// Upper-case muted label for groups of controls.
pub fn section_label(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui);
    ui.label(RichText::new(text.to_uppercase()).text_style(theme::caption_style()).color(p.text_faint).strong());
}

/// Centered icon, title and subtitle for empty views.
pub fn empty_state(ui: &mut Ui, glyph: &str, title: &str, subtitle: &str) {
    let p = theme::palette(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 70.0).max(16.0));
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        ui.painter().rect(rect, 16.0, p.surface, Stroke::new(1.0_f32, p.border), StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(26.0), p.text_muted);
        ui.add_space(10.0);
        ui.label(RichText::new(title).size(15.0).color(p.text).strong());
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).color(p.text_muted));
    });
}

/// Frame for raised cards.
pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(14))
}

/// Text field with a leading magnifier icon.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let p = theme::palette(ui);
    let width = if width.is_finite() { width } else { ui.available_width() };
    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    let mut inner = None;
    ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
        let focused_id = ui.id().with("search_edit");
        let focused = ui.memory(|m| m.has_focus(focused_id));
        ui.painter().rect(
            rect,
            8.0,
            p.input,
            Stroke::new(1.0_f32, if focused { p.accent } else { p.border }),
            StrokeKind::Inside,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 11.0, rect.center().y),
            Align2::LEFT_CENTER,
            icon::MAGNIFYING_GLASS,
            FontId::proportional(14.0),
            p.text_faint,
        );
        let edit_rect = Rect::from_min_max(
            egui::pos2(rect.left() + 28.0, rect.top()),
            egui::pos2(rect.right() - 6.0, rect.bottom()),
        );
        let r = ui.put(
            edit_rect,
            egui::TextEdit::singleline(text)
                .id(focused_id)
                .frame(false)
                .hint_text(RichText::new(hint).color(p.text_faint))
                .margin(vec2(0.0, 6.0)),
        );
        inner = Some(r);
    });
    inner.expect("search field response")
}

/// A full-width interactive row (tree item, list entry). Paints hover and
/// selection backgrounds, then lays out `contents` left to right inside.
pub fn row<R>(
    ui: &mut Ui,
    id: egui::Id,
    selected: bool,
    height: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, R) {
    let p = theme::palette(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            p.accent_soft
        } else if resp.hovered() {
            p.surface_hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let inner = ui
        .allocate_new_ui(
            UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                contents(ui)
            },
        )
        .inner;
    (resp, inner)
}

/// Compact rounded checkbox.
pub fn checkbox(ui: &mut Ui, value: &mut bool) -> Response {
    let p = theme::palette(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let b = Rect::from_center_size(rect.center(), Vec2::splat(15.0));
    if resp.clicked() {
        *value = !*value;
        resp.mark_changed();
    }
    if *value {
        ui.painter().rect_filled(b, 4.0, p.accent);
        ui.painter().text(b.center(), Align2::CENTER_CENTER, icon::CHECK, FontId::proportional(11.0), p.on_accent);
    } else {
        ui.painter().rect(
            b,
            4.0,
            p.input,
            Stroke::new(1.0_f32, if resp.hovered() { p.accent } else { p.border_strong }),
            StrokeKind::Inside,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Checkbox with a label to its right.
pub fn labeled_checkbox(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    ui.horizontal(|ui| {
        let r = checkbox(ui, value);
        let l = ui.add(egui::Label::new(label).sense(Sense::click()));
        if l.clicked() {
            *value = !*value;
        }
        r.union(l)
    })
    .inner
}

/// A thin horizontal rule in the border colour.
pub fn divider(ui: &mut Ui) {
    let p = theme::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border);
}

/// Keyboard shortcut hint ("⌘", "↩"…) drawn as small keys.
pub fn kbd(ui: &mut Ui, keys: &[&str]) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for k in keys {
            let k = super::platform::key_token(k);
            let g = galley(ui, &k, FontId::proportional(11.5), p.text_muted);
            let (rect, _) = ui.allocate_exact_size(vec2((g.size().x + 10.0).max(20.0), 20.0), Sense::hover());
            ui.painter().rect(rect, 5.0, p.surface, Stroke::new(1.0_f32, p.border_strong), StrokeKind::Inside);
            ui.painter().galley(rect.center() - g.size() / 2.0, g, p.text_muted);
        }
    });
}

/// Drop-down styled as a pill button; the menu content is added by `add`.
pub fn dropdown<R>(
    ui: &mut Ui,
    id: &str,
    leading: Option<(&str, Color32)>,
    text: &str,
    text_color: Option<Color32>,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let p = theme::palette(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let popup_id = ui.make_persistent_id(id);
    let open = ui.memory(|m| m.is_popup_open(popup_id));
    if ui.is_rect_visible(rect) {
        let fill = if open || resp.hovered() { p.surface_hover } else { p.surface };
        ui.painter().rect(
            rect,
            7.0,
            fill,
            Stroke::new(1.0_f32, if open { p.border_strong } else { p.border }),
            StrokeKind::Inside,
        );
        let mut x = rect.left() + 10.0;
        if let Some((glyph, c)) = leading {
            ui.painter().text(
                egui::pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                glyph,
                FontId::proportional(14.0),
                c,
            );
            x += 20.0;
        }
        let max_w = rect.right() - 24.0 - x;
        let mut job = egui::text::LayoutJob::simple_singleline(
            text.to_string(),
            FontId::proportional(13.0),
            text_color.unwrap_or(p.text),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let g = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
        ui.painter().text(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            icon::CARET_DOWN,
            FontId::proportional(12.0),
            p.text_muted,
        );
    }
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    egui::popup_below_widget(ui, popup_id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(width.max(180.0));
        add(ui)
    })
}

/// A selectable menu entry used inside dropdowns.
pub fn menu_item(ui: &mut Ui, selected: bool, text: impl Into<egui::WidgetText>) -> Response {
    let r =
        ui.add(egui::Button::new(text).selected(selected).frame(selected).min_size(vec2(ui.available_width(), 26.0)));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// iOS-style on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = theme::palette(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let track = if *on { p.accent } else { p.border_strong };
    ui.painter().rect_filled(rect, 9.0, track);
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A settings row: title and description on the left, a switch on the right.
pub fn setting_switch(ui: &mut Ui, title: &str, description: &str, on: &mut bool) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).color(p.text));
            if !description.is_empty() {
                ui.label(RichText::new(description).color(p.text_muted).size(12.0));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = toggle(ui, on).changed();
        });
    });
    ui.add_space(4.0);
    divider(ui);
    ui.add_space(4.0);
    changed
}

/// A truncating label that leaves `trailing_w` points on the right for
/// trailing controls (badges, counts, menu buttons) added by `trailing`.
pub fn label_with_trailing(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    trailing_w: f32,
    trailing: impl FnOnce(&mut Ui),
) {
    let w = (ui.available_width() - trailing_w).max(24.0);
    let h = ui.available_height().clamp(18.0, 30.0);
    ui.allocate_ui_with_layout(vec2(w, h), Layout::left_to_right(Align::Center), |ui| {
        ui.set_width(w);
        ui.add(egui::Label::new(text).truncate().selectable(false));
    });
    ui.with_layout(Layout::right_to_left(Align::Center), trailing);
}
