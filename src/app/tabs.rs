//! Collection/folder settings, environment editor and example viewer tabs.

use super::highlight::Lang;
use super::kit::{self, icon};
use super::widgets::{self, AuthAction, TableState};
use super::{CourierApp, theme};
use crate::model::*;
use egui::{RichText, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSection {
    Overview,
    Auth,
    Scripts,
    Variables,
}

pub struct NodeTab {
    pub collection_id: String,
    /// `None` for the collection itself, otherwise a folder id.
    pub node_id: Option<String>,
    pub section: NodeSection,
    pub post_script: bool,
    pub preview_description: bool,
}

impl NodeTab {
    pub fn new(collection_id: &str, node_id: Option<&str>) -> Self {
        Self {
            collection_id: collection_id.into(),
            node_id: node_id.map(str::to_string),
            section: NodeSection::Overview,
            post_script: false,
            preview_description: true,
        }
    }
}

/// Page title row: icon tile, editable name, subtitle and trailing actions.
fn page_header(
    ui: &mut Ui,
    glyph: &str,
    name: &mut String,
    editable: bool,
    subtitle: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 11.0, p.accent_soft);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(21.0),
            p.accent,
        );
        ui.add_space(6.0);
        ui.vertical(|ui| {
            if editable {
                ui.add(
                    egui::TextEdit::singleline(name)
                        .font(theme::title_style())
                        .frame(false)
                        .desired_width(420.0)
                        .margin(egui::vec2(0.0, 0.0)),
                );
            } else {
                ui.label(RichText::new(name.as_str()).text_style(theme::title_style()));
            }
            ui.label(RichText::new(subtitle).color(p.text_muted).size(12.5));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
    });
    ui.add_space(14.0);
}

pub fn show_node(app: &mut CourierApp, ui: &mut Ui, tab_id: u64, tab: &mut NodeTab) {
    let cid = tab.collection_id.clone();
    let vars = app.var_ctx(Some(&cid));
    let font = app.settings.editor_font_size;
    let Some(c) = app.collection(&cid) else {
        kit::empty_state(ui, icon::STACK, "Collection not found", "It may have been deleted.");
        return;
    };
    // Work on a copy and write back only when something changed.
    let mut coll = c.clone();
    let before = coll.clone();
    let is_collection = tab.node_id.is_none();
    let mut run = false;
    let mut fetch_token = None;
    let p = theme::palette(ui);
    let subtitle = if is_collection {
        format!("Collection · {} requests · {} variables", before.request_count(), before.variables.len())
    } else {
        let n = before.requests_under(tab.node_id.as_deref()).len();
        format!("Folder in {} · {n} requests", before.name)
    };

    {
        let (name, description, auth, scripts): (&mut String, &mut String, &mut Auth, &mut Scripts) =
            match tab.node_id.as_deref() {
                None => (&mut coll.name, &mut coll.description, &mut coll.auth, &mut coll.scripts),
                Some(fid) => match coll.find_mut(fid) {
                    Some(Item::Folder(f)) => (&mut f.name, &mut f.description, &mut f.auth, &mut f.scripts),
                    _ => {
                        kit::empty_state(ui, icon::FOLDER, "Folder not found", "It may have been deleted.");
                        return;
                    }
                },
            };
        page_header(ui, if is_collection { icon::STACK } else { icon::FOLDER_OPEN }, name, true, &subtitle, |ui| {
            if kit::button(ui, kit::Kind::Primary, Some(icon::PLAY), "Run").clicked() {
                run = true;
            }
        });
        let mut items = vec![
            (NodeSection::Overview, "Overview", None),
            (
                NodeSection::Auth,
                "Authorization",
                (!matches!(auth.kind, AuthType::Inherit | AuthType::NoAuth)).then(|| "•".to_string()),
            ),
            (
                NodeSection::Scripts,
                "Scripts",
                (!scripts.pre_request.trim().is_empty() || !scripts.post_response.trim().is_empty())
                    .then(|| "•".to_string()),
            ),
        ];
        if is_collection {
            items.push((
                NodeSection::Variables,
                "Variables",
                (!before.variables.is_empty()).then(|| before.variables.len().to_string()),
            ));
        }
        kit::tabs(ui, &mut tab.section, &items);
        ui.add_space(14.0);
        match tab.section {
            NodeSection::Overview => {
                ui.horizontal(|ui| {
                    kit::section_label(ui, "Description");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        kit::segmented(ui, &mut tab.preview_description, &[(true, "Preview"), (false, "Edit")]);
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if tab.preview_description {
                        kit::card(&p).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if description.trim().is_empty() {
                                ui.label(
                                    RichText::new(
                                        "No description yet. Switch to Edit to add one — Markdown is supported.",
                                    )
                                    .color(p.text_faint),
                                );
                            } else {
                                markdown_preview(ui, description);
                            }
                        });
                    } else {
                        ui.add(
                            egui::TextEdit::multiline(description)
                                .desired_rows(14)
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(12.0, 10.0))
                                .hint_text("Describe this collection or folder (Markdown)"),
                        );
                    }
                });
            }
            NodeSection::Auth => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let text = if is_collection {
                        "Used by every request in the collection that inherits its auth."
                    } else {
                        "Used by every request in this folder that inherits its auth."
                    };
                    ui.label(RichText::new(text).color(p.text_muted));
                    ui.add_space(10.0);
                    let inherited = Some(("the parent's auth".to_string(), String::new()));
                    let (_, action) =
                        widgets::auth_editor(ui, &format!("nodeauth{tab_id}"), auth, &vars, !is_collection, inherited);
                    if action == AuthAction::GetNewToken {
                        fetch_token = Some(auth.clone());
                    }
                });
            }
            NodeSection::Scripts => {
                ui.horizontal(|ui| {
                    kit::segmented(ui, &mut tab.post_script, &[(false, "Pre-request"), (true, "Post-response")]);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(if tab.post_script {
                            "Runs after every request below this node, before the request's own script."
                        } else {
                            "Runs before every request below this node, before the request's own script."
                        })
                        .color(p.text_faint)
                        .size(12.0),
                    );
                });
                ui.add_space(8.0);
                let target = if tab.post_script { &mut scripts.post_response } else { &mut scripts.pre_request };
                widgets::code_editor(
                    ui,
                    &format!("nodescript{tab_id}{}", tab.post_script),
                    target,
                    Lang::Js,
                    None,
                    16,
                    font,
                );
            }
            NodeSection::Variables => {}
        }
    }
    if tab.section == NodeSection::Variables && is_collection {
        ui.label(RichText::new("Available to every request in this collection. Current values stay on this machine and are not exported.").color(p.text_muted));
        ui.add_space(10.0);
        egui::ScrollArea::vertical().id_salt(("vars", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
            variables_table(ui, &format!("collvars{tab_id}"), &mut coll.variables);
        });
    }
    if coll != before {
        if let Some(c) = app.collection_mut(&cid) {
            *c = coll;
        }
        app.persist_collection(&cid);
    }
    if run {
        app.open_runner(&cid, tab.node_id.as_deref());
    }
    if let Some(auth) = fetch_token {
        app.fetch_token(tab_id, auth, Some(cid));
    }
}

/// A small Markdown renderer: headings, bullet lists, code blocks and paragraphs.
fn markdown_preview(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui);
    let mut in_code = false;
    let mut code = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                egui::Frame::new()
                    .fill(p.input)
                    .stroke(egui::Stroke::new(1.0_f32, p.border))
                    .inner_margin(10)
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(code.trim_end()).monospace());
                    });
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        let t = line.trim();
        if let Some(h) = t.strip_prefix("### ") {
            ui.label(RichText::new(h).strong().size(15.0));
        } else if let Some(h) = t.strip_prefix("## ") {
            ui.label(RichText::new(h).strong().size(17.0));
        } else if let Some(h) = t.strip_prefix("# ") {
            ui.label(RichText::new(h).strong().size(20.0));
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("•").color(p.accent));
                ui.label(item.replace("**", "").replace('`', ""));
            });
        } else if t.is_empty() {
            ui.add_space(6.0);
        } else {
            ui.label(RichText::new(t.replace("**", "").replace('`', "")).color(p.text));
        }
    }
    if in_code && !code.is_empty() {
        ui.label(RichText::new(code.trim_end()).monospace());
    }
}

fn cell_edit<'a>(text: &'a mut String, hint: &str, secret: bool, faint: egui::Color32) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .frame(false)
        .password(secret)
        .margin(egui::vec2(8.0, 7.0))
        .hint_text(RichText::new(hint.to_string()).color(faint))
}

/// Editable variables table: enabled, key, type, initial, current, delete.
pub fn variables_table(ui: &mut Ui, id: &str, vars: &mut Vec<Variable>) -> bool {
    let p = theme::palette(ui);
    let mut changed = false;
    let mut remove = None;
    egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, p.border)).corner_radius(8).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = ui.available_width();
        let (hrect, _) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::hover());
        ui.painter().rect_filled(hrect, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.surface_hover);
        let lead = 36.0;
        let type_w = 96.0;
        let trail = 34.0;
        let flex = (width - lead - type_w - trail).max(240.0);
        let key_w = flex * 0.3;
        let val_w = flex * 0.35;
        let xs = [
            hrect.left() + lead,
            hrect.left() + lead + key_w,
            hrect.left() + lead + key_w + type_w,
            hrect.left() + lead + key_w + type_w + val_w,
        ];
        for (x, label) in xs.iter().zip(["Variable", "Type", "Initial value", "Current value"]) {
            ui.painter().text(
                egui::pos2(x + 9.0, hrect.center().y),
                egui::Align2::LEFT_CENTER,
                label.to_uppercase(),
                egui::FontId::proportional(10.5),
                p.text_faint,
            );
        }
        let n = vars.len();
        for i in 0..=n {
            let is_new = i == n;
            let mut v = if is_new { Variable::new("", "") } else { vars[i].clone() };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::hover());
            let hovered = ui.rect_contains_pointer(rect);
            if hovered {
                ui.painter().rect_filled(rect, 0.0, p.surface_hover.gamma_multiply(0.6));
            }
            ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0_f32, p.border));
            for x in &xs[1..] {
                ui.painter().vline(*x, rect.y_range(), egui::Stroke::new(1.0_f32, p.border));
            }
            let cell = |x0: f32, w: f32| {
                egui::Rect::from_min_size(egui::pos2(x0 + 1.0, rect.top()), egui::vec2(w - 1.0, 32.0))
            };
            let mut row_changed = false;
            let place = |ui: &mut Ui, r: egui::Rect, add: &mut dyn FnMut(&mut Ui)| {
                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| add(ui),
                );
            };
            if !is_new {
                place(ui, egui::Rect::from_min_size(rect.min, egui::vec2(lead, 32.0)), &mut |ui| {
                    ui.add_space(9.0);
                    row_changed |= kit::checkbox(ui, &mut v.enabled).changed();
                });
            }
            let faint = p.text_faint;
            place(ui, cell(xs[0], key_w), &mut |ui| {
                row_changed |= ui
                    .add_sized(
                        [key_w - 1.0, 32.0],
                        cell_edit(&mut v.key, if is_new { "Add a variable" } else { "" }, false, faint),
                    )
                    .changed();
            });
            place(ui, cell(xs[1], type_w), &mut |ui| {
                ui.add_space(6.0);
                let mut secret = v.secret;
                let label = if secret { "Secret" } else { "Default" };
                egui::ComboBox::from_id_salt((id, i, "t")).width(type_w - 20.0).selected_text(label).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut secret, false, "Default");
                        ui.selectable_value(&mut secret, true, "Secret");
                    },
                );
                if secret != v.secret {
                    v.secret = secret;
                    row_changed = true;
                }
            });
            let secret_now = v.secret;
            place(ui, cell(xs[2], val_w), &mut |ui| {
                row_changed |=
                    ui.add_sized([val_w - 1.0, 32.0], cell_edit(&mut v.initial, "", secret_now, faint)).changed();
            });
            let cur_w = width - (xs[3] - rect.left()) - trail;
            let mut current = v.current.clone().unwrap_or_default();
            let initial = v.initial.clone();
            let secret = v.secret;
            place(ui, cell(xs[3], cur_w), &mut |ui| {
                let hint = if v.current.is_none() { initial.as_str() } else { "" };
                if ui.add_sized([cur_w - 1.0, 32.0], cell_edit(&mut current, hint, secret, faint)).changed() {
                    v.current = if current.is_empty() { None } else { Some(current.clone()) };
                    row_changed = true;
                }
            });
            if !is_new && hovered {
                place(
                    ui,
                    egui::Rect::from_min_max(egui::pos2(rect.right() - trail, rect.top()), rect.max),
                    &mut |ui| {
                        if kit::icon_button(ui, icon::TRASH, "Delete variable").clicked() {
                            remove = Some(i);
                        }
                    },
                );
            }
            if row_changed {
                changed = true;
                if is_new {
                    vars.push(v);
                } else {
                    vars[i] = v;
                }
            }
        }
    });
    if let Some(i) = remove {
        vars.remove(i);
        changed = true;
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if kit::button(ui, kit::Kind::Secondary, Some(icon::ARROW_LINE_DOWN), "Persist all")
            .on_hover_text("Copy every current value into the initial value")
            .clicked()
        {
            vars.persist_all();
            changed = true;
        }
        if kit::button(ui, kit::Kind::Ghost, Some(icon::ARROW_COUNTER_CLOCKWISE), "Reset all")
            .on_hover_text("Discard current values and use the initial values")
            .clicked()
        {
            vars.reset_all();
            changed = true;
        }
    });
    changed
}

pub struct EnvTab {
    /// `None` for globals.
    pub env_id: Option<String>,
    pub search: String,
    pub table: TableState,
}

impl EnvTab {
    pub fn new(env_id: Option<&str>) -> Self {
        Self { env_id: env_id.map(str::to_string), search: String::new(), table: TableState::default() }
    }
}

pub fn show_env(app: &mut CourierApp, ui: &mut Ui, tab: &mut EnvTab) {
    let (mut env, is_globals) = match &tab.env_id {
        Some(id) => match app.environment(id) {
            Some(e) => (e.clone(), false),
            None => {
                kit::empty_state(ui, icon::GLOBE, "Environment not found", "It may have been deleted.");
                return;
            }
        },
        None => (app.globals.clone(), true),
    };
    let before = env.clone();
    let mut make_active = None;
    let p = theme::palette(ui);
    let active = app.active_env.as_deref() == Some(env.id.as_str());
    let subtitle = if is_globals {
        "Global variables are available in every request, whatever environment is active.".to_string()
    } else {
        format!(
            "{} variables · initial values are shared on export, current values stay on this machine",
            env.variables.len()
        )
    };
    let mut name = if is_globals { "Globals".to_string() } else { env.name.clone() };
    page_header(ui, icon::GLOBE, &mut name, !is_globals, &subtitle, |ui| {
        if !is_globals {
            if active {
                kit::badge(ui, &format!("{}  Active", icon::CHECK_CIRCLE), p.accent);
            } else if kit::button(ui, kit::Kind::Secondary, Some(icon::CHECK_CIRCLE), "Set active").clicked() {
                make_active = Some(env.id.clone());
            }
        }
    });
    if !is_globals {
        env.name = name;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        variables_table(ui, &format!("env{:?}", tab.env_id), &mut env.variables);
    });
    if env != before {
        if is_globals {
            app.globals = env;
            app.persist_globals();
        } else if let Some(e) = app.environments.iter_mut().find(|e| e.id == env.id) {
            *e = env.clone();
            app.persist_environment(&env.id);
        }
    }
    if let Some(id) = make_active {
        app.active_env = Some(id);
    }
}

pub struct ExampleTab {
    pub collection_id: String,
    pub request_id: String,
    pub request_name: String,
    pub example: Example,
    pub editing: bool,
}

pub fn show_example(app: &mut CourierApp, ui: &mut Ui, tab: &mut ExampleTab) {
    let p = theme::palette(ui);
    let before = tab.example.clone();
    let subtitle = format!("Example of {}", tab.request_name);
    let mut name = tab.example.name.clone();
    let mut delete = false;
    let editing = tab.editing;
    page_header(ui, icon::BOOKMARK_SIMPLE, &mut name, editing, &subtitle, |ui| {
        if kit::button(ui, kit::Kind::Danger, Some(icon::TRASH), "Delete").clicked() {
            delete = true;
        }
        let label = if editing { "Done" } else { "Edit" };
        if kit::button(ui, kit::Kind::Secondary, Some(icon::PENCIL_SIMPLE), label).clicked() {
            tab.editing = !tab.editing;
        }
    });
    tab.example.name = name;
    if delete {
        if let Some(r) = app.collection_mut(&tab.collection_id).and_then(|c| c.find_request_mut(&tab.request_id)) {
            r.examples.retain(|e| e.id != tab.example.id);
        }
        app.persist_collection(&tab.collection_id.clone());
        app.toast("Example deleted");
    }
    if let Some(req) = &tab.example.original_request {
        egui::Frame::new()
            .fill(p.input)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .corner_radius(9)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    kit::method_tag(ui, &req.method, 56.0);
                    ui.label(RichText::new(&req.url).monospace());
                });
            });
        ui.add_space(12.0);
    }
    ui.horizontal(|ui| {
        kit::section_label(ui, "Response");
        ui.add_space(8.0);
        if tab.editing {
            ui.add(egui::DragValue::new(&mut tab.example.code).range(100..=599));
            widgets::text_input(ui, &mut tab.example.status, "Status text", 160.0);
        } else {
            kit::badge(
                ui,
                &format!("{} {}", tab.example.code, tab.example.status),
                theme::status_color(tab.example.code, p.dark),
            );
        }
    });
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new(format!("Headers ({})", tab.example.headers.len())).color(p.text_muted))
        .show(ui, |ui| {
            let mut state = TableState::default();
            if tab.editing {
                widgets::kv_table(
                    ui,
                    "example_headers",
                    &mut tab.example.headers,
                    &mut state,
                    &crate::vars::VarContext::default(),
                    false,
                );
            } else {
                for h in &tab.example.headers {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&h.key).monospace().color(p.text_muted));
                        ui.label(RichText::new(&h.value).monospace());
                    });
                }
            }
        });
    ui.add_space(8.0);
    let lang = match tab.example.preview_language.as_deref() {
        Some("json") => Lang::Json,
        Some("xml") | Some("html") => Lang::Xml,
        Some("javascript") => Lang::Js,
        _ => Lang::Plain,
    };
    let font = app.settings.editor_font_size;
    if tab.editing {
        widgets::code_editor(ui, "example_body", &mut tab.example.body, lang, None, 16, font);
    } else {
        let pretty = super::highlight::pretty(&tab.example.body, lang);
        widgets::code_view(ui, "example_body_view", &pretty, lang, "", font, false);
    }
    if tab.example != before && !delete {
        if let Some(r) = app.collection_mut(&tab.collection_id).and_then(|c| c.find_request_mut(&tab.request_id))
            && let Some(e) = r.examples.iter_mut().find(|e| e.id == tab.example.id)
        {
            *e = tab.example.clone();
        }
        app.persist_collection(&tab.collection_id.clone());
    }
}
