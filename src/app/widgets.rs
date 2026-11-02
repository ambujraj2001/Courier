//! Reusable editors: key/value tables, code editors, variable-aware text
//! fields and the auth form, styled with Courier's design tokens.

use super::highlight::{self, Lang};
use super::kit::{self, icon};
use super::theme;
use crate::auth::{self, FieldKind};
use crate::model::{Auth, AuthType, FormField, FormFieldKind, KeyValue, parse_bulk, to_bulk};
use crate::vars::VarContext;
use egui::{Align, FontId, Rect, RichText, Stroke, TextEdit, Ui, UiBuilder, vec2};
use std::sync::Arc;

pub fn is_dark(ui: &Ui) -> bool {
    ui.visuals().dark_mode
}

pub fn var_hover_public(resp: &egui::Response, text: &str, vars: &VarContext) {
    let refs = VarContext::references(text);
    if refs.is_empty() {
        return;
    }
    resp.clone().on_hover_ui(|ui| {
        let p = theme::palette(ui);
        egui::Grid::new("var_hover").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
            for (_, _, name) in refs {
                ui.label(RichText::new(format!("{{{{{name}}}}}")).monospace().color(p.text_muted));
                match vars.lookup(&name) {
                    Some((value, scope)) => {
                        let shown = if scope == crate::vars::ScopeKind::Dynamic {
                            "generated on send".to_string()
                        } else {
                            value
                        };
                        ui.label(RichText::new(shown).monospace());
                        kit::badge(ui, scope.label(), p.accent);
                    }
                    None => {
                        ui.label(RichText::new("not defined").color(p.danger));
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    });
}

/// Single-line text field with `{{variable}}` highlighting and a tooltip
/// listing the resolved values. `frame` false draws it borderless (tables).
pub fn var_field_ext(
    ui: &mut Ui,
    text: &mut String,
    vars: &VarContext,
    hint: &str,
    width: f32,
    frame: bool,
    font_size: f32,
) -> egui::Response {
    let p = theme::palette(ui);
    let dark = p.dark;
    let font = FontId::monospace(font_size);
    let mut layouter = |ui: &Ui, s: &str, wrap: f32| -> Arc<egui::Galley> {
        let job = highlight::layout(s, Lang::Plain, font.clone(), dark, Some(vars), "", wrap);
        ui.fonts(|f| f.layout_job(job))
    };
    let edit = TextEdit::singleline(text)
        .hint_text(RichText::new(hint).color(p.text_faint))
        .frame(frame)
        .margin(vec2(8.0, if frame { 6.0 } else { 7.0 }))
        .font(FontId::monospace(font_size))
        .layouter(&mut layouter);
    let resp = if width.is_finite() { ui.add_sized([width, 30.0], edit) } else { ui.add(edit) };
    var_hover_public(&resp, text, vars);
    resp
}

pub fn var_field(ui: &mut Ui, text: &mut String, vars: &VarContext, hint: &str, width: f32) -> egui::Response {
    var_field_ext(ui, text, vars, hint, width, true, 12.5)
}

/// Plain framed text input with comfortable padding.
pub fn text_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let p = theme::palette(ui);
    ui.add_sized(
        [width, 30.0],
        TextEdit::singleline(text).hint_text(RichText::new(hint).color(p.text_faint)).margin(vec2(8.0, 6.0)),
    )
}

fn editor_frame(ui: &mut Ui, focused: bool, add: impl FnOnce(&mut Ui)) {
    let p = theme::palette(ui);
    egui::Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0_f32, if focused { p.accent } else { p.border }))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(1))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
}

fn gutter(ui: &mut Ui, lines: usize, font_size: f32) {
    let p = theme::palette(ui);
    let digits = lines.max(1).to_string().len().max(2);
    let numbers: String = (1..=lines.max(1)).map(|n| format!("{n:>digits$}")).collect::<Vec<_>>().join("\n");
    egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 6, top: 8, bottom: 8 }).show(ui, |ui| {
        ui.label(RichText::new(numbers).font(FontId::monospace(font_size)).color(p.text_faint));
    });
}

/// Multi-line code editor with line numbers, syntax and variable highlighting.
pub fn code_editor(
    ui: &mut Ui,
    id: &str,
    text: &mut String,
    lang: Lang,
    vars: Option<&VarContext>,
    rows: usize,
    font_size: f32,
) -> egui::Response {
    let dark = is_dark(ui);
    let font = FontId::monospace(font_size);
    let edit_id = egui::Id::new(("code_editor", id));
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let mut out = None;
    let height = (ui.available_height() - 4.0).max(rows as f32 * font_size * 1.4);
    editor_frame(ui, focused, |ui| {
        egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).max_height(height).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                gutter(ui, text.split('\n').count(), font_size);
                let mut layouter = |ui: &Ui, s: &str, _wrap: f32| -> Arc<egui::Galley> {
                    let job = highlight::layout(s, lang, font.clone(), dark, vars, "", f32::INFINITY);
                    ui.fonts(|f| f.layout_job(job))
                };
                out = Some(
                    ui.add(
                        TextEdit::multiline(text)
                            .id(edit_id)
                            .frame(false)
                            .code_editor()
                            .margin(vec2(8.0, 8.0))
                            .desired_rows(rows)
                            .desired_width(f32::INFINITY)
                            .lock_focus(true)
                            .layouter(&mut layouter),
                    ),
                );
            });
        });
    });
    out.expect("editor response")
}

/// Read-only highlighted text view (selectable, with search highlighting).
pub fn code_view(ui: &mut Ui, id: &str, text: &str, lang: Lang, search: &str, font_size: f32, wrap: bool) {
    let dark = is_dark(ui);
    let font = FontId::monospace(font_size);
    editor_frame(ui, false, |ui| {
        egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if !wrap {
                    gutter(ui, text.split('\n').count(), font_size);
                }
                let mut layouter = |ui: &Ui, s: &str, wrap_width: f32| -> Arc<egui::Galley> {
                    let w = if wrap { wrap_width } else { f32::INFINITY };
                    let job = highlight::layout(s, lang, font.clone(), dark, None, search, w);
                    ui.fonts(|f| f.layout_job(job))
                };
                let mut view = text;
                ui.add(
                    TextEdit::multiline(&mut view)
                        .id_salt(id)
                        .frame(false)
                        .code_editor()
                        .margin(vec2(8.0, 8.0))
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter),
                );
            });
        });
    });
}

/// Per-table UI state kept by the caller.
#[derive(Debug, Clone, Default)]
pub struct TableState {
    pub bulk: bool,
    pub bulk_text: String,
}

const ROW_H: f32 = 32.0;

/// Draws the table header; returns the column x-ranges.
fn table_header(ui: &mut Ui, labels: &[&str], fractions: &[f32], lead: f32, trail: f32) -> Vec<(f32, f32)> {
    let p = theme::palette(ui);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 28.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.surface_hover);
    let flex = width - lead - trail;
    let mut x = rect.left() + lead;
    let mut cols = Vec::new();
    for (label, f) in labels.iter().zip(fractions) {
        let w = flex * f;
        cols.push((x, x + w));
        ui.painter().text(
            egui::pos2(x + 9.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label.to_uppercase(),
            FontId::proportional(10.5),
            p.text_faint,
        );
        x += w;
    }
    cols
}

fn table_row_bg(ui: &mut Ui, rect: Rect, cols: &[(f32, f32)], last: bool) -> bool {
    let p = theme::palette(ui);
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        let radius = if last { egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 } } else { egui::CornerRadius::ZERO };
        ui.painter().rect_filled(rect, radius, p.surface_hover.gamma_multiply(0.6));
    }
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, p.border));
    for (x, _) in cols.iter().skip(1) {
        ui.painter().vline(*x, rect.y_range(), Stroke::new(1.0_f32, p.border));
    }
    hovered
}

fn in_cell<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_new_ui(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(Align::Center)), add).inner
}

fn bulk_toggle(ui: &mut Ui, rows: &mut Vec<KeyValue>, state: &mut TableState) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        let enabled = rows.iter().filter(|r| r.enabled).count();
        ui.label(RichText::new(format!("{enabled} of {} enabled", rows.len())).color(p.text_faint).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let label = if state.bulk { "Table view" } else { "Bulk edit" };
            let glyph = if state.bulk { icon::LIST } else { icon::PENCIL_SIMPLE };
            if kit::small_button(ui, kit::Kind::Ghost, Some(glyph), label)
                .on_hover_text("Edit as `key:value` lines")
                .clicked()
            {
                if state.bulk {
                    *rows = parse_bulk(&state.bulk_text);
                    changed = true;
                } else {
                    state.bulk_text = to_bulk(rows);
                }
                state.bulk = !state.bulk;
            }
        });
    });
    changed
}

/// Editable key/value table with enable checkboxes, descriptions, delete
/// buttons, an always-present empty row and bulk-edit mode.
/// Returns true if anything changed.
pub fn kv_table(
    ui: &mut Ui,
    id: &str,
    rows: &mut Vec<KeyValue>,
    state: &mut TableState,
    vars: &VarContext,
    describe: bool,
) -> bool {
    let p = theme::palette(ui);
    let mut changed = bulk_toggle(ui, rows, state);
    ui.add_space(4.0);
    if state.bulk {
        let r = code_editor(ui, &format!("{id}-bulk"), &mut state.bulk_text, Lang::Plain, None, 8, 12.5);
        if r.changed() {
            *rows = parse_bulk(&state.bulk_text);
            changed = true;
        }
        return changed;
    }
    let (labels, fractions): (&[&str], &[f32]) = if describe {
        (&["Key", "Value", "Description"], &[0.32, 0.40, 0.28])
    } else {
        (&["Key", "Value"], &[0.42, 0.58])
    };
    let mut remove = None;
    egui::Frame::new().stroke(Stroke::new(1.0_f32, p.border)).corner_radius(8).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let cols = table_header(ui, labels, fractions, 36.0, 34.0);
        let n = rows.len();
        for i in 0..=n {
            let is_new = i == n;
            let mut row = if is_new { KeyValue::new("", "") } else { rows[i].clone() };
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), egui::Sense::hover());
            let hovered = table_row_bg(ui, rect, &cols, is_new);
            let mut row_changed = false;
            if !is_new {
                in_cell(ui, Rect::from_min_max(rect.min, egui::pos2(rect.left() + 36.0, rect.bottom())), |ui| {
                    ui.add_space(9.0);
                    row_changed |= kit::checkbox(ui, &mut row.enabled).changed();
                });
            }
            let dim = !row.enabled && !is_new;
            for (c, (x0, x1)) in cols.iter().enumerate() {
                let cell = Rect::from_min_max(egui::pos2(*x0 + 1.0, rect.top()), egui::pos2(*x1, rect.bottom()));
                in_cell(ui, cell, |ui| {
                    ui.push_id((id, i, c), |ui| {
                        if dim {
                            ui.multiply_opacity(0.5);
                        }
                        let (text, hint) = match c {
                            0 => (&mut row.key, if is_new { "Key" } else { "" }),
                            1 => (&mut row.value, if is_new { "Value" } else { "" }),
                            _ => (&mut row.description, if is_new { "Description" } else { "" }),
                        };
                        let w = cell.width();
                        if c < 2 {
                            row_changed |= var_field_ext(ui, text, vars, hint, w, false, 12.5).changed();
                        } else {
                            let edit = TextEdit::singleline(text)
                                .frame(false)
                                .margin(vec2(8.0, 7.0))
                                .hint_text(RichText::new(hint).color(p.text_faint));
                            row_changed |= ui.add_sized([w, ROW_H], edit).changed();
                        }
                    });
                });
            }
            if !is_new && hovered {
                in_cell(ui, Rect::from_min_max(egui::pos2(rect.right() - 32.0, rect.top()), rect.max), |ui| {
                    if kit::icon_button(ui, icon::TRASH, "Delete row").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if row_changed {
                changed = true;
                if is_new {
                    rows.push(row);
                } else {
                    rows[i] = row;
                }
            }
        }
    });
    if let Some(i) = remove {
        rows.remove(i);
        changed = true;
    }
    changed
}

/// Multipart form editor (text or file rows).
pub fn form_table(ui: &mut Ui, id: &str, rows: &mut Vec<FormField>, vars: &VarContext) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    let mut remove = None;
    egui::Frame::new().stroke(Stroke::new(1.0_f32, p.border)).corner_radius(8).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let cols = table_header(ui, &["Key", "Type", "Value"], &[0.32, 0.14, 0.54], 36.0, 34.0);
        let n = rows.len();
        for i in 0..=n {
            let is_new = i == n;
            let mut row = if is_new { FormField { enabled: true, ..Default::default() } } else { rows[i].clone() };
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), egui::Sense::hover());
            let hovered = table_row_bg(ui, rect, &cols, is_new);
            let mut row_changed = false;
            if !is_new {
                in_cell(ui, Rect::from_min_max(rect.min, egui::pos2(rect.left() + 36.0, rect.bottom())), |ui| {
                    ui.add_space(9.0);
                    row_changed |= kit::checkbox(ui, &mut row.enabled).changed();
                });
            }
            let cell = |c: usize| {
                Rect::from_min_max(egui::pos2(cols[c].0 + 1.0, rect.top()), egui::pos2(cols[c].1, rect.bottom()))
            };
            in_cell(ui, cell(0), |ui| {
                ui.push_id((id, i, "k"), |ui| {
                    row_changed |= var_field_ext(
                        ui,
                        &mut row.key,
                        vars,
                        if is_new { "Key" } else { "" },
                        cell(0).width(),
                        false,
                        12.5,
                    )
                    .changed()
                });
            });
            in_cell(ui, cell(1), |ui| {
                ui.add_space(4.0);
                egui::ComboBox::from_id_salt((id, i, "t"))
                    .width(cell(1).width() - 16.0)
                    .selected_text(if row.kind == FormFieldKind::File { "File" } else { "Text" })
                    .show_ui(ui, |ui| {
                        row_changed |= ui.selectable_value(&mut row.kind, FormFieldKind::Text, "Text").changed();
                        row_changed |= ui.selectable_value(&mut row.kind, FormFieldKind::File, "File").changed();
                    });
            });
            in_cell(ui, cell(2), |ui| match row.kind {
                FormFieldKind::Text => {
                    ui.push_id((id, i, "v"), |ui| {
                        row_changed |= var_field_ext(
                            ui,
                            &mut row.value,
                            vars,
                            if is_new { "Value" } else { "" },
                            cell(2).width(),
                            false,
                            12.5,
                        )
                        .changed()
                    });
                }
                FormFieldKind::File => {
                    ui.add_space(6.0);
                    let names: Vec<String> = row
                        .files
                        .iter()
                        .map(|f| {
                            std::path::Path::new(f)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| f.clone())
                        })
                        .collect();
                    let label = if names.is_empty() { "Select files…".to_string() } else { names.join(", ") };
                    if kit::small_button(ui, kit::Kind::Secondary, Some(icon::FILE_ARROW_UP), &label)
                        .on_hover_text(row.files.join("\n"))
                        .clicked()
                        && let Some(paths) = rfd::FileDialog::new().pick_files()
                    {
                        row.files = paths.iter().map(|p| p.display().to_string()).collect();
                        row_changed = true;
                    }
                }
            });
            if !is_new && hovered {
                in_cell(ui, Rect::from_min_max(egui::pos2(rect.right() - 32.0, rect.top()), rect.max), |ui| {
                    if kit::icon_button(ui, icon::TRASH, "Delete row").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if row_changed {
                changed = true;
                if is_new {
                    rows.push(row);
                } else {
                    rows[i] = row;
                }
            }
        }
    });
    if let Some(i) = remove {
        rows.remove(i);
        changed = true;
    }
    changed
}

/// What the auth editor asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthAction {
    None,
    GetNewToken,
}

/// A two-column form row: muted label on the left, control on the right.
pub fn form_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(190.0, 30.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            p.text_muted,
        );
        add(ui)
    })
    .inner
}

/// A soft informational note with an icon.
pub fn note(ui: &mut Ui, glyph: &str, color: egui::Color32, text: &str) {
    let p = theme::palette(ui);
    kit::card(&p).fill(p.surface_hover).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(glyph).size(16.0).color(color));
            ui.label(RichText::new(text).color(p.text_muted));
        });
    });
}

/// Auth type picker and parameter form. `inherited` describes the effective
/// auth when the type is Inherit.
pub fn auth_editor(
    ui: &mut Ui,
    id: &str,
    auth: &mut Auth,
    vars: &VarContext,
    allow_inherit: bool,
    inherited: Option<(String, String)>,
) -> (bool, AuthAction) {
    let p = theme::palette(ui);
    let mut changed = false;
    let mut action = AuthAction::None;
    let field_w = (ui.available_width() - 210.0).clamp(220.0, 520.0);
    form_row(ui, "Auth type", |ui| {
        let mut picked = None;
        kit::dropdown(ui, &format!("{id}-type"), Some((icon::KEY, p.accent)), auth.kind.label(), None, field_w, |ui| {
            for kind in AuthType::SELECTABLE.iter().filter(|k| allow_inherit || **k != AuthType::Inherit) {
                if kit::menu_item(ui, &auth.kind == kind, kind.label()).clicked() {
                    picked = Some(kind.clone());
                }
            }
        });
        if let Some(k) = picked
            && k != auth.kind
        {
            auth.kind = k;
            changed = true;
        }
    });
    ui.add_space(10.0);
    match &auth.kind {
        AuthType::Inherit => {
            let text = match inherited {
                Some((label, owner)) if !owner.is_empty() => format!("This request uses {label} from “{owner}”."),
                _ => "This request inherits auth from its parent. It has no parent, so no auth is sent.".to_string(),
            };
            note(ui, icon::INFO, p.info, &text);
            return (changed, action);
        }
        AuthType::NoAuth => {
            note(ui, icon::INFO, p.text_muted, "This request does not use any authorization.");
            return (changed, action);
        }
        kind if !kind.is_supported() => {
            let text =
                format!("{} is kept when importing and exporting, but Courier cannot send it yet.", kind.label());
            note(ui, icon::WARNING, p.warning, &text);
            ui.add_space(8.0);
        }
        _ => {}
    }
    for field in auth::fields(&auth.kind) {
        form_row(ui, field.label, |ui| match field.kind {
            FieldKind::Bool => {
                let mut v = auth.get_bool(field.key);
                if kit::checkbox(ui, &mut v).changed() {
                    auth.set(field.key, v);
                    changed = true;
                }
            }
            FieldKind::Choice(options) => {
                let v = auth.get(field.key);
                let shown = if v.is_empty() { options.first().copied().unwrap_or("").to_string() } else { v.clone() };
                let mut picked = None;
                let label = if shown.is_empty() { "(none)" } else { shown.as_str() };
                kit::dropdown(ui, &format!("{id}-{}", field.key), None, label, None, field_w, |ui| {
                    for o in options {
                        if kit::menu_item(ui, v == *o, if o.is_empty() { "(none)" } else { o }).clicked() {
                            picked = Some(o.to_string());
                        }
                    }
                });
                if let Some(o) = picked {
                    auth.set(field.key, o);
                    changed = true;
                }
            }
            FieldKind::Multiline => {
                let mut v = auth.get(field.key);
                let edit = TextEdit::multiline(&mut v).id_salt((id, field.key)).code_editor().margin(vec2(8.0, 6.0));
                if ui.add_sized([field_w, 84.0], edit).changed() {
                    auth.set(field.key, v);
                    changed = true;
                }
            }
            FieldKind::Text | FieldKind::Secret => {
                let mut v = auth.get(field.key);
                let r = ui.push_id((id, field.key), |ui| {
                    if field.kind == FieldKind::Secret && VarContext::references(&v).is_empty() {
                        let edit = TextEdit::singleline(&mut v).password(true).margin(vec2(8.0, 6.0));
                        ui.add_sized([field_w, 30.0], edit)
                    } else {
                        var_field(ui, &mut v, vars, "", field_w)
                    }
                });
                if r.inner.changed() {
                    auth.set(field.key, v);
                    changed = true;
                }
            }
        });
        ui.add_space(2.0);
    }
    if auth.kind == AuthType::OAuth2 {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(198.0);
            if kit::button(ui, kit::Kind::Primary, Some(icon::KEY), "Get New Access Token").clicked() {
                action = AuthAction::GetNewToken;
            }
            if !auth.get("tokenExpiresAt").is_empty() {
                ui.label(
                    RichText::new(format!("Current token expires {}", auth.get("tokenExpiresAt"))).color(p.text_faint),
                );
            }
        });
    }
    (changed, action)
}

/// Small coloured method label.
pub fn method_label(ui: &mut Ui, method: &str, width: f32) {
    kit::method_tag(ui, method, width);
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.2} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

pub fn human_time(ms: f64) -> String {
    if ms < 1000.0 { format!("{ms:.0} ms") } else { format!("{:.2} s", ms / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_formats() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.00 MB");
        assert_eq!(human_time(12.4), "12 ms");
        assert_eq!(human_time(1500.0), "1.50 s");
    }
}
