//! Left sidebar: collections tree, environments list and history.

use super::kit::{self, icon};
use super::{ConfirmAction, CourierApp, Dialog, SidebarSection, dialogs, theme};
use crate::model::*;
use egui::{RichText, Ui};

/// Payload carried while dragging a sidebar item.
#[derive(Clone, Debug)]
struct DragItem {
    collection_id: String,
    item_id: String,
}

enum Action {
    OpenRequest(String, String),
    OpenNode(String, Option<String>),
    OpenExample(String, String, String),
    AddRequest(String, Option<String>),
    AddFolder(String, Option<String>),
    NewCollection,
    Import,
    Rename(String, String),
    CommitRename(String, String),
    Duplicate(String, String),
    DuplicateCollection(String),
    Delete(String, String),
    DeleteCollection(String),
    Run(String, Option<String>),
    Export(String),
    CopyCurl(String, String),
    Move { from: String, item: String, to: String, parent: Option<String>, index: usize },
    Toggle(String),
}

const ROW_H: f32 = 28.0;
const INDENT: f32 = 14.0;

pub fn show(app: &mut CourierApp, ui: &mut Ui) {
    let p = theme::palette(ui);
    let title = match app.sidebar {
        SidebarSection::Collections => "Collections",
        SidebarSection::Environments => "Environments",
        SidebarSection::History => "History",
    };
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(15.0).strong().color(p.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            match app.sidebar {
                SidebarSection::Collections => {
                    if kit::icon_button(ui, icon::TRAY_ARROW_DOWN, &super::platform::keys("Import (⌘O)")).clicked() {
                        actions.push(Action::Import);
                    }
                    if kit::icon_button(ui, icon::PLUS, "New collection").clicked() {
                        actions.push(Action::NewCollection);
                    }
                }
                SidebarSection::Environments => {
                    if kit::icon_button(ui, icon::TRAY_ARROW_DOWN, "Import environment").clicked() {
                        actions.push(Action::Import);
                    }
                    if kit::icon_button(ui, icon::PLUS, "New environment").clicked() {
                        let id = app.create_environment("New Environment");
                        app.open_environment(Some(&id));
                    }
                }
                SidebarSection::History => {
                    if kit::icon_button(ui, icon::TRASH, "Clear history").clicked() && !app.history.entries.is_empty() {
                        app.dialog = Some(Dialog::Confirm {
                            title: "Clear history".into(),
                            message: "Delete all history entries? This cannot be undone.".into(),
                            action: ConfirmAction::ClearHistory,
                        });
                    }
                }
            }
        });
    });
    ui.add_space(8.0);
    let hint = match app.sidebar {
        SidebarSection::Collections => "Search requests",
        SidebarSection::Environments => "Search environments",
        SidebarSection::History => "Search history",
    };
    let r = kit::search_field(ui, &mut app.sidebar_filter, hint, f32::INFINITY);
    if std::mem::take(&mut app.focus_sidebar_search) {
        r.request_focus();
    }
    ui.add_space(10.0);
    match app.sidebar {
        SidebarSection::Collections => collections(app, ui, &mut actions),
        SidebarSection::Environments => environments(app, ui),
        SidebarSection::History => history(app, ui),
    }
    apply(app, actions);
}

fn matches_filter(item: &Item, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    match item {
        Item::Request(r) => r.name.to_lowercase().contains(filter) || r.request.url.to_lowercase().contains(filter),
        Item::Folder(f) => f.name.to_lowercase().contains(filter) || f.items.iter().any(|i| matches_filter(i, filter)),
    }
}

struct TreeCtx<'a> {
    app: &'a CourierApp,
    filter: &'a str,
    active_request: Option<&'a str>,
    active_node: Option<(&'a str, Option<&'a str>)>,
}

fn collections(app: &mut CourierApp, ui: &mut Ui, actions: &mut Vec<Action>) {
    let filter = app.sidebar_filter.to_lowercase();
    let (active_request, active_node) = match app.tabs.get(app.active_tab).map(|t| &t.kind) {
        Some(super::TabKind::Request(r)) => (r.source.as_ref().map(|(_, r)| r.as_str()), None),
        Some(super::TabKind::Node(n)) => (None, Some((n.collection_id.as_str(), n.node_id.as_deref()))),
        _ => (None, None),
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if app.collections.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                let p = theme::palette(ui);
                ui.label(RichText::new(icon::STACK).size(30.0).color(p.text_faint));
                ui.add_space(6.0);
                ui.label(RichText::new("No collections yet").strong());
                ui.label(
                    RichText::new("Save and organise your requests,\nthen run them together.").color(p.text_muted),
                );
                ui.add_space(10.0);
                if kit::button(ui, kit::Kind::Primary, Some(icon::PLUS), "New collection").clicked() {
                    actions.push(Action::NewCollection);
                }
                ui.add_space(4.0);
                if kit::button(ui, kit::Kind::Secondary, Some(icon::TRAY_ARROW_DOWN), "Import").clicked() {
                    actions.push(Action::Import);
                }
            });
            return;
        }
        let ctx = TreeCtx { app, filter: &filter, active_request, active_node };
        for c in &app.collections {
            if !filter.is_empty()
                && !c.name.to_lowercase().contains(&filter)
                && !c.items.iter().any(|i| matches_filter(i, &filter))
            {
                continue;
            }
            collection_row(ui, &ctx, c, actions);
        }
    });
}

fn rename_field(ui: &mut Ui, ctx: &TreeCtx, id: &str, actions: &mut Vec<Action>) -> bool {
    let Some((rid, text)) = &ctx.app.renaming else { return false };
    if rid != id {
        return false;
    }
    let mut t = text.clone();
    let r = ui
        .add_sized([ui.available_width() - 4.0, 24.0], egui::TextEdit::singleline(&mut t).margin(egui::vec2(6.0, 3.0)));
    r.request_focus();
    if r.changed() {
        actions.push(Action::Rename(id.to_string(), t.clone()));
    }
    if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        actions.push(Action::CommitRename(id.to_string(), t));
    }
    true
}

fn caret(ui: &mut Ui, open: bool, visible: bool) -> egui::Response {
    let p = theme::palette(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::click());
    if visible {
        let glyph = if open { icon::CARET_DOWN } else { icon::CARET_RIGHT };
        let color = if resp.hovered() { p.text } else { p.text_faint };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(11.0), color);
    }
    resp
}

fn more_button(ui: &mut Ui, row_hovered: bool, add: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        let r = ui.add_visible(row_hovered, egui::Button::new(RichText::new(icon::DOTS_THREE).size(15.0)).frame(false));
        egui::containers::popup::popup_below_widget(
            ui,
            ui.make_persistent_id(("more", r.id)),
            &r,
            egui::PopupCloseBehavior::CloseOnClick,
            add,
        );
        if r.clicked() {
            ui.memory_mut(|m| m.toggle_popup(ui.make_persistent_id(("more", r.id))));
        }
    });
}

fn collection_menu(ui: &mut Ui, c: &Collection, actions: &mut Vec<Action>) {
    ui.set_min_width(190.0);
    let id = c.id.clone();
    let entries: Vec<(&str, &str, Action)> = vec![
        (icon::SLIDERS_HORIZONTAL, "Settings", Action::OpenNode(id.clone(), None)),
        (icon::PLAY, "Run collection", Action::Run(id.clone(), None)),
        (icon::FILE_PLUS, "Add request", Action::AddRequest(id.clone(), None)),
        (icon::FOLDER_PLUS, "Add folder", Action::AddFolder(id.clone(), None)),
        (icon::PENCIL_SIMPLE, "Rename", Action::Rename(id.clone(), c.name.clone())),
        (icon::COPY, "Duplicate", Action::DuplicateCollection(id.clone())),
        (icon::EXPORT, "Export…", Action::Export(id.clone())),
        (icon::TRASH, "Delete", Action::DeleteCollection(id)),
    ];
    menu(ui, entries, actions);
}

fn folder_menu(ui: &mut Ui, c: &Collection, f: &Folder, actions: &mut Vec<Action>) {
    ui.set_min_width(190.0);
    let (cid, fid) = (c.id.clone(), f.id.clone());
    let entries: Vec<(&str, &str, Action)> = vec![
        (icon::SLIDERS_HORIZONTAL, "Settings", Action::OpenNode(cid.clone(), Some(fid.clone()))),
        (icon::PLAY, "Run folder", Action::Run(cid.clone(), Some(fid.clone()))),
        (icon::FILE_PLUS, "Add request", Action::AddRequest(cid.clone(), Some(fid.clone()))),
        (icon::FOLDER_PLUS, "Add folder", Action::AddFolder(cid.clone(), Some(fid.clone()))),
        (icon::PENCIL_SIMPLE, "Rename", Action::Rename(fid.clone(), f.name.clone())),
        (icon::COPY, "Duplicate", Action::Duplicate(cid.clone(), fid.clone())),
        (icon::TRASH, "Delete", Action::Delete(cid, fid)),
    ];
    menu(ui, entries, actions);
}

fn request_menu(ui: &mut Ui, c: &Collection, r: &RequestItem, actions: &mut Vec<Action>) {
    ui.set_min_width(190.0);
    let (cid, rid) = (c.id.clone(), r.id.clone());
    let entries: Vec<(&str, &str, Action)> = vec![
        (icon::ARROW_SQUARE_OUT, "Open", Action::OpenRequest(cid.clone(), rid.clone())),
        (icon::CODE, "Copy as cURL", Action::CopyCurl(cid.clone(), rid.clone())),
        (icon::PENCIL_SIMPLE, "Rename", Action::Rename(rid.clone(), r.name.clone())),
        (icon::COPY, "Duplicate", Action::Duplicate(cid.clone(), rid.clone())),
        (icon::TRASH, "Delete", Action::Delete(cid, rid)),
    ];
    menu(ui, entries, actions);
}

fn menu(ui: &mut Ui, entries: Vec<(&str, &str, Action)>, actions: &mut Vec<Action>) {
    let p = theme::palette(ui);
    for (glyph, label, action) in entries {
        let danger = label == "Delete";
        if danger {
            ui.separator();
        }
        let color = if danger { p.danger } else { p.text };
        let text = RichText::new(format!("{glyph}   {label}")).color(color);
        if ui.add(egui::Button::new(text).frame(false).min_size(egui::vec2(ui.available_width(), 26.0))).clicked() {
            actions.push(action);
            ui.close_menu();
        }
    }
}

fn collection_row(ui: &mut Ui, ctx: &TreeCtx, c: &Collection, actions: &mut Vec<Action>) {
    let p = theme::palette(ui);
    let open = ctx.app.expanded.contains(&c.id) || !ctx.filter.is_empty();
    let selected = ctx.active_node == Some((c.id.as_str(), None));
    let (resp, _) = kit::row(ui, egui::Id::new(("coll", &c.id)), selected, ROW_H, |ui| {
        if caret(ui, open, true).clicked() {
            actions.push(Action::Toggle(c.id.clone()));
        }
        ui.label(RichText::new(icon::STACK).size(15.0).color(if open { p.accent } else { p.text_muted }));
        if !rename_field(ui, ctx, &c.id, actions) {
            let hovered = ui.rect_contains_pointer(ui.max_rect());
            kit::label_with_trailing(ui, RichText::new(&c.name).strong().color(p.text), 26.0, |ui| {
                more_button(ui, hovered, |ui| collection_menu(ui, c, actions))
            });
        }
    });
    if resp.clicked() {
        actions.push(Action::Toggle(c.id.clone()));
    }
    if resp.double_clicked() {
        actions.push(Action::OpenNode(c.id.clone(), None));
    }
    resp.context_menu(|ui| collection_menu(ui, c, actions));
    if let Some(pl) = resp.dnd_release_payload::<DragItem>() {
        actions.push(Action::Move {
            from: pl.collection_id.clone(),
            item: pl.item_id.clone(),
            to: c.id.clone(),
            parent: None,
            index: usize::MAX,
        });
    }
    if resp.dnd_hover_payload::<DragItem>().is_some() {
        ui.painter().rect_stroke(resp.rect, 6.0, egui::Stroke::new(1.5_f32, p.accent), egui::StrokeKind::Inside);
    }
    if open {
        if c.items.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                if kit::small_button(ui, kit::Kind::Ghost, Some(icon::PLUS), "Add a request").clicked() {
                    actions.push(Action::AddRequest(c.id.clone(), None));
                }
            });
        }
        for (i, item) in c.items.iter().enumerate() {
            item_row(ui, ctx, c, None, i, item, 1, actions);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn item_row(
    ui: &mut Ui,
    ctx: &TreeCtx,
    c: &Collection,
    parent: Option<&str>,
    index: usize,
    item: &Item,
    depth: usize,
    actions: &mut Vec<Action>,
) {
    if !matches_filter(item, ctx.filter) {
        return;
    }
    let p = theme::palette(ui);
    let payload = DragItem { collection_id: c.id.clone(), item_id: item.id().to_string() };
    let indent = depth as f32 * INDENT;
    match item {
        Item::Folder(f) => {
            let open = ctx.app.expanded.contains(&f.id) || !ctx.filter.is_empty();
            let selected = ctx.active_node == Some((c.id.as_str(), Some(f.id.as_str())));
            let (resp, _) = kit::row(ui, egui::Id::new(("folder", &f.id)), selected, ROW_H, |ui| {
                ui.add_space(indent);
                if caret(ui, open, true).clicked() {
                    actions.push(Action::Toggle(f.id.clone()));
                }
                let glyph = if open { icon::FOLDER_OPEN } else { icon::FOLDER };
                ui.label(RichText::new(glyph).size(15.0).color(p.text_muted));
                if !rename_field(ui, ctx, &f.id, actions) {
                    let hovered = ui.rect_contains_pointer(ui.max_rect());
                    kit::label_with_trailing(ui, RichText::new(&f.name).color(p.text), 26.0, |ui| {
                        more_button(ui, hovered, |ui| folder_menu(ui, c, f, actions))
                    });
                }
            });
            resp.dnd_set_drag_payload(payload);
            if resp.clicked() {
                actions.push(Action::Toggle(f.id.clone()));
            }
            if resp.double_clicked() {
                actions.push(Action::OpenNode(c.id.clone(), Some(f.id.clone())));
            }
            resp.context_menu(|ui| folder_menu(ui, c, f, actions));
            drop_target(ui, &resp, c, parent, index, Some(&f.id), actions);
            if open {
                if f.items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(indent + 36.0);
                        ui.label(RichText::new("Empty folder").color(p.text_faint).size(12.0));
                    });
                }
                for (i, child) in f.items.iter().enumerate() {
                    item_row(ui, ctx, c, Some(&f.id), i, child, depth + 1, actions);
                }
            }
        }
        Item::Request(r) => {
            let selected = ctx.active_request == Some(r.id.as_str());
            let has_examples = !r.examples.is_empty();
            let open = ctx.app.expanded.contains(&r.id);
            let (resp, _) = kit::row(ui, egui::Id::new(("req", &r.id)), selected, ROW_H, |ui| {
                ui.add_space(indent);
                if caret(ui, open, has_examples).clicked() && has_examples {
                    actions.push(Action::Toggle(r.id.clone()));
                }
                kit::method_tag(ui, &r.request.method, 34.0);
                if !rename_field(ui, ctx, &r.id, actions) {
                    let color = if selected { p.accent } else { p.text };
                    let hovered = ui.rect_contains_pointer(ui.max_rect());
                    kit::label_with_trailing(ui, RichText::new(&r.name).color(color), 26.0, |ui| {
                        more_button(ui, hovered, |ui| request_menu(ui, c, r, actions))
                    });
                }
            });
            resp.dnd_set_drag_payload(payload);
            let resp = resp.on_hover_text(&r.request.url);
            if resp.clicked() {
                actions.push(Action::OpenRequest(c.id.clone(), r.id.clone()));
            }
            resp.context_menu(|ui| request_menu(ui, c, r, actions));
            drop_target(ui, &resp, c, parent, index, None, actions);
            if has_examples && open {
                for e in &r.examples {
                    let (er, _) = kit::row(ui, egui::Id::new(("ex", &e.id)), false, 26.0, |ui| {
                        ui.add_space(indent + 34.0);
                        ui.label(RichText::new(icon::BOOKMARK_SIMPLE).color(p.text_faint));
                        ui.add(
                            egui::Label::new(RichText::new(&e.name).color(p.text_muted).size(12.5))
                                .truncate()
                                .selectable(false),
                        );
                    });
                    if er.clicked() {
                        actions.push(Action::OpenExample(c.id.clone(), r.id.clone(), e.id.clone()));
                    }
                }
            }
        }
    }
}

/// Handle drops on a row: onto a folder = append inside, otherwise insert
/// before the row (or after it, in its lower half).
fn drop_target(
    ui: &Ui,
    resp: &egui::Response,
    c: &Collection,
    parent: Option<&str>,
    index: usize,
    folder: Option<&str>,
    actions: &mut Vec<Action>,
) {
    let pointer = ui.ctx().pointer_interact_pos();
    let rect = resp.rect;
    let into =
        |y: f32| folder.is_some() && y > rect.top() + rect.height() * 0.25 && y < rect.bottom() - rect.height() * 0.25;
    if resp.dnd_hover_payload::<DragItem>().is_some() {
        let stroke = egui::Stroke::new(2.0_f32, theme::accent(ui.visuals().dark_mode));
        match pointer {
            Some(p) if into(p.y) => {
                ui.painter().rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);
            }
            Some(p) if p.y > rect.center().y => {
                ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
            }
            _ => {
                ui.painter().hline(rect.x_range(), rect.top(), stroke);
            }
        }
    }
    if let Some(pl) = resp.dnd_release_payload::<DragItem>() {
        let y = pointer.map(|p| p.y).unwrap_or(rect.center().y);
        let (parent, index) = if into(y) {
            (folder.map(str::to_string), usize::MAX)
        } else if y > rect.center().y {
            (parent.map(str::to_string), index + 1)
        } else {
            (parent.map(str::to_string), index)
        };
        actions.push(Action::Move {
            from: pl.collection_id.clone(),
            item: pl.item_id.clone(),
            to: c.id.clone(),
            parent,
            index,
        });
    }
}

fn apply(app: &mut CourierApp, actions: Vec<Action>) {
    for a in actions {
        match a {
            Action::OpenRequest(c, r) => app.open_request(&c, &r),
            Action::OpenNode(c, n) => app.open_node(&c, n.as_deref()),
            Action::OpenExample(c, r, e) => app.open_example(&c, &r, &e),
            Action::AddRequest(c, parent) => {
                let item = RequestItem::new("New Request", Request::default());
                app.add_item(&c, parent.as_deref(), Item::Request(item));
            }
            Action::AddFolder(c, parent) => {
                app.add_item(&c, parent.as_deref(), Item::Folder(Folder::new("New Folder")))
            }
            Action::NewCollection => {
                let new = app.create_collection("New Collection");
                app.renaming = Some((new, "New Collection".into()));
            }
            Action::Import => app.dialog = Some(Dialog::Import(dialogs::ImportDialog::default())),
            Action::Rename(id, text) => app.renaming = Some((id, text)),
            Action::CommitRename(id, text) => {
                app.renaming = None;
                app.rename(&id, text);
            }
            Action::Duplicate(c, id) => app.duplicate_item(&c, &id),
            Action::DuplicateCollection(c) => app.duplicate_collection(&c),
            Action::Delete(c, id) => {
                let name =
                    app.collection(&c).and_then(|col| col.find(&id)).map(|i| i.name().to_string()).unwrap_or_default();
                app.dialog = Some(Dialog::Confirm {
                    title: "Delete item".into(),
                    message: format!("Delete “{name}”? You can undo this for a few seconds."),
                    action: ConfirmAction::DeleteItem { collection_id: c, item_id: id },
                });
            }
            Action::DeleteCollection(c) => {
                let name = app.collection(&c).map(|col| col.name.clone()).unwrap_or_default();
                app.dialog = Some(Dialog::Confirm {
                    title: "Delete collection".into(),
                    message: format!("Delete the collection “{name}” and everything in it?"),
                    action: ConfirmAction::DeleteCollection(c),
                });
            }
            Action::Run(c, folder) => app.open_runner(&c, folder.as_deref()),
            Action::Export(id) => app.export_collection_to_file(&id),
            Action::CopyCurl(c, r) => {
                if let Some(col) = app.collection(&c)
                    && let Some(item) = col.find_request(&r)
                {
                    let (auth, _) = crate::exec::effective_auth(item, Some(col));
                    let vars = app.var_ctx(Some(&c));
                    let p = crate::formats::snippet::snippet_request(&item.request, &auth, &vars, &app.settings, true);
                    app.egui_ctx.copy_text(crate::formats::snippet::to_curl(&p));
                    app.toast("Copied cURL command");
                }
            }
            Action::Move { from, item, to, parent, index } => {
                app.move_item(&from, &item, &to, parent.as_deref(), index)
            }
            Action::Toggle(id) => {
                if !app.expanded.remove(&id) {
                    app.expanded.insert(id);
                }
            }
        }
    }
}

fn environments(app: &mut CourierApp, ui: &mut Ui) {
    let p = theme::palette(ui);
    let filter = app.sidebar_filter.to_lowercase();
    let mut open = None;
    let mut set_active = None;
    let mut duplicate = None;
    let mut delete = None;
    let mut export = None;
    let mut rename = None;
    let mut commit = None;
    let open_env = match app.tabs.get(app.active_tab).map(|t| &t.kind) {
        Some(super::TabKind::Environment(e)) => Some(e.env_id.clone()),
        _ => None,
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let (g, _) = kit::row(ui, egui::Id::new("globals_row"), open_env == Some(None), ROW_H, |ui| {
            ui.label(RichText::new(icon::GLOBE).size(15.0).color(p.text_muted));
            ui.label(RichText::new("Globals").color(p.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(app.globals.variables.len().to_string()).color(p.text_faint).size(12.0));
            });
        });
        if g.clicked() {
            open = Some(None);
        }
        g.context_menu(|ui| {
            if ui.button(format!("{}   Export…", icon::EXPORT)).clicked() {
                export = Some(None);
                ui.close_menu();
            }
        });
        ui.add_space(8.0);
        kit::section_label(ui, "Environments");
        ui.add_space(4.0);
        if app.environments.is_empty() {
            ui.label(
                RichText::new("Environments let you switch between sets of variables, such as dev and production.")
                    .color(p.text_muted),
            );
            ui.add_space(8.0);
            if kit::button(ui, kit::Kind::Secondary, Some(icon::PLUS), "New environment").clicked() {
                let id = app.create_environment("New Environment");
                open = Some(Some(id));
            }
        }
        for e in &app.environments {
            if !filter.is_empty() && !e.name.to_lowercase().contains(&filter) {
                continue;
            }
            let is_active = app.active_env.as_deref() == Some(e.id.as_str());
            let selected = open_env.as_ref() == Some(&Some(e.id.clone()));
            let (r, _) = kit::row(ui, egui::Id::new(("env", &e.id)), selected, ROW_H, |ui| {
                let dot = if is_active {
                    RichText::new(icon::CHECK_CIRCLE).color(p.accent)
                } else {
                    RichText::new(icon::CIRCLE).color(p.text_faint)
                };
                if ui
                    .add(egui::Label::new(dot.size(15.0)).sense(egui::Sense::click()))
                    .on_hover_text(if is_active { "Active — click to deactivate" } else { "Set active" })
                    .clicked()
                {
                    set_active = Some(if is_active { None } else { Some(e.id.clone()) });
                }
                if let Some((rid, text)) = &app.renaming
                    && rid == &e.id
                {
                    let mut t = text.clone();
                    let r = ui.add_sized([ui.available_width() - 4.0, 24.0], egui::TextEdit::singleline(&mut t));
                    r.request_focus();
                    if r.changed() {
                        rename = Some((e.id.clone(), t.clone()));
                    }
                    if r.lost_focus() {
                        commit = Some((e.id.clone(), t));
                    }
                    return;
                }
                kit::label_with_trailing(ui, RichText::new(&e.name).color(p.text), 64.0, |ui| {
                    if is_active {
                        kit::badge(ui, "Active", p.accent);
                    } else {
                        ui.label(RichText::new(e.variables.len().to_string()).color(p.text_faint).size(12.0));
                    }
                });
            });
            if r.clicked() {
                open = Some(Some(e.id.clone()));
            }
            r.context_menu(|ui| {
                ui.set_min_width(180.0);
                if ui
                    .button(format!("{}   {}", icon::CHECK_CIRCLE, if is_active { "Deactivate" } else { "Set active" }))
                    .clicked()
                {
                    set_active = Some(if is_active { None } else { Some(e.id.clone()) });
                    ui.close_menu();
                }
                if ui.button(format!("{}   Rename", icon::PENCIL_SIMPLE)).clicked() {
                    rename = Some((e.id.clone(), e.name.clone()));
                    ui.close_menu();
                }
                if ui.button(format!("{}   Duplicate", icon::COPY)).clicked() {
                    duplicate = Some(e.id.clone());
                    ui.close_menu();
                }
                if ui.button(format!("{}   Export…", icon::EXPORT)).clicked() {
                    export = Some(Some(e.id.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(RichText::new(format!("{}   Delete", icon::TRASH)).color(p.danger)).clicked() {
                    delete = Some(e.id.clone());
                    ui.close_menu();
                }
            });
        }
    });
    if let Some(o) = open {
        app.open_environment(o.as_deref());
    }
    if let Some(a) = set_active {
        app.active_env = a;
    }
    if let Some(id) = duplicate {
        app.duplicate_environment(&id);
    }
    if let Some(id) = export {
        app.export_environment_to_file(id.as_deref());
    }
    if let Some(r) = rename {
        app.renaming = Some(r);
    }
    if let Some((id, t)) = commit {
        app.renaming = None;
        app.rename(&id, t);
    }
    if let Some(id) = delete {
        let name = app.environment(&id).map(|e| e.name.clone()).unwrap_or_default();
        app.dialog = Some(Dialog::Confirm {
            title: "Delete environment".into(),
            message: format!("Delete the environment “{name}”?"),
            action: ConfirmAction::DeleteEnvironment(id),
        });
    }
}

fn history(app: &mut CourierApp, ui: &mut Ui) {
    let p = theme::palette(ui);
    let mut open = None;
    let mut delete = None;
    let mut save = None;
    let mut copy = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let groups = app.history.grouped(&app.sidebar_filter);
        if groups.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::CLOCK_COUNTER_CLOCKWISE).size(30.0).color(p.text_faint));
                ui.add_space(6.0);
                ui.label(RichText::new("No history yet").strong());
                ui.label(RichText::new("Requests you send appear here.").color(p.text_muted));
            });
        }
        for (day, entries) in groups {
            ui.add_space(6.0);
            kit::section_label(ui, &crate::history::day_label(day));
            ui.add_space(2.0);
            for e in entries {
                let (r, _) = kit::row(ui, egui::Id::new(("hist", &e.id)), false, ROW_H, |ui| {
                    kit::method_tag(ui, &e.method, 34.0);
                    let short = e.url.split_once("://").map(|(_, r)| r).unwrap_or(&e.url);
                    kit::label_with_trailing(ui, RichText::new(short).color(p.text).size(12.5), 40.0, |ui| {
                        match (e.response.as_ref().map(|r| r.status), &e.error) {
                            (Some(s), _) => {
                                ui.label(
                                    RichText::new(s.to_string())
                                        .monospace()
                                        .size(11.0)
                                        .color(theme::status_color(s, p.dark)),
                                );
                            }
                            (None, Some(_)) => {
                                ui.label(RichText::new(icon::WARNING_CIRCLE).color(p.danger));
                            }
                            _ => {}
                        }
                    });
                });
                let r = r.on_hover_ui(|ui| {
                    ui.label(RichText::new(&e.url).monospace());
                    ui.label(
                        RichText::new(e.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                            .color(p.text_muted),
                    );
                    if let Some(err) = &e.error {
                        ui.label(RichText::new(err).color(p.danger));
                    }
                });
                if r.clicked() {
                    open = Some(e.id.clone());
                }
                r.context_menu(|ui| {
                    ui.set_min_width(200.0);
                    if ui.button(format!("{}   Open", icon::ARROW_SQUARE_OUT)).clicked() {
                        open = Some(e.id.clone());
                        ui.close_menu();
                    }
                    if ui.button(format!("{}   Save to collection…", icon::FLOPPY_DISK)).clicked() {
                        save = Some(e.id.clone());
                        ui.close_menu();
                    }
                    if ui.button(format!("{}   Copy as cURL", icon::CODE)).clicked() {
                        copy = Some(e.id.clone());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(RichText::new(format!("{}   Delete", icon::TRASH)).color(p.danger)).clicked() {
                        delete = Some(e.id.clone());
                        ui.close_menu();
                    }
                });
            }
        }
    });
    let entry = |app: &CourierApp, id: Option<String>| {
        id.and_then(|id| app.history.entries.iter().find(|e| e.id == id).cloned())
    };
    if let Some(e) = entry(app, open) {
        let name = if e.name.is_empty() { e.url.clone() } else { e.name.clone() };
        let item =
            RequestItem::new(if name.is_empty() { "History request".to_string() } else { name }, e.request.clone());
        let mut tab = super::RequestTab::new(None, item, None);
        if let Some(res) = e.response.clone() {
            tab.resp = super::RespState::Done(Box::new(crate::exec::Execution {
                request: e.request.clone(),
                response: Some(res),
                ..Default::default()
            }));
        }
        app.push_tab(super::TabKind::Request(Box::new(tab)));
    }
    if let Some(e) = entry(app, save) {
        let tab = super::RequestTab::new(None, RequestItem::new(e.name.clone(), e.request.clone()), None);
        let idx = app.push_tab(super::TabKind::Request(Box::new(tab)));
        app.dialog = Some(Dialog::SaveAs(dialogs::SaveAsDialog::new(app, idx)));
    }
    if let Some(e) = entry(app, copy) {
        let p = crate::formats::snippet::snippet_request(
            &e.request,
            &e.request.auth,
            &app.var_ctx(None),
            &app.settings,
            true,
        );
        app.egui_ctx.copy_text(crate::formats::snippet::to_curl(&p));
        app.toast("Copied cURL command");
    }
    if let Some(id) = delete {
        let days = app.history.delete(&id);
        app.persist_history_days(days);
    }
}
