//! Collection runner tab: configuration, live results and exports.

use super::kit::{self, icon};
use super::widgets::{self, human_time};
use super::{AppEvent, CourierApp, theme};
use crate::runner::{self, RequestRun, RunConfig, RunControl, RunEvent, RunResult, RunStatus};
use egui::{RichText, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultFilter {
    All,
    Passed,
    Failed,
}

pub enum RunState {
    Configuring,
    Running { control: RunControl, current: Option<String> },
    Done,
}

pub struct RunnerTab {
    pub config: RunConfig,
    /// Request ids in run order with their "include" flag.
    pub selection: Vec<(String, bool)>,
    pub data_error: Option<String>,
    pub state: RunState,
    pub result: RunResult,
    pub filter: ResultFilter,
    pub selected: Option<(usize, usize)>,
}

impl RunnerTab {
    pub fn new(config: RunConfig, app: &CourierApp) -> Self {
        let mut tab = Self {
            config,
            selection: Vec::new(),
            data_error: None,
            state: RunState::Configuring,
            result: RunResult::default(),
            filter: ResultFilter::All,
            selected: None,
        };
        tab.reload_selection(app);
        tab
    }

    fn reload_selection(&mut self, app: &CourierApp) {
        self.selection = app
            .collection(&self.config.collection_id)
            .map(|c| c.requests_under(self.config.folder_id.as_deref()).iter().map(|r| (r.id.clone(), true)).collect())
            .unwrap_or_default();
    }

    pub fn stop(&self) {
        if let RunState::Running { control, .. } = &self.state {
            control.stop();
        }
    }

    pub fn on_event(&mut self, event: RunEvent) {
        match event {
            RunEvent::IterationStarted(i) => {
                if self.result.iterations.len() <= i {
                    self.result.iterations.push(runner::IterationResult { index: i, requests: Vec::new() });
                }
            }
            RunEvent::RequestStarted { request_id, .. } => {
                if let RunState::Running { current, .. } = &mut self.state {
                    *current = Some(request_id);
                }
            }
            RunEvent::RequestFinished { iteration, run } => {
                if let Some(it) = self.result.iterations.iter_mut().find(|i| i.index == iteration) {
                    it.requests.push(*run);
                }
                self.result.compute_totals();
            }
            RunEvent::Console(_) => {}
        }
    }

    pub fn finish(&mut self, result: RunResult) {
        self.result = result;
        self.state = RunState::Done;
    }
}

fn start(app: &mut CourierApp, tab_id: u64, tab: &mut RunnerTab) {
    let Some(collection) = app.collection(&tab.config.collection_id).cloned() else { return };
    tab.config.request_ids = tab.selection.iter().filter(|(_, on)| *on).map(|(id, _)| id.clone()).collect();
    if tab.config.request_ids.is_empty() {
        app.toast_error("Select at least one request to run");
        return;
    }
    let env = tab.config.environment_id.as_deref().and_then(|id| app.environment(id)).cloned();
    let mut vars = app.var_ctx(Some(&collection.id));
    vars.environment = env.as_ref().map(|e| e.variables.clone());
    let env_name = env.map(|e| e.name).unwrap_or_default();
    let control = RunControl::default();
    tab.state = RunState::Running { control: control.clone(), current: None };
    tab.result = RunResult {
        collection_name: collection.name.clone(),
        environment_name: env_name.clone(),
        started_at: chrono::Utc::now(),
        ..Default::default()
    };
    tab.selected = None;
    let config = tab.config.clone();
    let executor = app.executor.clone();
    let tx = app.tx.clone();
    let ctx = app.egui_ctx.clone();
    app.rt.spawn(async move {
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        let (result, vars) =
            runner::run_collection(&executor, &collection, &config, vars, &env_name, &control, move |event| {
                let _ = tx2.send(AppEvent::Run { tab: tab_id, event });
                ctx2.request_repaint();
            })
            .await;
        let _ = tx.send(AppEvent::RunFinished {
            tab: tab_id,
            result: Box::new(result),
            vars: Box::new(vars),
            keep: config.keep_variable_values,
        });
        ctx.request_repaint();
    });
}

pub fn show(app: &mut CourierApp, ui: &mut Ui, tab_id: u64, tab: &mut RunnerTab) {
    let running = matches!(tab.state, RunState::Running { .. });
    let p = theme::palette(ui);
    egui::SidePanel::left(egui::Id::new(("runner_cfg", tab_id)))
        .resizable(true)
        .default_width(380.0)
        .width_range(320.0..=520.0)
        .frame(egui::Frame::new().fill(p.canvas).inner_margin(egui::Margin { left: 0, right: 16, top: 0, bottom: 0 }))
        .show_inside(ui, |ui| {
            ui.add_enabled_ui(!running, |ui| config_panel(app, ui, tab_id, tab));
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.canvas).inner_margin(egui::Margin { left: 16, right: 4, top: 0, bottom: 0 }))
        .show_inside(ui, |ui| results_panel(app, ui, tab_id, tab));
}

fn picker(
    ui: &mut Ui,
    id: String,
    glyph: &str,
    current: &str,
    options: &[(Option<String>, String)],
    value: &mut Option<String>,
) -> bool {
    let p = theme::palette(ui);
    let mut picked = None;
    let w = ui.available_width();
    kit::dropdown(ui, &id, Some((glyph, p.text_muted)), current, None, w, |ui| {
        for (v, label) in options {
            if kit::menu_item(ui, value == v, label).clicked() {
                picked = Some(v.clone());
            }
        }
    });
    match picked {
        Some(v) if v != *value => {
            *value = v;
            true
        }
        _ => false,
    }
}

fn config_panel(app: &mut CourierApp, ui: &mut Ui, tab_id: u64, tab: &mut RunnerTab) {
    let p = theme::palette(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new("Run collection").text_style(theme::title_style()));
        ui.label(
            RichText::new("Choose what to run, how many times and with which data.").color(p.text_muted).size(12.5),
        );
        ui.add_space(14.0);

        kit::section_label(ui, "Collection");
        ui.add_space(4.0);
        let colls: Vec<(Option<String>, String)> =
            app.collections.iter().map(|c| (Some(c.id.clone()), c.name.clone())).collect();
        let mut cid = Some(tab.config.collection_id.clone());
        let current = app.collection(&tab.config.collection_id).map(|c| c.name.clone()).unwrap_or_default();
        if picker(ui, format!("run_coll{tab_id}"), icon::STACK, &current, &colls, &mut cid) {
            tab.config.collection_id = cid.unwrap_or_default();
            tab.config.folder_id = None;
            tab.reload_selection(app);
        }
        ui.add_space(10.0);
        kit::section_label(ui, "Folder");
        ui.add_space(4.0);
        let mut folders: Vec<(Option<String>, String)> = vec![(None, "All requests".into())];
        folders.extend(
            app.collection(&tab.config.collection_id)
                .map(folder_list)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, n)| (Some(id), n)),
        );
        let shown = folders
            .iter()
            .find(|(id, _)| *id == tab.config.folder_id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "All requests".into());
        let mut fid = tab.config.folder_id.clone();
        if picker(ui, format!("run_folder{tab_id}"), icon::FOLDER, &shown, &folders, &mut fid) {
            tab.config.folder_id = fid;
            tab.reload_selection(app);
        }
        ui.add_space(10.0);
        kit::section_label(ui, "Environment");
        ui.add_space(4.0);
        let mut envs: Vec<(Option<String>, String)> = vec![(None, "No environment".into())];
        envs.extend(app.environments.iter().map(|e| (Some(e.id.clone()), e.name.clone())));
        let env_name = tab
            .config
            .environment_id
            .as_deref()
            .and_then(|id| app.environment(id))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "No environment".into());
        let mut eid = tab.config.environment_id.clone();
        if picker(ui, format!("run_env{tab_id}"), icon::GLOBE, &env_name, &envs, &mut eid) {
            tab.config.environment_id = eid;
        }
        ui.add_space(14.0);
        ui.columns(2, |cols| {
            kit::section_label(&mut cols[0], "Iterations");
            cols[0]
                .add(egui::DragValue::new(&mut tab.config.iterations).range(0..=10_000))
                .on_hover_text("0 = one iteration per data row");
            kit::section_label(&mut cols[1], "Delay (ms)");
            cols[1].add(egui::DragValue::new(&mut tab.config.delay_ms).range(0..=600_000).speed(10));
        });
        ui.add_space(14.0);
        kit::section_label(ui, "Data file");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = tab.config.data_file_name.clone().unwrap_or_else(|| "Select CSV or JSON…".into());
            if kit::button(ui, kit::Kind::Secondary, Some(icon::FILE_ARROW_UP), &label).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Data", &["csv", "json"]).pick_file()
            {
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| runner::parse_data_file(&t))
                {
                    Ok(rows) => {
                        tab.config.data = rows;
                        tab.config.data_file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                        tab.data_error = None;
                        if tab.config.iterations == 1 {
                            tab.config.iterations = 0;
                        }
                    }
                    Err(e) => tab.data_error = Some(e),
                }
            }
            if tab.config.data_file_name.is_some() && kit::icon_button(ui, icon::X, "Remove data file").clicked() {
                tab.config.data.clear();
                tab.config.data_file_name = None;
            }
        });
        if let Some(e) = &tab.data_error {
            ui.label(RichText::new(e).color(p.danger).size(12.0));
        }
        if !tab.config.data.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(
                RichText::new(format!("Preview · {} rows", tab.config.data.len())).color(p.text_muted),
            )
            .show(ui, |ui| {
                let cols: Vec<String> = tab.config.data[0].keys().cloned().collect();
                egui::Grid::new("data_preview").spacing([16.0, 4.0]).show(ui, |ui| {
                    for c in &cols {
                        ui.label(RichText::new(c).text_style(theme::caption_style()).color(p.text_faint));
                    }
                    ui.end_row();
                    for row in tab.config.data.iter().take(20) {
                        for c in &cols {
                            ui.label(RichText::new(row.get(c).cloned().unwrap_or_default()).monospace().size(12.0));
                        }
                        ui.end_row();
                    }
                });
            });
        }
        ui.add_space(14.0);
        kit::section_label(ui, "Options");
        ui.add_space(6.0);
        kit::setting_switch(
            ui,
            "Keep variable values",
            "Write variable changes back after the run.",
            &mut tab.config.keep_variable_values,
        );
        kit::setting_switch(
            ui,
            "Save responses",
            "Keep headers and bodies in the results.",
            &mut tab.config.persist_responses,
        );
        kit::setting_switch(ui, "Stop on error", "Stop when a request cannot be sent.", &mut tab.config.stop_on_error);
        kit::setting_switch(
            ui,
            "Stop on first test failure",
            "Stop as soon as any test fails.",
            &mut tab.config.stop_on_failure,
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            kit::section_label(ui, "Run order");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::small_button(ui, kit::Kind::Ghost, None, "Reset").clicked() {
                    tab.reload_selection(app);
                }
                if kit::small_button(ui, kit::Kind::Ghost, None, "None").clicked() {
                    tab.selection.iter_mut().for_each(|(_, on)| *on = false);
                }
                if kit::small_button(ui, kit::Kind::Ghost, None, "All").clicked() {
                    tab.selection.iter_mut().for_each(|(_, on)| *on = true);
                }
            });
        });
        ui.add_space(4.0);
        let coll = app.collection(&tab.config.collection_id).cloned();
        let mut swap = None;
        egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, p.border)).corner_radius(8).inner_margin(4).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let n = tab.selection.len();
                for (i, (id, on)) in tab.selection.iter_mut().enumerate() {
                    let Some(r) = coll.as_ref().and_then(|c| c.find_request(id)) else { continue };
                    let (_, _) = kit::row(ui, egui::Id::new(("runsel", id.as_str())), false, 30.0, |ui| {
                        kit::checkbox(ui, on);
                        kit::method_tag(ui, &r.request.method, 36.0);
                        ui.add(
                            egui::Label::new(RichText::new(&r.name).color(if *on { p.text } else { p.text_faint }))
                                .truncate()
                                .selectable(false),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            if i + 1 < n && kit::icon_button(ui, icon::ARROW_DOWN, "Move down").clicked() {
                                swap = Some((i, i + 1));
                            }
                            if i > 0 && kit::icon_button(ui, icon::ARROW_UP, "Move up").clicked() {
                                swap = Some((i - 1, i));
                            }
                        });
                    });
                }
            },
        );
        if let Some((a, b)) = swap
            && b < tab.selection.len()
        {
            tab.selection.swap(a, b);
        }
        ui.add_space(14.0);
        let selected = tab.selection.iter().filter(|(_, on)| *on).count();
        let label = format!("Run {} request{}", selected, if selected == 1 { "" } else { "s" });
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
        let fill = if resp.hovered() { p.accent_hover } else { p.accent };
        ui.painter().rect_filled(rect, 9.0, fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{}  {label}", icon::PLAY),
            egui::FontId::proportional(14.0),
            p.on_accent,
        );
        let name = app.collection(&tab.config.collection_id).map(|c| c.name.clone()).unwrap_or_default();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Run {name}")));
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            start(app, tab_id, tab);
        }
        ui.add_space(10.0);
    });
}

fn stat(ui: &mut Ui, label: &str, value: String, color: egui::Color32) {
    let p = theme::palette(ui);
    let w = ui.data(|d| d.get_temp::<f32>(egui::Id::new("stat_w"))).unwrap_or(110.0);
    kit::card(&p).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.set_width(w);
        ui.vertical(|ui| {
            ui.label(RichText::new(label.to_uppercase()).text_style(theme::caption_style()).color(p.text_faint));
            ui.label(RichText::new(value).size(20.0).strong().color(color));
        });
    });
}

fn results_panel(app: &mut CourierApp, ui: &mut Ui, tab_id: u64, tab: &mut RunnerTab) {
    let p = theme::palette(ui);
    let mut run_again = false;
    ui.horizontal(|ui| match &tab.state {
        RunState::Configuring => {
            ui.label(RichText::new("Results").text_style(theme::title_style()));
        }
        RunState::Running { control, .. } => {
            ui.add(egui::Spinner::new().color(p.accent));
            ui.label(
                RichText::new(if control.is_paused() { "Paused" } else { "Running…" }).text_style(theme::title_style()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::button(ui, kit::Kind::Danger, Some(icon::STOP), "Stop").clicked() {
                    control.stop();
                }
                if control.is_paused() {
                    if kit::button(ui, kit::Kind::Secondary, Some(icon::PLAY), "Resume").clicked() {
                        control.resume();
                    }
                } else if kit::button(ui, kit::Kind::Secondary, Some(icon::PAUSE), "Pause").clicked() {
                    control.pause();
                }
            });
        }
        RunState::Done => {
            let (label, color, glyph) = match tab.result.status {
                RunStatus::Completed => ("Run complete", p.success, icon::CHECK_CIRCLE),
                RunStatus::Stopped => ("Run stopped", p.warning, icon::STOP),
                RunStatus::Aborted => ("Run aborted", p.danger, icon::WARNING_CIRCLE),
                RunStatus::Running => ("Running", p.accent, icon::PLAY),
            };
            ui.label(RichText::new(glyph).size(20.0).color(color));
            ui.label(RichText::new(label).text_style(theme::title_style()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::button(ui, kit::Kind::Secondary, Some(icon::EXPORT), "JUnit XML").clicked() {
                    app.save_text_dialog("courier-run.xml", tab.result.to_junit());
                }
                if kit::button(ui, kit::Kind::Secondary, Some(icon::EXPORT), "JSON").clicked() {
                    let text = crate::storage::to_json_bytes(&tab.result);
                    app.save_text_dialog("courier-run.json", String::from_utf8_lossy(&text).into_owned());
                }
                if kit::button(ui, kit::Kind::Primary, Some(icon::ARROWS_CLOCKWISE), "Run again").clicked() {
                    run_again = true;
                }
            });
        }
    });
    if let Some(reason) = &tab.result.stop_reason {
        ui.add_space(6.0);
        widgets::note(ui, icon::WARNING, p.warning, reason);
    }
    ui.add_space(12.0);
    let t = tab.result.totals.clone();
    ui.horizontal(|ui| {
        // Card content width: share minus spacing, inner margins and stroke.
        let gap = ui.spacing().item_spacing.x;
        let w = ((ui.available_width() - 4.0 * gap) / 5.0 - 30.0).max(60.0);
        ui.data_mut(|d| d.insert_temp(egui::Id::new("stat_w"), w));
        stat(ui, "Iterations", tab.result.iterations.len().to_string(), p.text);
        stat(ui, "Requests", t.requests.to_string(), p.text);
        stat(ui, "Passed", t.tests_passed.to_string(), p.success);
        stat(ui, "Failed", t.tests_failed.to_string(), if t.tests_failed > 0 { p.danger } else { p.text });
        stat(ui, "Avg. time", human_time(t.avg_time_ms), p.text);
    });
    ui.add_space(12.0);
    kit::segmented(
        ui,
        &mut tab.filter,
        &[(ResultFilter::All, "All"), (ResultFilter::Passed, "Passed"), (ResultFilter::Failed, "Failed")],
    );
    ui.add_space(10.0);
    let selected = tab.selected;
    let mut new_selected = None;
    let filter = tab.filter;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if tab.result.iterations.is_empty() {
            kit::empty_state(ui, icon::PLAY, "No results yet", "Configure the run on the left, then start it.");
        }
        for (ii, it) in tab.result.iterations.iter().enumerate() {
            kit::section_label(ui, &format!("Iteration {}", it.index + 1));
            ui.add_space(4.0);
            for (ri, r) in it.requests.iter().enumerate() {
                let failed = r.error.is_some() || r.tests.iter().any(|t| !t.passed && !t.skipped);
                if (filter == ResultFilter::Passed && failed) || (filter == ResultFilter::Failed && !failed) {
                    continue;
                }
                let open = selected == Some((ii, ri));
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(egui::Stroke::new(1.0_f32, if open { p.border_strong } else { p.border }))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let resp = ui
                            .horizontal(|ui| {
                                let (glyph, color) = if r.skipped {
                                    (icon::CIRCLE, p.text_faint)
                                } else if failed {
                                    (icon::X_CIRCLE, p.danger)
                                } else {
                                    (icon::CHECK_CIRCLE, p.success)
                                };
                                ui.label(RichText::new(glyph).size(16.0).color(color));
                                kit::method_tag(ui, &r.method, 40.0);
                                ui.label(RichText::new(&r.name).strong());
                                kit::label_with_trailing(
                                    ui,
                                    RichText::new(&r.url).color(p.text_faint).size(12.0),
                                    190.0,
                                    |ui| {
                                        ui.label(RichText::new(human_time(r.time_ms)).color(p.text_muted).size(12.0));
                                        match (r.status, &r.error) {
                                            (Some(s), _) => {
                                                kit::badge(
                                                    ui,
                                                    &format!("{s} {}", r.status_text),
                                                    theme::status_color(s, p.dark),
                                                );
                                            }
                                            (None, Some(_)) => {
                                                kit::badge(ui, "Error", p.danger);
                                            }
                                            _ if r.skipped => {
                                                kit::badge(ui, "Skipped", p.text_muted);
                                            }
                                            _ => {}
                                        }
                                    },
                                );
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            new_selected = Some(if open { None } else { Some((ii, ri)) });
                        }
                        for test in &r.tests {
                            ui.horizontal(|ui| {
                                ui.add_space(26.0);
                                let (label, color) = if test.skipped {
                                    ("SKIP", p.text_faint)
                                } else if test.passed {
                                    ("PASS", p.success)
                                } else {
                                    ("FAIL", p.danger)
                                };
                                ui.label(RichText::new(label).monospace().size(11.0).strong().color(color));
                                ui.label(&test.name);
                                if let Some(e) = &test.error {
                                    ui.label(RichText::new(e).color(p.text_muted).size(12.0));
                                }
                            });
                        }
                        if let Some(e) = &r.error {
                            ui.horizontal(|ui| {
                                ui.add_space(26.0);
                                ui.label(RichText::new(e).color(p.danger).size(12.5));
                            });
                        }
                        for e in &r.script_errors {
                            ui.horizontal(|ui| {
                                ui.add_space(26.0);
                                ui.label(RichText::new(e).color(p.warning).size(12.5));
                            });
                        }
                        if open {
                            ui.add_space(6.0);
                            details(ui, r);
                        }
                    });
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
        }
    });
    if let Some(s) = new_selected {
        tab.selected = s;
    }
    if run_again {
        start(app, tab_id, tab);
    }
}

fn details(ui: &mut Ui, r: &RequestRun) {
    let p = theme::palette(ui);
    egui::Frame::new().fill(p.input).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if r.response_body.is_none() && r.request_headers.is_empty() {
            ui.label(
                RichText::new("Turn on “Save responses” to keep headers and bodies for each request.")
                    .color(p.text_faint),
            );
            return;
        }
        kit::section_label(ui, "Request headers");
        for (k, v) in &r.request_headers {
            ui.label(RichText::new(format!("{k}: {v}")).monospace().size(12.0));
        }
        ui.add_space(6.0);
        kit::section_label(ui, "Response headers");
        for (k, v) in &r.response_headers {
            ui.label(RichText::new(format!("{k}: {v}")).monospace().size(12.0));
        }
        if let Some(b) = &r.response_body {
            ui.add_space(6.0);
            kit::section_label(ui, "Response body");
            let text: String = b.chars().take(20_000).collect();
            ui.label(RichText::new(text).monospace().size(12.0));
        }
    });
}

fn folder_list(c: &crate::model::Collection) -> Vec<(String, String)> {
    fn walk(items: &[crate::model::Item], prefix: &str, out: &mut Vec<(String, String)>) {
        for i in items {
            if let crate::model::Item::Folder(f) = i {
                let name = if prefix.is_empty() { f.name.clone() } else { format!("{prefix} / {}", f.name) };
                out.push((f.id.clone(), name.clone()));
                walk(&f.items, &name, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&c.items, "", &mut out);
    out
}
