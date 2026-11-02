//! The request builder (URL bar, params, auth, headers, body, scripts,
//! settings) and the response viewer.

use super::highlight::{self, Lang};
use super::kit::{self, icon};
use super::widgets::{self, AuthAction, TableState, human_size, human_time};
use super::{CourierApp, Dialog, dialogs, theme};
use crate::exec::{self, Execution};
use crate::model::*;
use crate::settings::Layout;
use crate::vars::VarContext;
use egui::{RichText, Ui};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReqSection {
    Params,
    Auth,
    Headers,
    Body,
    Scripts,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespSection {
    Body,
    Headers,
    Cookies,
    Tests,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyView {
    Pretty,
    Raw,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFilter {
    All,
    Passed,
    Failed,
    Skipped,
}

pub enum RespState {
    Empty,
    Loading { started: Instant, cancel: CancellationToken },
    Done(Box<Execution>),
}

pub struct RequestTab {
    pub source: Option<(String, String)>,
    pub item: RequestItem,
    /// Last saved version (for the dirty marker); `None` for unsaved tabs.
    pub saved: Option<RequestItem>,
    pub section: ReqSection,
    pub post_script: bool,
    pub params: TableState,
    pub path_vars: TableState,
    pub headers: TableState,
    pub urlencoded: TableState,
    pub show_auto_headers: bool,
    pub resp: RespState,
    pub resp_section: RespSection,
    pub body_view: BodyView,
    pub search: String,
    pub test_filter: TestFilter,
    pub pretty_cache: Option<(usize, String)>,
}

impl RequestTab {
    pub fn new(source: Option<(String, String)>, item: RequestItem, saved: Option<RequestItem>) -> Self {
        Self {
            source,
            item,
            saved,
            section: ReqSection::Params,
            post_script: true,
            params: TableState::default(),
            path_vars: TableState::default(),
            headers: TableState::default(),
            urlencoded: TableState::default(),
            show_auto_headers: false,
            resp: RespState::Empty,
            resp_section: RespSection::Body,
            body_view: BodyView::Pretty,
            search: String::new(),
            test_filter: TestFilter::All,
            pretty_cache: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.saved {
            Some(s) => s != &self.item,
            None => !self.item.request.url.is_empty() || self.item.request.body.mode != BodyMode::None,
        }
    }
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub fn show(app: &mut CourierApp, ui: &mut Ui, idx: usize, tab_id: u64, tab: &mut RequestTab) {
    let collection_id = tab.source.as_ref().map(|(c, _)| c.clone());
    let vars = app.var_ctx(collection_id.as_deref());
    header(app, ui, idx, tab);
    ui.add_space(10.0);
    url_bar(app, ui, idx, tab, &vars);
    ui.add_space(12.0);
    let p = theme::palette(ui);
    let side = app.settings.layout == Layout::SideBySide;
    let panel_frame = egui::Frame::new().fill(p.canvas);
    if side {
        egui::SidePanel::left(egui::Id::new(("req_editor", tab_id)))
            .resizable(true)
            .frame(panel_frame.inner_margin(egui::Margin { left: 0, right: 14, top: 0, bottom: 0 }))
            .default_width(ui.available_width() / 2.0)
            .width_range(320.0..=(ui.available_width() - 300.0).max(321.0))
            .show_inside(ui, |ui| request_editor(app, ui, tab_id, tab, &vars));
    } else {
        egui::TopBottomPanel::top(egui::Id::new(("req_editor", tab_id)))
            .resizable(true)
            .frame(panel_frame.inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 12 }))
            .default_height(ui.available_height() * 0.46)
            .height_range(140.0..=(ui.available_height() - 140.0).max(141.0))
            .show_inside(ui, |ui| request_editor(app, ui, tab_id, tab, &vars));
    }
    let margin = if side {
        egui::Margin { left: 14, right: 0, top: 0, bottom: 0 }
    } else {
        egui::Margin { left: 0, right: 0, top: 12, bottom: 0 }
    };
    egui::CentralPanel::default()
        .frame(panel_frame.inner_margin(margin))
        .show_inside(ui, |ui| response_view(app, ui, tab));
}

fn header(app: &mut CourierApp, ui: &mut Ui, idx: usize, tab: &mut RequestTab) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        if let Some((cid, rid)) = &tab.source
            && let Some(c) = app.collection(cid)
        {
            ui.label(RichText::new(icon::STACK).color(p.text_faint));
            let mut path = vec![c.name.clone()];
            path.extend(c.ancestors(rid).unwrap_or_default().iter().map(|f| f.name.clone()));
            for part in path {
                ui.label(RichText::new(part).color(p.text_muted));
                ui.label(RichText::new(icon::CARET_RIGHT).size(10.0).color(p.text_faint));
            }
        } else {
            ui.label(RichText::new(icon::FILE_TEXT).color(p.text_faint));
            ui.label(RichText::new("Unsaved request").color(p.text_muted));
            ui.label(RichText::new(icon::CARET_RIGHT).size(10.0).color(p.text_faint));
        }
        let mut name = tab.item.name.clone();
        let r = ui.add(
            egui::TextEdit::singleline(&mut name)
                .frame(false)
                .font(theme::title_style())
                .desired_width((ui.available_width() - 210.0).max(160.0))
                .margin(egui::vec2(2.0, 0.0)),
        );
        if r.changed() {
            tab.item.name = name;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let dirty = tab.is_dirty();
            let save = kit::button(
                ui,
                if dirty { kit::Kind::Secondary } else { kit::Kind::Ghost },
                Some(icon::FLOPPY_DISK),
                "Save",
            )
            .on_hover_text(super::platform::keys("Save (⌘S) · right-click for Save As"));
            if save.clicked() {
                match &tab.source {
                    Some((cid, rid)) => {
                        let (cid, rid) = (cid.clone(), rid.clone());
                        if let Some(r) = app.collection_mut(&cid).and_then(|c| c.find_request_mut(&rid)) {
                            *r = RequestItem { id: rid.clone(), ..tab.item.clone() };
                        }
                        app.persist_collection(&cid);
                        tab.saved = Some(tab.item.clone());
                        app.toast("Saved");
                    }
                    None => app.dialog = Some(Dialog::SaveAs(dialogs::SaveAsDialog::for_tab(app, idx, &tab.item.name))),
                }
            }
            save.context_menu(|ui| {
                if ui.button(format!("{}   Save As…", icon::FLOPPY_DISK)).clicked() {
                    app.dialog = Some(Dialog::SaveAs(dialogs::SaveAsDialog::for_tab(app, idx, &tab.item.name)));
                    ui.close_menu();
                }
            });
            if kit::icon_button(ui, icon::CODE, "Code snippet (cURL, HTTP)").clicked() {
                app.dialog = Some(Dialog::Snippet(dialogs::SnippetDialog::new(app, tab)));
            }
        });
    });
}

fn url_bar(app: &mut CourierApp, ui: &mut Ui, idx: usize, tab: &mut RequestTab, vars: &crate::vars::VarContext) {
    let p = theme::palette(ui);
    let loading = matches!(tab.resp, RespState::Loading { .. });
    let height = 40.0;
    let send_w = 112.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let bar_w = ui.available_width() - send_w - 8.0;
        let (bar, _) = ui.allocate_exact_size(egui::vec2(bar_w, height), egui::Sense::hover());
        let url_id = egui::Id::new(("url_field", idx, &tab.item.id));
        let focused = ui.memory(|m| m.focused()).is_some_and(|f| f == url_id || ui.memory(|m| m.has_focus(url_id)));
        ui.painter().rect(
            bar,
            9.0,
            p.input,
            egui::Stroke::new(1.0_f32, if focused { p.accent } else { p.border_strong }),
            egui::StrokeKind::Inside,
        );
        // Method picker.
        let method_rect = egui::Rect::from_min_size(bar.min, egui::vec2(108.0, height));
        let method = tab.item.request.method.clone();
        let mresp = ui.interact(method_rect, egui::Id::new(("method_btn", idx)), egui::Sense::click());
        if mresp.hovered() {
            ui.painter().rect_filled(method_rect.shrink(3.0), 7.0, p.surface_hover);
        }
        ui.painter().text(
            egui::pos2(method_rect.left() + 14.0, method_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &method,
            egui::FontId::monospace(13.0),
            theme::method_color(&method, p.dark),
        );
        ui.painter().text(
            egui::pos2(method_rect.right() - 12.0, method_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            icon::CARET_DOWN,
            egui::FontId::proportional(11.0),
            p.text_muted,
        );
        ui.painter().vline(
            method_rect.right(),
            (bar.top() + 9.0)..=(bar.bottom() - 9.0),
            egui::Stroke::new(1.0_f32, p.border_strong),
        );
        let popup = ui.make_persistent_id(("method_popup", idx));
        if mresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup));
        }
        let mresp = ui.interact(method_rect, egui::Id::new(("method_btn", idx)), egui::Sense::hover());
        egui::popup_below_widget(ui, popup, &mresp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(170.0);
            for m in METHODS {
                let text = RichText::new(m).monospace().color(theme::method_color(m, p.dark));
                if kit::menu_item(ui, method == m, text).clicked() {
                    tab.item.request.method = m.to_string();
                    ui.memory_mut(|mem| mem.close_popup());
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Custom").color(p.text_muted));
                let mut custom = if METHODS.contains(&method.as_str()) { String::new() } else { method.clone() };
                if widgets::text_input(ui, &mut custom, "PURGE", 90.0).changed() && !custom.trim().is_empty() {
                    tab.item.request.method = custom.trim().to_uppercase();
                }
            });
        });
        // URL field.
        let url_rect = egui::Rect::from_min_max(
            egui::pos2(method_rect.right() + 4.0, bar.top() + 1.0),
            egui::pos2(bar.right() - 4.0, bar.bottom() - 1.0),
        );
        let mut url = tab.item.request.url.clone();
        let r = ui
            .allocate_new_ui(
                egui::UiBuilder::new().max_rect(url_rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    let dark = p.dark;
                    let font = egui::FontId::monospace(13.5);
                    let mut layouter = |ui: &Ui, s: &str, wrap: f32| {
                        let job = highlight::layout(s, Lang::Plain, font.clone(), dark, Some(vars), "", wrap);
                        ui.fonts(|f| f.layout_job(job))
                    };
                    ui.add_sized(
                        url_rect.size(),
                        egui::TextEdit::singleline(&mut url)
                            .id(url_id)
                            .frame(false)
                            .margin(egui::vec2(10.0, 10.0))
                            .hint_text(RichText::new("Enter a URL or paste a cURL command").color(p.text_faint))
                            .layouter(&mut layouter),
                    )
                },
            )
            .inner;
        if app.focus_url {
            r.request_focus();
            app.focus_url = false;
        }
        if r.changed() {
            let trimmed = url.trim_start();
            if trimmed.starts_with("curl ") && url.len() > tab.item.request.url.len() + 5 {
                match crate::formats::curl::parse(trimmed) {
                    Ok(mut parsed) if !parsed.requests.is_empty() => {
                        let imported = parsed.requests.remove(0);
                        tab.item.request = imported.request;
                        if tab.item.name == "Untitled Request" {
                            tab.item.name = imported.name;
                        }
                        app.toast("Converted the cURL command into a request");
                    }
                    _ => tab.item.request.set_url(&url),
                }
            } else {
                tab.item.request.set_url(&url);
            }
        }
        if !VarContext::references(&tab.item.request.url).is_empty() {
            widgets::var_hover_public(&r, &tab.item.request.url, vars);
        }
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !loading {
            app.deferred.push(super::Deferred::Send(idx, None));
        }
        // Send button with a split menu.
        let (send_rect, _) = ui.allocate_exact_size(egui::vec2(send_w, height), egui::Sense::hover());
        let main = egui::Rect::from_min_max(send_rect.min, egui::pos2(send_rect.right() - 30.0, send_rect.bottom()));
        let caret = egui::Rect::from_min_max(egui::pos2(send_rect.right() - 30.0, send_rect.top()), send_rect.max);
        let mr = ui
            .interact(main, egui::Id::new(("send", idx)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let cr = ui
            .interact(caret, egui::Id::new(("send_caret", idx)), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let (fill, label, glyph, fg) = if loading {
            (if mr.hovered() { p.surface_active } else { p.surface_hover }, "Cancel", icon::X, p.text)
        } else {
            (
                if mr.hovered() || cr.hovered() { p.accent_hover } else { p.accent },
                "Send",
                icon::PAPER_PLANE_TILT,
                p.on_accent,
            )
        };
        ui.painter().rect_filled(send_rect, 9.0, fill);
        ui.painter().vline(
            caret.left(),
            (caret.top() + 10.0)..=(caret.bottom() - 10.0),
            egui::Stroke::new(1.0_f32, fg.gamma_multiply(0.35)),
        );
        let g = ui.painter().layout_no_wrap(format!("{glyph}  {label}"), egui::FontId::proportional(13.5), fg);
        ui.painter().galley(main.center() - g.size() / 2.0, g, fg);
        ui.painter().text(
            caret.center(),
            egui::Align2::CENTER_CENTER,
            icon::CARET_DOWN,
            egui::FontId::proportional(11.0),
            fg,
        );
        // Accessible label for tests and screen readers.
        mr.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if mr.clicked() {
            app.deferred.push(if loading { super::Deferred::Cancel(idx) } else { super::Deferred::Send(idx, None) });
        }
        let popup = ui.make_persistent_id(("send_menu", idx));
        if cr.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup));
        }
        egui::popup_below_widget(ui, popup, &cr, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(200.0);
            if kit::menu_item(ui, false, format!("{}   Send and Download", icon::DOWNLOAD_SIMPLE)).clicked()
                && let Some(path) = rfd::FileDialog::new().set_file_name("response").save_file()
            {
                app.deferred.push(super::Deferred::Send(idx, Some(path)));
            }
        });
    });
}

fn request_editor(
    app: &mut CourierApp,
    ui: &mut Ui,
    tab_id: u64,
    tab: &mut RequestTab,
    vars: &crate::vars::VarContext,
) {
    let p = theme::palette(ui);
    let r = &tab.item.request;
    let count = |n: usize| (n > 0).then(|| n.to_string());
    let params = r.query.iter().filter(|q| q.enabled).count() + r.path_variables.len();
    let headers = r.headers.iter().filter(|h| h.enabled).count();
    let has_scripts =
        !tab.item.scripts.pre_request.trim().is_empty() || !tab.item.scripts.post_response.trim().is_empty();
    let body_badge = (r.body.mode != BodyMode::None).then(|| "•".to_string());
    let auth_badge = (!matches!(r.auth.kind, AuthType::Inherit | AuthType::NoAuth)).then(|| "•".to_string());
    let settings_badge = (!r.settings.is_empty()).then(|| r.settings.len().to_string());
    kit::tabs(
        ui,
        &mut tab.section,
        &[
            (ReqSection::Params, "Params", count(params)),
            (ReqSection::Auth, "Authorization", auth_badge),
            (ReqSection::Headers, "Headers", count(headers)),
            (ReqSection::Body, "Body", body_badge),
            (ReqSection::Scripts, "Scripts", has_scripts.then(|| "•".to_string())),
            (ReqSection::Settings, "Settings", settings_badge),
        ],
    );
    ui.add_space(12.0);
    let font = app.settings.editor_font_size;
    let fill_editor = matches!(tab.section, ReqSection::Scripts)
        || (tab.section == ReqSection::Body && matches!(tab.item.request.body.mode, BodyMode::Raw | BodyMode::Graphql));
    if fill_editor {
        // Code editors fill the panel and scroll themselves.
        match tab.section {
            ReqSection::Scripts => scripts_editor(ui, tab_id, tab, font),
            _ => body_editor(app, ui, tab_id, tab, vars, font),
        }
        return;
    }
    egui::ScrollArea::vertical().id_salt(("req_scroll", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
        match tab.section {
            ReqSection::Params => {
                kit::section_label(ui, "Query parameters");
                ui.add_space(2.0);
                if widgets::kv_table(
                    ui,
                    &format!("params{tab_id}"),
                    &mut tab.item.request.query,
                    &mut tab.params,
                    vars,
                    true,
                ) {
                    tab.item.request.sync_url_from_query();
                    let url = tab.item.request.url.clone();
                    tab.item.request.path_variables =
                        crate::model::url::merge_path_variables(&tab.item.request.path_variables, &url);
                }
                if !tab.item.request.path_variables.is_empty() {
                    ui.add_space(16.0);
                    kit::section_label(ui, "Path variables");
                    ui.add_space(2.0);
                    let mut rows = tab.item.request.path_variables.clone();
                    if widgets::kv_table(ui, &format!("pathvars{tab_id}"), &mut rows, &mut tab.path_vars, vars, true) {
                        // Keys come from the URL; only values and descriptions are editable here.
                        let names = crate::model::url::path_variable_names(&tab.item.request.url);
                        for (i, name) in names.iter().enumerate() {
                            if let (Some(target), Some(src)) = (tab.item.request.path_variables.get_mut(i), rows.get(i))
                            {
                                target.key = name.clone();
                                target.value = src.value.clone();
                                target.description = src.description.clone();
                            }
                        }
                    }
                }
            }
            ReqSection::Auth => {
                let inherited = if tab.item.request.auth.kind == AuthType::Inherit {
                    let coll = tab.source.as_ref().and_then(|(c, _)| app.collection(c));
                    let (a, owner) = exec::effective_auth(&tab.item, coll);
                    Some((a.kind.label().to_string(), owner))
                } else {
                    None
                };
                let (_, action) = widgets::auth_editor(
                    ui,
                    &format!("auth{tab_id}"),
                    &mut tab.item.request.auth,
                    vars,
                    true,
                    inherited,
                );
                if action == AuthAction::GetNewToken {
                    app.fetch_token(tab_id, tab.item.request.auth.clone(), tab.source.as_ref().map(|(c, _)| c.clone()));
                }
            }
            ReqSection::Headers => {
                let autos = crate::http::auto_headers(&tab.item.request, &app.settings);
                let caret = if tab.show_auto_headers { icon::CARET_DOWN } else { icon::CARET_RIGHT };
                let label = format!("{caret}  Auto-generated headers ({})", autos.len());
                if ui
                    .add(egui::Label::new(RichText::new(label).color(p.text_muted)).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    tab.show_auto_headers = !tab.show_auto_headers;
                }
                if tab.show_auto_headers {
                    ui.add_space(4.0);
                    kit::card(&p).fill(p.surface_hover).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut disabled = tab
                            .item
                            .request
                            .settings
                            .get("disabledSystemHeaders")
                            .and_then(|v| v.as_object())
                            .cloned()
                            .unwrap_or_default();
                        let mut changed = false;
                        for name in
                            ["User-Agent", "Accept", "Cache-Control", "Accept-Encoding", "Connection", "Content-Type"]
                        {
                            let lower = name.to_lowercase();
                            let value = autos.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
                            let is_off = disabled.get(&lower).and_then(|v| v.as_bool()).unwrap_or(false);
                            if value.is_none() && !is_off {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                let mut on = !is_off;
                                if kit::checkbox(ui, &mut on).changed() {
                                    if on {
                                        disabled.remove(&lower);
                                    } else {
                                        disabled.insert(lower.clone(), true.into());
                                    }
                                    changed = true;
                                }
                                let (r, _) = ui.allocate_exact_size(egui::vec2(150.0, 20.0), egui::Sense::hover());
                                ui.painter().text(
                                    r.left_center(),
                                    egui::Align2::LEFT_CENTER,
                                    name,
                                    egui::FontId::monospace(12.0),
                                    p.text_muted,
                                );
                                ui.label(
                                    RichText::new(value.unwrap_or_else(|| "disabled".into()))
                                        .monospace()
                                        .size(12.0)
                                        .color(if on { p.text } else { p.text_faint }),
                                );
                            });
                        }
                        if changed {
                            if disabled.is_empty() {
                                tab.item.request.settings.remove("disabledSystemHeaders");
                            } else {
                                tab.item
                                    .request
                                    .settings
                                    .insert("disabledSystemHeaders".into(), serde_json::Value::Object(disabled));
                            }
                        }
                        ui.label(
                            RichText::new("Authorization headers are added when the request is sent.")
                                .color(p.text_faint)
                                .size(12.0),
                        );
                    });
                }
                ui.add_space(10.0);
                widgets::kv_table(
                    ui,
                    &format!("headers{tab_id}"),
                    &mut tab.item.request.headers,
                    &mut tab.headers,
                    vars,
                    true,
                );
            }
            ReqSection::Body => body_editor(app, ui, tab_id, tab, vars, font),
            ReqSection::Scripts => {}
            ReqSection::Settings => request_settings(ui, &mut tab.item.request),
        }
    });
}

fn scripts_editor(ui: &mut Ui, tab_id: u64, tab: &mut RequestTab, font: f32) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        kit::segmented(ui, &mut tab.post_script, &[(false, "Pre-request"), (true, "Post-response")]);
        ui.add_space(8.0);
        ui.label(
            RichText::new(if tab.post_script {
                "Runs after the response arrives · pm.test(), pm.expect()"
            } else {
                "Runs before sending · collection and folder scripts run first"
            })
            .color(p.text_faint)
            .size(12.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut insert = None;
            kit::dropdown(
                ui,
                &format!("snips{tab_id}"),
                Some((icon::LIGHTNING, p.accent)),
                "Snippets",
                None,
                140.0,
                |ui| {
                    ui.set_min_width(280.0);
                    for (label, code) in snippets(tab.post_script) {
                        if kit::menu_item(ui, false, label).clicked() {
                            insert = Some(code);
                        }
                    }
                },
            );
            if let Some(code) = insert {
                let target = if tab.post_script {
                    &mut tab.item.scripts.post_response
                } else {
                    &mut tab.item.scripts.pre_request
                };
                if !target.is_empty() && !target.ends_with('\n') {
                    target.push('\n');
                }
                target.push_str(code);
                target.push('\n');
            }
        });
    });
    ui.add_space(8.0);
    let target = if tab.post_script { &mut tab.item.scripts.post_response } else { &mut tab.item.scripts.pre_request };
    widgets::code_editor(ui, &format!("script{tab_id}{}", tab.post_script), target, Lang::Js, None, 10, font);
}

fn body_editor(
    app: &mut CourierApp,
    ui: &mut Ui,
    tab_id: u64,
    tab: &mut RequestTab,
    vars: &crate::vars::VarContext,
    font: f32,
) {
    let p = theme::palette(ui);
    let body = &mut tab.item.request.body;
    ui.horizontal(|ui| {
        kit::segmented(
            ui,
            &mut body.mode,
            &[
                (BodyMode::None, "None"),
                (BodyMode::Formdata, "Form data"),
                (BodyMode::Urlencoded, "URL encoded"),
                (BodyMode::Raw, "Raw"),
                (BodyMode::File, "Binary"),
                (BodyMode::Graphql, "GraphQL"),
            ],
        );
        if body.mode == BodyMode::Raw {
            ui.add_space(6.0);
            let mut picked = None;
            kit::dropdown(
                ui,
                &format!("rawlang{tab_id}"),
                Some((icon::BRACKETS_CURLY, p.text_muted)),
                body.raw_language.label(),
                None,
                130.0,
                |ui| {
                    for l in RawLanguage::ALL {
                        if kit::menu_item(ui, body.raw_language == l, l.label()).clicked() {
                            picked = Some(l);
                        }
                    }
                },
            );
            if let Some(l) = picked {
                body.raw_language = l;
            }
            if body.raw_language == RawLanguage::Json {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::small_button(ui, kit::Kind::Ghost, Some(icon::SPARKLE), "Beautify").clicked() {
                        let stripped = crate::http::prepare::strip_json_comments(&body.raw);
                        match serde_json::from_str::<serde_json::Value>(&stripped) {
                            Ok(v) => body.raw = serde_json::to_string_pretty(&v).unwrap_or_default(),
                            Err(e) => app.toast_error(format!("Invalid JSON: {e}")),
                        }
                    }
                });
            }
        }
    });
    ui.add_space(10.0);
    match body.mode {
        BodyMode::None => {
            ui.label(RichText::new("This request does not have a body.").color(p.text_faint));
        }
        BodyMode::Raw => {
            let lang = match body.raw_language {
                RawLanguage::Json => Lang::Json,
                RawLanguage::Javascript => Lang::Js,
                RawLanguage::Html | RawLanguage::Xml => Lang::Xml,
                RawLanguage::Text => Lang::Plain,
            };
            if body.raw_language == RawLanguage::Json && !body.raw.trim().is_empty() {
                let stripped = crate::http::prepare::strip_json_comments(&vars.resolve(&body.raw));
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&stripped) {
                    ui.label(RichText::new(format!("{}  {e}", icon::WARNING)).color(p.warning).size(12.0));
                    ui.add_space(4.0);
                }
            }
            widgets::code_editor(ui, &format!("raw{tab_id}"), &mut body.raw, lang, Some(vars), 10, font);
        }
        BodyMode::Urlencoded => {
            widgets::kv_table(ui, &format!("urlenc{tab_id}"), &mut body.urlencoded, &mut tab.urlencoded, vars, true);
        }
        BodyMode::Formdata => {
            widgets::form_table(ui, &format!("form{tab_id}"), &mut body.formdata, vars);
        }
        BodyMode::File => {
            kit::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::FILE_ARROW_UP).size(22.0).color(p.text_muted));
                    ui.vertical(|ui| {
                        let label = body.file.clone().unwrap_or_else(|| "No file selected".into());
                        ui.label(RichText::new(label).monospace());
                        ui.label(
                            RichText::new("The file is sent as the raw request body.").color(p.text_faint).size(12.0),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if body.file.is_some() && kit::icon_button(ui, icon::X, "Clear").clicked() {
                            body.file = None;
                        }
                        if kit::button(ui, kit::Kind::Secondary, None, "Choose file…").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_file()
                        {
                            body.file = Some(path.display().to_string());
                        }
                    });
                });
            });
        }
        BodyMode::Graphql => {
            ui.columns(2, |cols| {
                kit::section_label(&mut cols[0], "Query");
                widgets::code_editor(
                    &mut cols[0],
                    &format!("gql{tab_id}"),
                    &mut body.graphql.query,
                    Lang::Js,
                    Some(vars),
                    10,
                    font,
                );
                kit::section_label(&mut cols[1], "Variables");
                widgets::code_editor(
                    &mut cols[1],
                    &format!("gqlv{tab_id}"),
                    &mut body.graphql.variables,
                    Lang::Json,
                    Some(vars),
                    10,
                    font,
                );
            });
        }
    }
}

fn request_settings(ui: &mut Ui, req: &mut Request) {
    let mut toggle = |ui: &mut Ui, key: &str, title: &str, help: &str, default: bool| {
        let mut v = req.setting_bool(key).unwrap_or(default);
        if kit::setting_switch(ui, title, help, &mut v) {
            if v == default {
                req.settings.remove(key);
            } else {
                req.settings.insert(key.into(), v.into());
            }
        }
    };
    toggle(ui, "strictSSL", "SSL certificate verification", "Verify TLS certificates for this request.", true);
    toggle(ui, "followRedirects", "Automatically follow redirects", "Follow 3xx responses to their Location.", true);
    toggle(
        ui,
        "followOriginalHttpMethod",
        "Follow original HTTP method",
        "Keep the method on 301/302 instead of switching to GET.",
        false,
    );
    toggle(
        ui,
        "followAuthorizationHeader",
        "Keep Authorization on cross-origin redirects",
        "Forward the Authorization header to other hosts.",
        false,
    );
    toggle(
        ui,
        "removeRefererHeaderOnRedirect",
        "Remove Referer on redirect",
        "Do not send a Referer header after a redirect.",
        false,
    );
    toggle(ui, "disableCookies", "Disable cookie jar", "Do not send or store cookies for this request.", false);
    toggle(
        ui,
        "disableBodyPruning",
        "Send body with GET and HEAD",
        "Keep the body for methods that usually have none.",
        false,
    );
    ui.add_space(6.0);
    widgets::form_row(ui, "Maximum redirects", |ui| {
        let mut n = req.setting_u64("maxRedirects").unwrap_or(10);
        if ui.add(egui::DragValue::new(&mut n).range(0..=100)).changed() {
            if n == 10 {
                req.settings.remove("maxRedirects");
            } else {
                req.settings.insert("maxRedirects".into(), n.into());
            }
        }
    });
    widgets::form_row(ui, "Timeout (ms, 0 = default)", |ui| {
        let mut t = req.setting_u64("timeout").unwrap_or(0);
        if ui.add(egui::DragValue::new(&mut t).range(0..=3_600_000).speed(100)).changed() {
            if t == 0 {
                req.settings.remove("timeout");
            } else {
                req.settings.insert("timeout".into(), t.into());
            }
        }
    });
    widgets::form_row(ui, "HTTP version", |ui| {
        let current = match req.settings.get("protocolVersion").and_then(|v| v.as_str()) {
            Some("http1") => "http1",
            Some("http2") => "http2",
            _ => "auto",
        };
        let mut v: &'static str = current;
        if kit::segmented(ui, &mut v, &[("auto", "Auto"), ("http1", "HTTP/1.x"), ("http2", "HTTP/2")]) {
            if v == "auto" {
                req.settings.remove("protocolVersion");
            } else {
                req.settings.insert("protocolVersion".into(), v.into());
            }
        }
    });
}

fn snippets(post: bool) -> Vec<(&'static str, &'static str)> {
    if post {
        vec![
            ("Status code is 200", "pm.test(\"Status code is 200\", () => {\n  pm.response.to.have.status(200);\n});"),
            (
                "Response time is below 200ms",
                "pm.test(\"Response time is below 200ms\", () => {\n  pm.expect(pm.response.responseTime).to.be.below(200);\n});",
            ),
            (
                "Body contains string",
                "pm.test(\"Body contains string\", () => {\n  pm.expect(pm.response.text()).to.include(\"string_you_want_to_search\");\n});",
            ),
            (
                "JSON value check",
                "pm.test(\"JSON value check\", () => {\n  const data = pm.response.json();\n  pm.expect(data.value).to.eql(100);\n});",
            ),
            (
                "Content-Type header is present",
                "pm.test(\"Content-Type header is present\", () => {\n  pm.response.to.have.header(\"Content-Type\");\n});",
            ),
            (
                "Validate JSON schema",
                "const schema = { type: \"object\", required: [\"id\"] };\npm.test(\"Schema is valid\", () => {\n  pm.response.to.have.jsonSchema(schema);\n});",
            ),
            ("Save a value to the environment", "pm.environment.set(\"token\", pm.response.json().token);"),
            ("Set next request", "pm.execution.setNextRequest(\"Request name\");"),
        ]
    } else {
        vec![
            ("Set an environment variable", "pm.environment.set(\"variable_key\", \"variable_value\");"),
            ("Set a collection variable", "pm.collectionVariables.set(\"variable_key\", \"variable_value\");"),
            ("Get a variable", "const value = pm.variables.get(\"variable_key\");"),
            (
                "Add a header",
                "pm.request.headers.add({ key: \"X-Request-Id\", value: pm.variables.replaceIn(\"{{$guid}}\") });",
            ),
            (
                "Send a request",
                "pm.sendRequest(\"https://example.com\", (err, res) => {\n  console.log(err ? err : res.json());\n});",
            ),
            ("Skip this request", "pm.execution.skipRequest();"),
        ]
    }
}

// ---------------------------------------------------------------- response

fn response_view(app: &mut CourierApp, ui: &mut Ui, tab: &mut RequestTab) {
    let p = theme::palette(ui);
    match &tab.resp {
        RespState::Empty => {
            kit::empty_state(
                ui,
                icon::PAPER_PLANE_TILT,
                "Ready when you are",
                "Enter a URL and send the request to see the response here.",
            );
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let w = 150.0;
                    ui.add_space((ui.available_width() - w) / 2.0);
                    kit::kbd(ui, &["⌘", "Enter"]);
                    ui.label(RichText::new("to send").color(p.text_faint).size(12.0));
                });
            });
            return;
        }
        RespState::Loading { started, .. } => {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 40.0).max(10.0));
                ui.add(egui::Spinner::new().size(26.0).color(p.accent));
                ui.add_space(10.0);
                ui.label(RichText::new("Sending request…").strong());
                ui.label(
                    RichText::new(human_time(started.elapsed().as_secs_f64() * 1000.0)).color(p.text_muted).monospace(),
                );
            });
            return;
        }
        RespState::Done(_) => {}
    }
    let RespState::Done(exec) = &tab.resp else { return };
    let exec = exec.clone();
    for e in &exec.script_errors {
        widgets::note(ui, icon::WARNING, p.danger, e);
        ui.add_space(6.0);
    }
    if exec.skipped {
        kit::empty_state(
            ui,
            icon::ARROW_SQUARE_OUT,
            "Request skipped",
            "A pre-request script called pm.execution.skipRequest().",
        );
        return;
    }
    let Some(res) = &exec.response else {
        let message = exec.error.clone().unwrap_or_default();
        kit::empty_state(ui, icon::WARNING_CIRCLE, "Could not send request", &message);
        if let Some(sent) = &exec.sent {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{} {}", sent.method, sent.url)).monospace().color(p.text_faint).size(12.0),
                );
            });
        }
        return;
    };

    let tests_total = exec.tests.len();
    let tests_passed = exec.passed_tests();
    ui.horizontal(|ui| {
        let tests_badge = (tests_total > 0).then(|| format!("{tests_passed}/{tests_total}"));
        let cookies = res.header_all("set-cookie").count();
        kit::tabs(
            ui,
            &mut tab.resp_section,
            &[
                (RespSection::Body, "Body", None),
                (RespSection::Headers, "Headers", Some(res.headers.len().to_string())),
                (RespSection::Cookies, "Cookies", (cookies > 0).then(|| cookies.to_string())),
                (RespSection::Tests, "Tests", tests_badge),
                (RespSection::Request, "Request", None),
            ],
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let size = ui.label(
                RichText::new(format!("{}  {}", icon::DATABASE, human_size(res.total_size())))
                    .color(p.text_muted)
                    .size(12.5),
            );
            size.on_hover_ui(|ui| {
                ui.label(format!("Headers  {}", human_size(res.header_size)));
                ui.label(format!("Body  {}", human_size(res.body_size)));
                if res.truncated {
                    ui.label(RichText::new("Body was truncated in memory (size limit).").color(p.warning));
                }
            });
            let time = ui.label(
                RichText::new(format!("{}  {}", icon::TIMER, human_time(res.timings.total_ms)))
                    .color(p.text_muted)
                    .size(12.5),
            );
            time.on_hover_ui(|ui| {
                egui::Grid::new("timings").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                    ui.label(RichText::new("Waiting (TTFB)").color(p.text_muted));
                    ui.label(human_time(res.timings.ttfb_ms));
                    ui.end_row();
                    ui.label(RichText::new("Download").color(p.text_muted));
                    ui.label(human_time(res.timings.download_ms));
                    ui.end_row();
                    ui.label(RichText::new("Total").strong());
                    ui.label(RichText::new(human_time(res.timings.total_ms)).strong());
                    ui.end_row();
                });
            });
            let status = format!("{} {}", res.status, res.status_text);
            kit::badge(ui, &status, theme::status_color(res.status, p.dark))
                .on_hover_text(format!("{} · {}", res.http_version, res.final_url));
        });
    });
    ui.add_space(10.0);
    match tab.resp_section {
        RespSection::Body => body_view(app, ui, tab, res),
        RespSection::Headers => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                read_only_table(ui, &res.headers);
                if !res.redirects.is_empty() {
                    ui.add_space(14.0);
                    kit::section_label(ui, "Redirects");
                    for r in &res.redirects {
                        ui.label(
                            RichText::new(format!("{}  {r}", icon::ARROW_SQUARE_OUT))
                                .monospace()
                                .color(p.text_muted)
                                .size(12.0),
                        );
                    }
                }
            });
        }
        RespSection::Cookies => {
            let set: Vec<String> = res.header_all("set-cookie").map(str::to_string).collect();
            if set.is_empty() {
                kit::empty_state(ui, icon::COOKIE, "No cookies", "This response did not set any cookies.");
            } else if let Ok(url) = ::url::Url::parse(&res.final_url) {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("resp_cookies").num_columns(5).spacing([20.0, 8.0]).show(ui, |ui| {
                        for h in ["NAME", "VALUE", "DOMAIN", "PATH", "EXPIRES"] {
                            ui.label(RichText::new(h).text_style(theme::caption_style()).color(p.text_faint));
                        }
                        ui.end_row();
                        for s in &set {
                            if let Some(c) = crate::cookies::parse_set_cookie(s, &url, chrono::Utc::now()) {
                                ui.label(RichText::new(&c.name).monospace());
                                ui.label(RichText::new(&c.value).monospace().color(p.text_muted));
                                ui.label(&c.domain);
                                ui.label(&c.path);
                                ui.label(
                                    RichText::new(
                                        c.expires
                                            .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                                            .unwrap_or_else(|| "Session".into()),
                                    )
                                    .color(p.text_muted),
                                );
                                ui.end_row();
                            }
                        }
                    });
                });
            }
        }
        RespSection::Tests => {
            if exec.tests.is_empty() {
                kit::empty_state(
                    ui,
                    icon::FLASK,
                    "No tests",
                    "Write tests with pm.test() in the request's post-response script.",
                );
                return;
            }
            ui.horizontal(|ui| {
                kit::segmented(
                    ui,
                    &mut tab.test_filter,
                    &[
                        (TestFilter::All, "All"),
                        (TestFilter::Passed, "Passed"),
                        (TestFilter::Failed, "Failed"),
                        (TestFilter::Skipped, "Skipped"),
                    ],
                );
                ui.add_space(8.0);
                let failed = exec.failed_tests();
                let (text, color) = if failed == 0 {
                    (format!("{tests_passed} passed"), p.success)
                } else {
                    (format!("{failed} failed"), p.danger)
                };
                kit::badge(ui, &text, color);
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for t in &exec.tests {
                    let keep = match tab.test_filter {
                        TestFilter::All => true,
                        TestFilter::Passed => t.passed,
                        TestFilter::Failed => !t.passed && !t.skipped,
                        TestFilter::Skipped => t.skipped,
                    };
                    if !keep {
                        continue;
                    }
                    test_row(ui, t);
                }
            });
        }
        RespSection::Request => {
            if let Some(sent) = &exec.sent {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        kit::method_tag(ui, &sent.method, 52.0);
                        ui.label(RichText::new(&sent.url).monospace());
                    });
                    ui.add_space(10.0);
                    read_only_table(ui, &sent.headers);
                    if let Some(b) = sent.body.bytes().filter(|b| !b.is_empty()) {
                        ui.add_space(10.0);
                        kit::section_label(ui, "Body");
                        widgets::code_view(ui, "sent_body", &String::from_utf8_lossy(b), Lang::Plain, "", 12.5, true);
                    }
                });
            }
        }
    }
}

/// Key/value pairs in a bordered two-column table.
fn read_only_table(ui: &mut Ui, rows: &[(String, String)]) {
    let p = theme::palette(ui);
    egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, p.border)).corner_radius(8).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = ui.available_width();
        for (i, (k, v)) in rows.iter().enumerate() {
            let galley = ui.painter().layout(v.clone(), egui::FontId::monospace(12.0), p.text, width * 0.62 - 16.0);
            let h = (galley.size().y + 14.0).max(30.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
            if i > 0 {
                ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0_f32, p.border));
            }
            if ui.rect_contains_pointer(rect) {
                ui.painter().rect_filled(rect, 0.0, p.surface_hover.gamma_multiply(0.6));
            }
            let split = rect.left() + width * 0.38;
            ui.painter().text(
                egui::pos2(rect.left() + 12.0, rect.top() + 8.0),
                egui::Align2::LEFT_TOP,
                k,
                egui::FontId::monospace(12.0),
                p.text_muted,
            );
            ui.painter().galley(egui::pos2(split, rect.top() + 7.0), galley, p.text);
        }
    });
}

fn test_row(ui: &mut Ui, t: &crate::script::TestResult) {
    let p = theme::palette(ui);
    let (glyph, color, label) = if t.skipped {
        (icon::CIRCLE, p.text_faint, "SKIP")
    } else if t.passed {
        (icon::CHECK_CIRCLE, p.success, "PASS")
    } else {
        (icon::X_CIRCLE, p.danger, "FAIL")
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(8)
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).size(16.0).color(color));
                ui.label(RichText::new(label).monospace().size(11.0).color(color).strong());
                ui.label(RichText::new(&t.name).color(p.text));
            });
            if let Some(e) = &t.error {
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.label(RichText::new(e).monospace().size(12.0).color(p.text_muted));
                });
            }
        });
    ui.add_space(6.0);
}

fn body_view(app: &mut CourierApp, ui: &mut Ui, tab: &mut RequestTab, res: &crate::http::HttpResponse) {
    let p = theme::palette(ui);
    let lang = highlight::lang_for_content_type(res.content_type());
    let is_image = res.content_type().starts_with("image/");
    ui.horizontal(|ui| {
        kit::segmented(
            ui,
            &mut tab.body_view,
            &[(BodyView::Pretty, "Pretty"), (BodyView::Raw, "Raw"), (BodyView::Preview, "Preview")],
        );
        let lang_name = match lang {
            Lang::Json => "JSON",
            Lang::Xml if res.content_type().contains("html") => "HTML",
            Lang::Xml => "XML",
            Lang::Js => "JavaScript",
            Lang::Plain if is_image => "Image",
            Lang::Plain => "Text",
        };
        kit::badge(ui, lang_name, p.text_muted);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if tab.source.is_some() && kit::icon_button(ui, icon::BOOKMARK_SIMPLE, "Save as example").clicked() {
                save_example(app, tab, res);
            }
            if kit::icon_button(ui, icon::DOWNLOAD_SIMPLE, "Save response to file").clicked()
                && let Some(path) = rfd::FileDialog::new().set_file_name("response").save_file()
            {
                match crate::storage::write_atomic(&path, &res.body) {
                    Ok(()) => app.toast(format!("Saved to {}", path.display())),
                    Err(e) => app.toast_error(e.to_string()),
                }
            }
            if kit::icon_button(ui, icon::COPY, "Copy response body").clicked() {
                ui.ctx().copy_text(res.text());
                app.toast("Copied response body");
            }
            ui.add_space(6.0);
            kit::search_field(ui, &mut tab.search, "Find in response", 220.0);
        });
    });
    ui.add_space(8.0);
    if res.truncated {
        widgets::note(
            ui,
            icon::WARNING,
            p.warning,
            &format!("Showing the first {} of {}.", human_size(res.body.len() as u64), human_size(res.body_size)),
        );
        ui.add_space(6.0);
    }
    if let Some(path) = &res.downloaded_to {
        kit::empty_state(ui, icon::DOWNLOAD_SIMPLE, "Saved to disk", &path.display().to_string());
        return;
    }
    let font = app.settings.response_font_size();
    let wrap = app.settings.line_wrap;
    match tab.body_view {
        BodyView::Pretty => {
            let key = res.body.len() ^ (res.timings.total_ms as usize);
            if tab.pretty_cache.as_ref().map(|(k, _)| *k) != Some(key) {
                tab.pretty_cache = Some((key, highlight::pretty(&res.text(), lang)));
            }
            let text = tab.pretty_cache.as_ref().map(|(_, t)| t.as_str()).unwrap_or_default();
            widgets::code_view(ui, "resp_pretty", text, lang, &tab.search, font, wrap);
        }
        BodyView::Raw => widgets::code_view(ui, "resp_raw", &res.text(), Lang::Plain, &tab.search, font, true),
        BodyView::Preview => {
            kit::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(ui.available_height() - 4.0);
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    if is_image {
                        ui.add(
                            egui::Image::from_bytes(format!("bytes://response-{}", res.final_url), res.body.clone())
                                .max_width(ui.available_width()),
                        );
                    } else if lang == Lang::Xml && res.content_type().contains("html") {
                        ui.label(html_to_text(&res.text()));
                    } else {
                        ui.label(res.text());
                    }
                });
            });
        }
    }
}

fn save_example(app: &mut CourierApp, tab: &mut RequestTab, res: &crate::http::HttpResponse) {
    let Some((cid, rid)) = tab.source.clone() else { return };
    let example = Example {
        id: new_id(),
        name: format!("{} {}", res.status, res.status_text),
        original_request: Some(tab.item.request.clone()),
        status: res.status_text.clone(),
        code: res.status,
        headers: res.headers.iter().map(|(k, v)| KeyValue::new(k, v)).collect(),
        body: res.text(),
        preview_language: Some(match highlight::lang_for_content_type(res.content_type()) {
            Lang::Json => "json".into(),
            Lang::Xml => "xml".into(),
            Lang::Js => "javascript".into(),
            Lang::Plain => "text".into(),
        }),
        extra: Default::default(),
    };
    if let Some(r) = app.collection_mut(&cid).and_then(|c| c.find_request_mut(&rid)) {
        r.examples.push(example.clone());
        app.persist_collection(&cid);
        tab.item.examples.push(example.clone());
        if let Some(s) = &mut tab.saved {
            s.examples.push(example);
        }
        app.expanded.insert(rid);
        app.toast("Saved response as an example");
    }
}

/// Very small HTML-to-text conversion for the Preview tab.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut skip_depth = 0;
    let lower = html.to_lowercase();
    let mut i = 0;
    let b = html.as_bytes();
    while i < b.len() {
        if b[i] == b'<' {
            let rest = &lower[i..];
            if rest.starts_with("<script") || rest.starts_with("<style") {
                skip_depth += 1;
            } else if rest.starts_with("</script") || rest.starts_with("</style") {
                skip_depth -= 1;
            } else if rest.starts_with("<br")
                || rest.starts_with("<p")
                || rest.starts_with("</p")
                || rest.starts_with("<div")
                || rest.starts_with("<li")
                || rest.starts_with("<h")
                || rest.starts_with("<tr")
            {
                out.push('\n');
            }
            in_tag = true;
        } else if b[i] == b'>' {
            in_tag = false;
        } else if !in_tag && skip_depth <= 0 {
            let ch_len = html[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            out.push_str(&html[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        i += 1;
    }
    let text = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&");
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_preview_text() {
        let t = html_to_text(
            "<html><head><style>a{}</style></head><body><h1>Hi</h1><p>A &amp; B</p><script>x()</script></body></html>",
        );
        assert_eq!(t, "Hi\nA & B");
    }

    #[test]
    fn dirty_tracking() {
        let item = RequestItem::new("r", Request::new("GET", "http://x"));
        let mut tab = RequestTab::new(None, item.clone(), Some(item));
        assert!(!tab.is_dirty());
        tab.item.request.method = "POST".into();
        assert!(tab.is_dirty());
        let blank = RequestTab::new(None, RequestItem::new("u", Request::default()), None);
        assert!(!blank.is_dirty());
    }
}
