//! Modal dialogs: import, save as, settings, cookies, code snippets,
//! confirmations and the shortcut cheat sheet.

use super::kit::{self, icon};
use super::widgets;
use super::{ConfirmAction, CourierApp, Dialog, RequestTab, TabKind, theme};
use crate::exec;
use crate::formats::{self, ImportResult};
use crate::model::*;
use crate::settings::{ClientCert, Layout, Settings, Theme};
use egui::{RichText, Ui};

/// Title row with an optional subtitle and a close button. Returns true
/// when the close button was clicked.
fn header(ui: &mut Ui, glyph: &str, title: &str, subtitle: &str) -> bool {
    let p = theme::palette(ui);
    let mut close = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 10.0, p.accent_soft);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(18.0),
            p.accent,
        );
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(16.0).strong().color(p.text));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).color(p.text_muted).size(12.5));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            close = kit::icon_button(ui, icon::X, "Close").clicked();
        });
    });
    ui.add_space(14.0);
    close
}

/// Right-aligned footer buttons.
fn footer(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(16.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui)
    });
}

// ---------------------------------------------------------------- import

#[derive(Default)]
pub struct ImportDialog {
    pub text: String,
    pub error: Option<String>,
    pub preview: Option<ImportResult>,
}

impl ImportDialog {
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.refresh();
    }

    fn refresh(&mut self) {
        if self.text.trim().is_empty() {
            self.preview = None;
            self.error = None;
            return;
        }
        match formats::import_text(&self.text) {
            Ok(r) => {
                self.preview = Some(r);
                self.error = None;
            }
            Err(e) => {
                self.preview = None;
                self.error = Some(e);
            }
        }
    }
}

fn import_ui(app: &mut CourierApp, ui: &mut Ui, d: &mut ImportDialog) -> bool {
    let p = theme::palette(ui);
    let mut close = header(
        ui,
        icon::TRAY_ARROW_DOWN,
        "Import",
        "Postman Collection v2.0/v2.1, environments, globals, Courier backups or cURL",
    );
    // Drop zone / file picker.
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 86.0), egui::Sense::click());
    let hovered = resp.hovered();
    ui.painter().rect(
        rect,
        10.0,
        if hovered { p.surface_hover } else { p.input },
        egui::Stroke::new(1.0_f32, if hovered { p.accent } else { p.border_strong }),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center() - egui::vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        icon::FILE_ARROW_UP,
        egui::FontId::proportional(22.0),
        p.accent,
    );
    ui.painter().text(
        rect.center() + egui::vec2(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        "Drop a file anywhere, or click to choose one",
        egui::FontId::proportional(13.0),
        p.text_muted,
    );
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        && let Some(path) = rfd::FileDialog::new().add_filter("Importable", &["json", "txt", "sh", "curl"]).pick_file()
    {
        match std::fs::read_to_string(&path) {
            Ok(t) => d.set_text(t),
            Err(e) => d.error = Some(e.to_string()),
        }
    }
    ui.add_space(12.0);
    kit::section_label(ui, "Or paste text");
    ui.add_space(4.0);
    let r = ui.add(
        egui::TextEdit::multiline(&mut d.text)
            .code_editor()
            .desired_rows(8)
            .desired_width(f32::INFINITY)
            .margin(egui::vec2(10.0, 8.0))
            .hint_text(
                RichText::new("curl https://api.example.com/users -H 'Accept: application/json'").color(p.text_faint),
            ),
    );
    if r.changed() {
        d.refresh();
    }
    if let Some(e) = &d.error {
        ui.add_space(8.0);
        widgets::note(ui, icon::WARNING_CIRCLE, p.danger, e);
    }
    if let Some(pr) = &d.preview {
        ui.add_space(8.0);
        kit::card(&p).fill(p.surface_hover).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for item in &pr.items {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::CHECK_CIRCLE).color(p.success));
                    ui.label(item.describe());
                });
            }
            for w in &pr.warnings {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::WARNING).color(p.warning));
                    ui.label(RichText::new(w).color(p.text_muted));
                });
            }
        });
    }
    footer(ui, |ui| {
        let ready = d.preview.is_some();
        if kit::button_ext(ui, kit::Kind::Primary, None, "Import", false, ready).clicked() {
            if let Some(pr) = d.preview.take() {
                app.apply_import(pr);
            }
            close = true;
        }
        if kit::button(ui, kit::Kind::Secondary, None, "Cancel").clicked() {
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------- save as

pub struct SaveAsDialog {
    pub tab_idx: usize,
    pub name: String,
    pub collection_id: Option<String>,
    pub folder_id: Option<String>,
    pub new_collection: String,
}

impl SaveAsDialog {
    pub fn new(app: &CourierApp, tab_idx: usize) -> Self {
        let name = match app.tabs.get(tab_idx).map(|t| &t.kind) {
            Some(TabKind::Request(r)) => r.item.name.clone(),
            _ => "New Request".into(),
        };
        Self::for_tab(app, tab_idx, &name)
    }

    pub fn for_tab(app: &CourierApp, tab_idx: usize, name: &str) -> Self {
        let source = match app.tabs.get(tab_idx).map(|t| &t.kind) {
            Some(TabKind::Request(r)) => r.source.clone(),
            _ => None,
        };
        let collection_id =
            source.as_ref().map(|(c, _)| c.clone()).or_else(|| app.collections.first().map(|c| c.id.clone()));
        let folder_id = source.as_ref().and_then(|(c, r)| {
            app.collection(c).and_then(|c| c.ancestors(r)).and_then(|a| a.last().map(|f| f.id.clone()))
        });
        Self {
            tab_idx,
            name: if name == "Untitled Request" { String::new() } else { name.to_string() },
            collection_id,
            folder_id,
            new_collection: String::new(),
        }
    }
}

fn save_as_ui(app: &mut CourierApp, ui: &mut Ui, d: &mut SaveAsDialog) -> bool {
    let p = theme::palette(ui);
    let mut close = header(ui, icon::FLOPPY_DISK, "Save request", "Choose a name and where to keep it");
    kit::section_label(ui, "Name");
    ui.add_space(4.0);
    widgets::text_input(ui, &mut d.name, "Request name", ui.available_width());
    ui.add_space(12.0);
    kit::section_label(ui, "Location");
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(p.input)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .corner_radius(8)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                if app.collections.is_empty() {
                    ui.label(RichText::new("No collections yet — create one below.").color(p.text_faint));
                }
                for c in &app.collections {
                    let selected = d.collection_id.as_deref() == Some(c.id.as_str()) && d.folder_id.is_none();
                    let (r, _) = kit::row(ui, egui::Id::new(("saveas", &c.id)), selected, 28.0, |ui| {
                        ui.label(RichText::new(icon::STACK).color(p.accent));
                        ui.label(&c.name);
                    });
                    if r.clicked() {
                        d.collection_id = Some(c.id.clone());
                        d.folder_id = None;
                    }
                    fn folders(ui: &mut Ui, items: &[Item], depth: usize, cid: &str, d: &mut SaveAsDialog) {
                        let p = theme::palette(ui);
                        for i in items {
                            if let Item::Folder(f) = i {
                                let selected = d.folder_id.as_deref() == Some(f.id.as_str());
                                let (r, _) = kit::row(ui, egui::Id::new(("saveas", &f.id)), selected, 28.0, |ui| {
                                    ui.add_space(16.0 * depth as f32);
                                    ui.label(RichText::new(icon::FOLDER).color(p.text_muted));
                                    ui.label(&f.name);
                                });
                                if r.clicked() {
                                    d.collection_id = Some(cid.to_string());
                                    d.folder_id = Some(f.id.clone());
                                }
                                folders(ui, &f.items, depth + 1, cid, d);
                            }
                        }
                    }
                    folders(ui, &c.items, 1, &c.id, d);
                }
            });
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let w = ui.available_width() - 110.0;
        widgets::text_input(ui, &mut d.new_collection, "New collection name", w);
        if kit::button_ext(
            ui,
            kit::Kind::Secondary,
            Some(icon::PLUS),
            "Create",
            false,
            !d.new_collection.trim().is_empty(),
        )
        .clicked()
        {
            let id = app.create_collection(d.new_collection.trim());
            d.collection_id = Some(id);
            d.folder_id = None;
            d.new_collection.clear();
        }
    });
    footer(ui, |ui| {
        let can_save = d.collection_id.is_some() && !d.name.trim().is_empty();
        if kit::button_ext(ui, kit::Kind::Primary, None, "Save", false, can_save).clicked() {
            if let Some(cid) = d.collection_id.clone() {
                app.save_as(d.tab_idx, &cid, d.folder_id.as_deref(), d.name.trim());
            }
            close = true;
        }
        if kit::button(ui, kit::Kind::Secondary, None, "Cancel").clicked() {
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Appearance,
    Proxy,
    Certificates,
    Data,
}

pub struct SettingsDialog {
    pub draft: Settings,
    pub section: SettingsSection,
}

impl SettingsDialog {
    pub fn new(settings: &Settings) -> Self {
        Self { draft: settings.clone(), section: SettingsSection::General }
    }
}

fn number_row<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    speed: f64,
) {
    widgets::form_row(ui, label, |ui| {
        ui.add(egui::DragValue::new(value).range(range).speed(speed));
    });
}

fn settings_ui(app: &mut CourierApp, ui: &mut Ui, d: &mut SettingsDialog) -> bool {
    let p = theme::palette(ui);
    let mut close = header(ui, icon::GEAR, "Settings", "Preferences are stored locally in settings.json");
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(150.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (s, glyph, l) in [
                (SettingsSection::General, icon::SLIDERS_HORIZONTAL, "General"),
                (SettingsSection::Appearance, icon::SUN, "Appearance"),
                (SettingsSection::Proxy, icon::GLOBE, "Proxy"),
                (SettingsSection::Certificates, icon::SHIELD_CHECK, "Certificates"),
                (SettingsSection::Data, icon::HARD_DRIVES, "Data"),
            ] {
                let (r, _) = kit::row(ui, egui::Id::new(("settings_nav", l)), d.section == s, 30.0, |ui| {
                    ui.label(RichText::new(glyph).color(if d.section == s { p.accent } else { p.text_muted }));
                    ui.label(l);
                });
                if r.clicked() {
                    d.section = s;
                }
            }
        });
        ui.add_space(12.0);
        let s = &mut d.draft;
        ui.vertical(|ui| {
            egui::ScrollArea::vertical().max_height(420.0).min_scrolled_height(380.0).auto_shrink([false, true]).show(
                ui,
                |ui| match d.section {
                    SettingsSection::General => {
                        number_row(
                            ui,
                            "Request timeout (ms, 0 = none)",
                            &mut s.request_timeout_ms,
                            0..=3_600_000,
                            100.0,
                        );
                        number_row(ui, "Max response size (MB)", &mut s.max_response_size_mb, 1..=2048, 1.0);
                        number_row(ui, "Max redirects", &mut s.max_redirects, 0..=100, 1.0);
                        number_row(ui, "Script timeout (ms)", &mut s.script_timeout_ms, 100..=600_000, 100.0);
                        number_row(ui, "History entries to keep", &mut s.history_limit, 100..=10_000, 10.0);
                        widgets::form_row(ui, "Working directory", |ui| {
                            let w = ui.available_width() - 40.0;
                            widgets::text_input(ui, &mut s.working_dir, "For relative file paths", w);
                            if kit::icon_button(ui, icon::FOLDER_OPEN, "Choose folder").clicked()
                                && let Some(f) = rfd::FileDialog::new().pick_folder()
                            {
                                s.working_dir = f.display().to_string();
                            }
                        });
                        ui.add_space(10.0);
                        kit::setting_switch(
                            ui,
                            "SSL certificate verification",
                            "Reject invalid TLS certificates.",
                            &mut s.ssl_verification,
                        );
                        kit::setting_switch(
                            ui,
                            "Follow redirects",
                            "Follow 3xx responses automatically.",
                            &mut s.follow_redirects,
                        );
                        kit::setting_switch(
                            ui,
                            "Send no-cache header",
                            "Add Cache-Control: no-cache to requests.",
                            &mut s.send_no_cache_header,
                        );
                        kit::setting_switch(
                            ui,
                            "Trim keys and values",
                            "Remove surrounding whitespace before sending.",
                            &mut s.trim_keys_and_values,
                        );
                    }
                    SettingsSection::Appearance => {
                        widgets::form_row(ui, "Theme", |ui| {
                            kit::segmented(
                                ui,
                                &mut s.theme,
                                &[(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")],
                            );
                        });
                        widgets::form_row(ui, "Layout", |ui| {
                            kit::segmented(
                                ui,
                                &mut s.layout,
                                &[(Layout::Stacked, "Stacked"), (Layout::SideBySide, "Side by side")],
                            );
                        });
                        widgets::form_row(ui, "Editor font size", |ui| {
                            ui.add(egui::Slider::new(&mut s.editor_font_size, 10.0..=22.0));
                        });
                        widgets::form_row(ui, "Interface zoom", |ui| {
                            ui.add(egui::Slider::new(&mut s.ui_zoom, 0.6..=2.0));
                        });
                        ui.add_space(10.0);
                        kit::setting_switch(
                            ui,
                            "Wrap long lines",
                            "Wrap response bodies instead of scrolling sideways.",
                            &mut s.line_wrap,
                        );
                    }
                    SettingsSection::Proxy => {
                        kit::setting_switch(
                            ui,
                            "Use proxy environment variables",
                            "Respect HTTP_PROXY, HTTPS_PROXY and NO_PROXY.",
                            &mut s.proxy.use_env_vars,
                        );
                        kit::setting_switch(
                            ui,
                            "Use a custom proxy",
                            "Route every request through the proxy below.",
                            &mut s.proxy.custom_enabled,
                        );
                        ui.add_enabled_ui(s.proxy.custom_enabled, |ui| {
                            let w = (ui.available_width() - 200.0).max(160.0);
                            widgets::form_row(ui, "Proxy URL", |ui| {
                                widgets::text_input(
                                    ui,
                                    &mut s.proxy.custom_url,
                                    "http://proxy:8080 or socks5://host:1080",
                                    w,
                                )
                            });
                            widgets::form_row(ui, "Username", |ui| {
                                widgets::text_input(ui, &mut s.proxy.username, "", w)
                            });
                            widgets::form_row(ui, "Password", |ui| {
                                ui.add_sized(
                                    [w, 30.0],
                                    egui::TextEdit::singleline(&mut s.proxy.password)
                                        .password(true)
                                        .margin(egui::vec2(8.0, 6.0)),
                                )
                            });
                            widgets::form_row(ui, "Bypass", |ui| {
                                widgets::text_input(ui, &mut s.proxy.bypass, "localhost, 127.0.0.1, .internal", w)
                            });
                        });
                    }
                    SettingsSection::Certificates => {
                        kit::setting_switch(
                            ui,
                            "Custom CA certificate",
                            "Trust an additional certificate authority (PEM).",
                            &mut s.ca_cert_enabled,
                        );
                        ui.add_enabled_ui(s.ca_cert_enabled, |ui| {
                            widgets::form_row(ui, "CA file", |ui| {
                                let w = ui.available_width() - 40.0;
                                widgets::text_input(ui, &mut s.ca_cert_path, "PEM file", w);
                                if kit::icon_button(ui, icon::FOLDER_OPEN, "Choose file").clicked()
                                    && let Some(f) = rfd::FileDialog::new().pick_file()
                                {
                                    s.ca_cert_path = f.display().to_string();
                                }
                            });
                        });
                        ui.add_space(12.0);
                        kit::section_label(ui, "Client certificates");
                        ui.add_space(4.0);
                        let mut remove = None;
                        for (i, c) in s.client_certs.iter_mut().enumerate() {
                            kit::card(&p).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    kit::toggle(ui, &mut c.enabled);
                                    widgets::text_input(ui, &mut c.host, "Host", 200.0);
                                    let mut port = c.port.unwrap_or(0);
                                    ui.label(RichText::new("Port").color(p.text_muted));
                                    if ui.add(egui::DragValue::new(&mut port).range(0..=65535)).changed() {
                                        c.port = (port != 0).then_some(port);
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if kit::icon_button(ui, icon::TRASH, "Remove").clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                });
                                for (label, path) in [("Certificate", &mut c.cert_path), ("Key", &mut c.key_path)] {
                                    ui.horizontal(|ui| {
                                        let w = ui.available_width() - 40.0;
                                        widgets::text_input(ui, path, label, w);
                                        if kit::icon_button(ui, icon::FOLDER_OPEN, "Choose file").clicked()
                                            && let Some(f) = rfd::FileDialog::new().pick_file()
                                        {
                                            *path = f.display().to_string();
                                        }
                                    });
                                }
                            });
                            ui.add_space(6.0);
                        }
                        if let Some(i) = remove {
                            s.client_certs.remove(i);
                        }
                        if kit::button(ui, kit::Kind::Secondary, Some(icon::PLUS), "Add certificate").clicked() {
                            s.client_certs.push(ClientCert { enabled: true, ..Default::default() });
                        }
                    }
                    SettingsSection::Data => {
                        kit::card(&p).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new("Data folder").strong());
                            ui.label(
                                RichText::new(app.data.root.display().to_string())
                                    .monospace()
                                    .color(p.text_muted)
                                    .size(12.0),
                            );
                            ui.add_space(6.0);
                            if kit::small_button(
                                ui,
                                kit::Kind::Secondary,
                                Some(icon::ARROW_SQUARE_OUT),
                                &format!("Show in {}", super::platform::file_manager()),
                            )
                            .clicked()
                            {
                                let _ = super::platform::open(&app.data.root);
                            }
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if kit::button(ui, kit::Kind::Secondary, Some(icon::EXPORT), "Export backup…").clicked() {
                                let backup = formats::Backup {
                                    collections: app.collections.clone(),
                                    environments: app.environments.clone(),
                                    globals: Some(app.globals.clone()),
                                    settings: Some(app.settings.clone()),
                                };
                                app.save_text_dialog("courier-backup.json", formats::export_backup(&backup));
                            }
                            if kit::button(ui, kit::Kind::Secondary, Some(icon::TRAY_ARROW_DOWN), "Restore backup…")
                                .clicked()
                                && let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file()
                            {
                                match std::fs::read_to_string(path)
                                    .map_err(|e| e.to_string())
                                    .and_then(|t| formats::import_text(&t))
                                {
                                    Ok(r) => app.apply_import(r),
                                    Err(e) => app.toast_error(e),
                                }
                            }
                        });
                        ui.add_space(14.0);
                        kit::section_label(ui, "Danger zone");
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if kit::button(ui, kit::Kind::Danger, Some(icon::TRASH), "Clear history").clicked() {
                                let days = app.history.clear();
                                app.persist_history_days(days);
                                app.toast("History cleared");
                            }
                            if kit::button(ui, kit::Kind::Danger, Some(icon::ARROW_COUNTER_CLOCKWISE), "Reset settings")
                                .clicked()
                            {
                                *s = Settings::default();
                            }
                        });
                    }
                },
            );
        });
    });
    footer(ui, |ui| {
        if kit::button(ui, kit::Kind::Primary, None, "Save").clicked() {
            app.settings = d.draft.clone();
            app.persist_settings();
            theme::apply(&app.egui_ctx, app.settings.theme);
            app.egui_ctx.set_zoom_factor(app.settings.ui_zoom);
            close = true;
        }
        if kit::button(ui, kit::Kind::Secondary, None, "Cancel").clicked() {
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------- cookies

#[derive(Default)]
pub struct CookieDialog {
    pub domain: Option<String>,
    pub new_domain: String,
    pub raw: String,
    pub allowlist: Option<String>,
}

fn cookies_ui(app: &mut CourierApp, ui: &mut Ui, d: &mut CookieDialog) -> bool {
    let p = theme::palette(ui);
    let mut close = header(ui, icon::COOKIE, "Cookies", "Stored cookies are sent automatically to matching domains");
    let domains = app.jar.domains();
    let mut changed = false;
    ui.horizontal(|ui| {
        let w = ui.available_width() - 90.0;
        widgets::text_input(ui, &mut d.new_domain, "Add a domain, e.g. example.com", w);
        if kit::button_ext(ui, kit::Kind::Secondary, Some(icon::PLUS), "Add", false, !d.new_domain.trim().is_empty())
            .clicked()
        {
            d.domain = Some(d.new_domain.trim().to_lowercase());
            d.new_domain.clear();
        }
    });
    ui.add_space(10.0);
    ui.horizontal_top(|ui| {
        egui::Frame::new()
            .fill(p.input)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .corner_radius(8)
            .inner_margin(6)
            .show(ui, |ui| {
                ui.set_width(180.0);
                ui.set_min_height(220.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                if domains.is_empty() {
                    ui.label(RichText::new("No cookies yet").color(p.text_faint));
                }
                for dom in &domains {
                    let (r, _) = kit::row(
                        ui,
                        egui::Id::new(("dom", dom)),
                        d.domain.as_deref() == Some(dom.as_str()),
                        28.0,
                        |ui| {
                            ui.label(RichText::new(icon::GLOBE).color(p.text_muted));
                            ui.add(egui::Label::new(dom).truncate());
                        },
                    );
                    if r.clicked() {
                        d.domain = Some(dom.clone());
                    }
                }
            });
        ui.vertical(|ui| {
            let Some(dom) = d.domain.clone() else {
                ui.label(RichText::new("Select a domain to see its cookies.").color(p.text_faint));
                return;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&dom).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::small_button(ui, kit::Kind::Danger, Some(icon::TRASH), "Delete domain").clicked() {
                        app.jar.clear_domain(&dom);
                        d.domain = None;
                        changed = true;
                    }
                });
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                for c in app.jar.for_domain(&dom) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(c.to_set_cookie()).monospace().size(12.0)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::icon_button(ui, icon::X, "Delete cookie").clicked() {
                                app.jar.remove(&c.domain, &c.name, &c.path);
                                changed = true;
                            }
                        });
                    });
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let w = ui.available_width() - 80.0;
                widgets::text_input(ui, &mut d.raw, "name=value; Path=/; Max-Age=3600", w);
                if kit::button(ui, kit::Kind::Secondary, None, "Save").clicked() {
                    let url = ::url::Url::parse(&format!("https://{dom}/")).ok();
                    match url.and_then(|u| crate::cookies::parse_set_cookie(&d.raw, &u, chrono::Utc::now())) {
                        Some(mut c) => {
                            c.domain = dom.clone();
                            c.host_only = false;
                            app.jar.upsert(c);
                            d.raw.clear();
                            changed = true;
                        }
                        None => app.toast_error("Enter a cookie as name=value"),
                    }
                }
            });
        });
    });
    ui.add_space(12.0);
    kit::section_label(ui, "Script access");
    ui.label(
        RichText::new("Domains scripts may read with pm.cookies.jar(), comma-separated.")
            .color(p.text_muted)
            .size(12.0),
    );
    ui.add_space(4.0);
    let allow = d.allowlist.get_or_insert_with(|| app.jar.allowlist().join(", "));
    let w = ui.available_width();
    if widgets::text_input(ui, allow, "example.com, api.internal", w).changed() {
        app.jar.set_allowlist(allow.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
        changed = true;
    }
    if changed {
        app.persist_cookies();
    }
    footer(ui, |ui| {
        if kit::button(ui, kit::Kind::Secondary, None, "Done").clicked() {
            close = true;
        }
        if kit::button(ui, kit::Kind::Danger, None, "Clear all").clicked() {
            app.dialog_after = Some(Dialog::Confirm {
                title: "Clear cookies".into(),
                message: "Delete every stored cookie?".into(),
                action: ConfirmAction::ClearCookies,
            });
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------- snippets

pub struct SnippetDialog {
    pub item: RequestItem,
    pub auth: Auth,
    pub collection_id: Option<String>,
    pub raw_http: bool,
    pub resolve: bool,
}

impl SnippetDialog {
    pub fn new(app: &CourierApp, tab: &RequestTab) -> Self {
        let coll = tab.source.as_ref().and_then(|(c, _)| app.collection(c));
        let (auth, _) = exec::effective_auth(&tab.item, coll);
        Self {
            item: tab.item.clone(),
            auth,
            collection_id: tab.source.as_ref().map(|(c, _)| c.clone()),
            raw_http: false,
            resolve: true,
        }
    }
}

fn snippet_ui(app: &mut CourierApp, ui: &mut Ui, d: &mut SnippetDialog) -> bool {
    let mut close = header(ui, icon::CODE, "Code snippet", "Copy this request to run it elsewhere");
    ui.horizontal(|ui| {
        kit::segmented(ui, &mut d.raw_http, &[(false, "cURL"), (true, "HTTP")]);
        ui.add_space(12.0);
        kit::labeled_checkbox(ui, &mut d.resolve, "Resolve variables");
    });
    ui.add_space(10.0);
    let vars = app.var_ctx(d.collection_id.as_deref());
    let pr = formats::snippet::snippet_request(&d.item.request, &d.auth, &vars, &app.settings, d.resolve);
    let text = if d.raw_http { formats::snippet::to_raw_http(&pr) } else { formats::snippet::to_curl(&pr) };
    ui.allocate_ui(egui::vec2(ui.available_width(), 300.0), |ui| {
        widgets::code_view(ui, "snippet_view", &text, super::highlight::Lang::Plain, "", 12.5, true);
    });
    footer(ui, |ui| {
        if kit::button(ui, kit::Kind::Primary, Some(icon::COPY), "Copy").clicked() {
            ui.ctx().copy_text(text.clone());
            app.toast("Copied to clipboard");
        }
        if kit::button(ui, kit::Kind::Secondary, None, "Close").clicked() {
            close = true;
        }
    });
    close
}

// ---------------------------------------------------------------- shortcuts

pub const SHORTCUTS: &[(&str, &str)] = &[
    ("New request tab", "⌘ T"),
    ("New…", "⌘ N"),
    ("Send request", "⌘ Enter"),
    ("Send and download", "⌥ ⌘ Enter"),
    ("Save", "⌘ S"),
    ("Save As", "⇧ ⌘ S"),
    ("Close tab", "⌘ W"),
    ("Reopen closed tab", "⇧ ⌘ T"),
    ("Next tab", "⌃ Tab"),
    ("Previous tab", "⌃ ⇧ Tab"),
    ("Jump to tab 1–8", "⌘ 1–8"),
    ("Focus URL bar", "⌘ L"),
    ("Search the sidebar", "⌘ F"),
    ("Import", "⌘ O"),
    ("Toggle sidebar", "⌘ \\"),
    ("Toggle request/response layout", "⌥ ⌘ V"),
    ("Toggle console", "⌥ ⌘ C"),
    ("Manage environments", "⌥ ⌘ E"),
    ("Runner", "⇧ ⌘ R"),
    ("Settings", "⌘ ,"),
    ("Shortcut cheat sheet", "⌘ /"),
];

// ---------------------------------------------------------------- dispatcher

pub fn show(app: &mut CourierApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.dialog.take() else { return };
    let mut close = false;
    let p = theme::palette_for(ctx.style().visuals.dark_mode);
    let width = match &dialog {
        Dialog::Settings(_) => 720.0,
        Dialog::Cookies(_) | Dialog::Import(_) | Dialog::Snippet(_) => 620.0,
        Dialog::Confirm { .. } | Dialog::ConfirmClose { .. } => 440.0,
        _ => 540.0,
    };
    let frame = egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0_f32, p.border_strong))
        .corner_radius(14)
        .inner_margin(egui::Margin::same(22))
        .shadow(ctx.style().visuals.window_shadow);
    let backdrop = egui::Color32::from_black_alpha(if p.dark { 150 } else { 90 });
    let modal =
        egui::Modal::new(egui::Id::new("courier_modal")).frame(frame).backdrop_color(backdrop).show(ctx, |ui| {
            ui.set_width(width);
            match &mut dialog {
                Dialog::Import(d) => close = import_ui(app, ui, d),
                Dialog::SaveAs(d) => close = save_as_ui(app, ui, d),
                Dialog::Settings(d) => close = settings_ui(app, ui, d),
                Dialog::Cookies(d) => close = cookies_ui(app, ui, d),
                Dialog::Snippet(d) => close = snippet_ui(app, ui, d),
                Dialog::Shortcuts => {
                    close = header(ui, icon::KEYBOARD, "Keyboard shortcuts", "Work faster without the mouse");
                    egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
                        for (label, keys) in SHORTCUTS {
                            ui.horizontal(|ui| {
                                ui.label(*label);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let parts: Vec<&str> = keys.split(' ').filter(|k| !k.is_empty()).collect();
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        kit::kbd(ui, &parts)
                                    });
                                });
                            });
                            ui.add_space(2.0);
                            kit::divider(ui);
                            ui.add_space(2.0);
                        }
                    });
                }
                Dialog::NewItem => {
                    close = header(ui, icon::PLUS, "Create new", "Start a request, a collection or an environment");
                    ui.horizontal(|ui| {
                        for (glyph, title, sub, which) in [
                            (icon::PAPER_PLANE_TILT, "HTTP request", "Build and send", 0),
                            (icon::STACK, "Collection", "Group requests", 1),
                            (icon::GLOBE, "Environment", "Switch variables", 2),
                        ] {
                            let (rect, r) = ui.allocate_exact_size(egui::vec2(160.0, 110.0), egui::Sense::click());
                            let hovered = r.hovered();
                            ui.painter().rect(
                                rect,
                                12.0,
                                if hovered { p.surface_hover } else { p.surface },
                                egui::Stroke::new(1.0_f32, if hovered { p.accent } else { p.border }),
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().text(
                                rect.center_top() + egui::vec2(0.0, 32.0),
                                egui::Align2::CENTER_CENTER,
                                glyph,
                                egui::FontId::proportional(24.0),
                                p.accent,
                            );
                            ui.painter().text(
                                rect.center_top() + egui::vec2(0.0, 66.0),
                                egui::Align2::CENTER_CENTER,
                                title,
                                egui::FontId::proportional(14.0),
                                p.text,
                            );
                            ui.painter().text(
                                rect.center_top() + egui::vec2(0.0, 86.0),
                                egui::Align2::CENTER_CENTER,
                                sub,
                                egui::FontId::proportional(12.0),
                                p.text_muted,
                            );
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
                            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                match which {
                                    0 => app.new_request_tab(),
                                    1 => {
                                        let id = app.create_collection("New Collection");
                                        app.sidebar = super::SidebarSection::Collections;
                                        app.sidebar_open = true;
                                        app.renaming = Some((id, "New Collection".into()));
                                    }
                                    _ => {
                                        let id = app.create_environment("New Environment");
                                        app.open_environment(Some(&id));
                                    }
                                }
                                close = true;
                            }
                        }
                    });
                }
                Dialog::ConfirmClose { tab } => {
                    let tab = *tab;
                    let idx = app.tabs.iter().position(|t| t.id == tab);
                    let name = idx.map(|i| app.tab_title(&app.tabs[i]).1).unwrap_or_default();
                    close = header(ui, icon::WARNING, "Unsaved changes", "");
                    ui.label(
                        RichText::new(format!("“{name}” has unsaved changes. Save them before closing?"))
                            .color(p.text_muted),
                    );
                    footer(ui, |ui| {
                        if kit::button(ui, kit::Kind::Primary, None, "Save").clicked() {
                            if let Some(i) = idx {
                                let linked = matches!(&app.tabs[i].kind, TabKind::Request(r) if r.source.is_some());
                                if linked {
                                    app.save_tab_request(i);
                                    app.close_tab(i);
                                } else {
                                    app.dialog_after = Some(Dialog::SaveAs(SaveAsDialog::new(app, i)));
                                }
                            }
                            close = true;
                        }
                        if kit::button(ui, kit::Kind::Secondary, None, "Don't save").clicked() {
                            if let Some(i) = idx {
                                app.close_tab(i);
                            }
                            close = true;
                        }
                        if kit::button(ui, kit::Kind::Ghost, None, "Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Confirm { title, message, action } => {
                    close = header(ui, icon::TRASH, title.as_str(), "");
                    ui.label(RichText::new(message.as_str()).color(p.text_muted));
                    footer(ui, |ui| {
                        if kit::button(ui, kit::Kind::Danger, Some(icon::TRASH), "Delete").clicked() {
                            match action {
                                ConfirmAction::DeleteItem { collection_id, item_id } => {
                                    app.delete_item(collection_id, item_id)
                                }
                                ConfirmAction::DeleteCollection(id) => app.delete_collection(id),
                                ConfirmAction::DeleteEnvironment(id) => app.delete_environment(id),
                                ConfirmAction::ClearHistory => {
                                    let days = app.history.clear();
                                    app.persist_history_days(days);
                                }
                                ConfirmAction::ClearCookies => {
                                    app.jar.clear();
                                    app.persist_cookies();
                                }
                            }
                            close = true;
                        }
                        if kit::button(ui, kit::Kind::Secondary, None, "Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Issues(issues) => {
                    close = header(
                        ui,
                        icon::WARNING,
                        "Some files could not be loaded",
                        "They were moved aside so nothing is lost",
                    );
                    for i in issues.iter() {
                        widgets::note(ui, icon::FILE_TEXT, p.warning, &format!("{}\n{}", i.path.display(), i.error));
                        if let Some(m) = &i.moved_to {
                            ui.label(RichText::new(format!("Moved to {}", m.display())).color(p.text_faint).size(12.0));
                        }
                        ui.add_space(6.0);
                    }
                    footer(ui, |ui| {
                        if kit::button(ui, kit::Kind::Primary, None, "OK").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
    if modal.should_close() && !matches!(dialog, Dialog::ConfirmClose { .. }) {
        close = true;
    }
    if !close {
        app.dialog = Some(dialog);
    }
    if let Some(next) = app.dialog_after.take() {
        app.dialog = Some(next);
    }
}
