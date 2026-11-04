//! The Courier desktop UI (egui).

pub mod dialogs;
pub mod highlight;
pub mod kit;
pub mod platform;
mod request_tab;
pub mod runner_tab;
mod sidebar;
mod tabs;
pub mod theme;
pub mod widgets;

use crate::auth::oauth2::{self, OAuth2Token, TokenStore};
use crate::cookies::{CookieFile, CookieJar};
use crate::exec::{ExecOptions, Execution, Executor};
use crate::formats::{ImportResult, Imported};
use crate::history::{History, HistoryEntry};
use crate::model::*;
use crate::runner::{RunConfig, RunEvent, RunResult};
use crate::script::ConsoleEntry;
use crate::settings::Settings;
use crate::storage::{DataDir, LoadIssue, Persister};
use crate::vars::VarContext;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub use request_tab::{ReqSection, RequestTab, RespState};
pub use runner_tab::RunnerTab;
pub use tabs::{EnvTab, ExampleTab, NodeTab};

/// Results delivered from background tasks to the UI thread.
pub enum AppEvent {
    Executed { tab: u64, exec: Box<Execution>, source: Option<(String, String)> },
    Token { tab: u64, result: Result<OAuth2Token, String> },
    Run { tab: u64, event: RunEvent },
    RunFinished { tab: u64, result: Box<RunResult>, vars: Box<VarContext>, keep: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SidebarSection {
    #[default]
    Collections,
    Environments,
    History,
}

pub enum TabKind {
    Request(Box<RequestTab>),
    Node(NodeTab),
    Environment(EnvTab),
    Example(Box<ExampleTab>),
    Runner(Box<RunnerTab>),
}

pub struct Tab {
    pub id: u64,
    pub kind: TabKind,
}

/// What a deleted object needs to be restored by "Undo".
#[derive(Clone)]
pub enum Undo {
    Item { collection_id: String, parent: Option<String>, index: usize, item: Item },
    Collection(Collection),
    Environment(Environment),
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
    pub undo: Option<Undo>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConsoleLevel {
    Log,
    Info,
    Warn,
    Error,
    Http,
}

pub struct ConsoleLine {
    pub time: DateTime<Local>,
    pub level: ConsoleLevel,
    pub text: String,
}

/// Persisted UI state (`workspace.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub tabs: Vec<SavedTab>,
    pub active_tab: usize,
    pub active_env: Option<String>,
    pub sidebar: SidebarSection,
    pub expanded: Vec<String>,
    pub console_open: bool,
    pub sidebar_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SavedTab {
    Request { source: Option<(String, String)>, item: RequestItem, saved: Option<RequestItem> },
    Node { collection_id: String, node_id: Option<String> },
    Environment { env_id: Option<String> },
}

/// Actions requested while a tab is being drawn, run once the tab is back
/// in place (by tab index).
pub enum Deferred {
    Send(usize, Option<PathBuf>),
    Cancel(usize),
}

/// Modal dialogs.
pub enum Dialog {
    Import(dialogs::ImportDialog),
    SaveAs(dialogs::SaveAsDialog),
    Settings(dialogs::SettingsDialog),
    Cookies(dialogs::CookieDialog),
    Snippet(dialogs::SnippetDialog),
    ConfirmClose { tab: u64 },
    Confirm { title: String, message: String, action: ConfirmAction },
    Shortcuts,
    NewItem,
    Issues(Vec<LoadIssue>),
}

pub enum ConfirmAction {
    DeleteItem { collection_id: String, item_id: String },
    DeleteCollection(String),
    DeleteEnvironment(String),
    ClearHistory,
    ClearCookies,
}

pub struct CourierApp {
    pub data: DataDir,
    pub persister: Persister,
    pub settings: Settings,
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    pub globals: Environment,
    pub active_env: Option<String>,
    pub history: History,
    pub jar: Arc<CookieJar>,
    pub tokens: TokenStore,
    pub executor: Executor,
    pub rt: Arc<tokio::runtime::Runtime>,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub closed_tabs: Vec<SavedTab>,
    next_tab_id: u64,
    pub sidebar: SidebarSection,
    pub sidebar_open: bool,
    pub sidebar_filter: String,
    /// Text in the header's "search requests" box.
    pub global_search: String,
    pub expanded: HashSet<String>,
    pub renaming: Option<(String, String)>,
    pub console: Vec<ConsoleLine>,
    pub console_open: bool,
    pub console_filter: String,
    pub dialog: Option<Dialog>,
    /// A dialog to open once the current one closes.
    pub dialog_after: Option<Dialog>,
    pub toasts: Vec<Toast>,
    pub deferred: Vec<Deferred>,
    /// Focus requests from shortcuts, consumed by the widgets that own focus.
    pub focus_url: bool,
    pub focus_sidebar_search: bool,
    pub focus_global_search: bool,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    last_workspace: Vec<u8>,
    last_workspace_check: Instant,
    pub egui_ctx: egui::Context,
}

impl CourierApp {
    /// Load everything from `data_dir` and build the app.
    pub fn new(ctx: &egui::Context, data_dir: PathBuf) -> Self {
        let data = DataDir::open(&data_dir)
            .unwrap_or_else(|e| panic!("cannot open data directory {}: {e}", data_dir.display()));
        let mut issues = Vec::new();
        let (mut settings, i) = data.load_doc::<Settings>(&data.settings_path());
        issues.extend(i);
        if settings.migrate() {
            let _ = data.save_doc(&data.settings_path(), &settings);
        }
        let (collections, i) = data.load_collections();
        issues.extend(i);
        let (environments, i) = data.load_environments();
        issues.extend(i);
        let (globals, i) = data.load_globals();
        issues.extend(i);
        let (cookies, i) = data.load_doc::<CookieFile>(&data.cookies_path());
        issues.extend(i);
        let (tokens, i) = data.load_doc::<TokenStore>(&data.tokens_path());
        issues.extend(i);
        let (workspace, i) = data.load_doc::<Workspace>(&data.workspace_path());
        issues.extend(i);
        let history = History::load(&data.history_dir(), settings.history_limit);
        let jar = Arc::new(CookieJar::from_file(cookies));
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("courier-io")
                .build()
                .expect("tokio runtime"),
        );
        let executor = Executor::new(settings.clone(), jar.clone());
        let (tx, rx) = channel();
        theme::install_fonts(ctx);
        theme::apply(ctx, settings.theme);
        ctx.set_zoom_factor(settings.ui_zoom.clamp(0.5, 3.0));
        let mut app = Self {
            data,
            persister: Persister::new(Duration::from_millis(300)),
            settings,
            collections,
            environments,
            globals,
            active_env: None,
            history,
            jar,
            tokens,
            executor,
            rt,
            tabs: Vec::new(),
            active_tab: 0,
            closed_tabs: Vec::new(),
            next_tab_id: 1,
            sidebar: workspace.sidebar,
            sidebar_open: true,
            sidebar_filter: String::new(),
            global_search: String::new(),
            expanded: workspace.expanded.iter().cloned().collect(),
            renaming: None,
            console: Vec::new(),
            console_open: workspace.console_open,
            console_filter: String::new(),
            dialog: None,
            dialog_after: None,
            toasts: Vec::new(),
            deferred: Vec::new(),
            focus_url: false,
            focus_sidebar_search: false,
            focus_global_search: false,
            tx,
            rx,
            last_workspace: Vec::new(),
            last_workspace_check: Instant::now(),
            egui_ctx: ctx.clone(),
        };
        app.active_env = workspace.active_env.filter(|id| app.environments.iter().any(|e| &e.id == id));
        for saved in workspace.tabs {
            app.restore_tab(saved);
        }
        app.active_tab = workspace.active_tab.min(app.tabs.len().saturating_sub(1));
        if app.tabs.is_empty() {
            app.new_request_tab();
        }
        if !issues.is_empty() {
            app.dialog = Some(Dialog::Issues(issues));
        }
        app.last_workspace = crate::storage::to_json_bytes(&app.workspace());
        app
    }

    // ------------------------------------------------------------ tabs

    pub fn alloc_tab_id(&mut self) -> u64 {
        self.next_tab_id += 1;
        self.next_tab_id
    }

    pub fn push_tab(&mut self, kind: TabKind) -> usize {
        let id = self.alloc_tab_id();
        let at = if self.tabs.is_empty() { 0 } else { (self.active_tab + 1).min(self.tabs.len()) };
        self.tabs.insert(at, Tab { id, kind });
        self.active_tab = at;
        at
    }

    pub fn new_request_tab(&mut self) {
        let item = RequestItem::new("Untitled Request", Request::default());
        self.push_tab(TabKind::Request(Box::new(RequestTab::new(None, item, None))));
    }

    /// Focus an existing tab for `request_id` or open a new one.
    pub fn open_request(&mut self, collection_id: &str, request_id: &str) {
        if let Some(i) = self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Request(r) if r.source.as_ref().is_some_and(|(c, r)| c == collection_id && r == request_id))) {
            self.active_tab = i;
            return;
        }
        let Some(item) = self.collection(collection_id).and_then(|c| c.find_request(request_id)).cloned() else {
            return;
        };
        let tab = RequestTab::new(Some((collection_id.to_string(), request_id.to_string())), item.clone(), Some(item));
        self.push_tab(TabKind::Request(Box::new(tab)));
    }

    pub fn open_node(&mut self, collection_id: &str, node_id: Option<&str>) {
        if let Some(i) = self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Node(n) if n.collection_id == collection_id && n.node_id.as_deref() == node_id)) {
            self.active_tab = i;
            return;
        }
        self.push_tab(TabKind::Node(NodeTab::new(collection_id, node_id)));
    }

    pub fn open_environment(&mut self, env_id: Option<&str>) {
        if let Some(i) =
            self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Environment(e) if e.env_id.as_deref() == env_id))
        {
            self.active_tab = i;
            return;
        }
        self.push_tab(TabKind::Environment(EnvTab::new(env_id)));
    }

    pub fn open_example(&mut self, collection_id: &str, request_id: &str, example_id: &str) {
        let found = self
            .collection(collection_id)
            .and_then(|c| c.find_request(request_id))
            .and_then(|r| r.examples.iter().find(|e| e.id == example_id).map(|e| (r.name.clone(), e.clone())));
        if let Some((request_name, example)) = found {
            self.push_tab(TabKind::Example(Box::new(ExampleTab {
                collection_id: collection_id.into(),
                request_id: request_id.into(),
                request_name,
                example,
                editing: false,
            })));
        }
    }

    pub fn open_runner(&mut self, collection_id: &str, folder_id: Option<&str>) {
        let config = RunConfig {
            collection_id: collection_id.to_string(),
            folder_id: folder_id.map(str::to_string),
            environment_id: self.active_env.clone(),
            ..Default::default()
        };
        let tab = RunnerTab::new(config, self);
        self.push_tab(TabKind::Runner(Box::new(tab)));
    }

    pub fn tab_is_dirty(&self, idx: usize) -> bool {
        match &self.tabs[idx].kind {
            TabKind::Request(r) => r.is_dirty(),
            _ => false,
        }
    }

    /// Close a tab, asking first if it has unsaved changes.
    pub fn request_close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tab_is_dirty(idx) {
            self.dialog = Some(Dialog::ConfirmClose { tab: self.tabs[idx].id });
        } else {
            self.close_tab(idx);
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        if let TabKind::Runner(r) = &tab.kind {
            r.stop();
        }
        if let Some(saved) = Self::save_tab(&tab) {
            self.closed_tabs.push(saved);
            if self.closed_tabs.len() > 20 {
                self.closed_tabs.remove(0);
            }
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        } else if idx < self.active_tab {
            self.active_tab -= 1;
        }
        if self.tabs.is_empty() {
            self.new_request_tab();
        }
    }

    pub fn reopen_closed_tab(&mut self) {
        if let Some(saved) = self.closed_tabs.pop() {
            self.restore_tab(saved);
            self.active_tab = self.tabs.len() - 1;
        }
    }

    fn save_tab(tab: &Tab) -> Option<SavedTab> {
        match &tab.kind {
            TabKind::Request(r) => {
                Some(SavedTab::Request { source: r.source.clone(), item: r.item.clone(), saved: r.saved.clone() })
            }
            TabKind::Node(n) => {
                Some(SavedTab::Node { collection_id: n.collection_id.clone(), node_id: n.node_id.clone() })
            }
            TabKind::Environment(e) => Some(SavedTab::Environment { env_id: e.env_id.clone() }),
            TabKind::Example(_) | TabKind::Runner(_) => None,
        }
    }

    fn restore_tab(&mut self, saved: SavedTab) {
        let id = self.alloc_tab_id();
        let kind = match saved {
            SavedTab::Request { source, item, saved } => {
                // Drop the link if the request no longer exists.
                let source = source.filter(|(c, r)| self.collection(c).is_some_and(|c| c.find_request(r).is_some()));
                let saved = if source.is_some() { saved } else { None };
                TabKind::Request(Box::new(RequestTab::new(source, item, saved)))
            }
            SavedTab::Node { collection_id, node_id } => {
                if self.collection(&collection_id).is_none() {
                    return;
                }
                TabKind::Node(NodeTab::new(&collection_id, node_id.as_deref()))
            }
            SavedTab::Environment { env_id } => {
                if env_id.as_ref().is_some_and(|id| !self.environments.iter().any(|e| &e.id == id)) {
                    return;
                }
                TabKind::Environment(EnvTab::new(env_id.as_deref()))
            }
        };
        self.tabs.push(Tab { id, kind });
    }

    pub fn workspace(&self) -> Workspace {
        Workspace {
            tabs: self.tabs.iter().filter_map(Self::save_tab).collect(),
            active_tab: self.active_tab,
            active_env: self.active_env.clone(),
            sidebar: self.sidebar,
            expanded: self.expanded.iter().cloned().collect(),
            console_open: self.console_open,
            sidebar_open: self.sidebar_open,
        }
    }

    // ------------------------------------------------------------ data access

    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn environment(&self, id: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.id == id)
    }

    pub fn active_environment(&self) -> Option<&Environment> {
        self.active_env.as_deref().and_then(|id| self.environment(id))
    }

    /// Variable context for a request in `collection_id`.
    pub fn var_ctx(&self, collection_id: Option<&str>) -> VarContext {
        VarContext {
            local: Default::default(),
            data: Default::default(),
            environment: self.active_environment().map(|e| e.variables.clone()),
            collection: collection_id
                .and_then(|id| self.collection(id))
                .map(|c| c.variables.clone())
                .unwrap_or_default(),
            globals: self.globals.variables.clone(),
        }
    }

    // ------------------------------------------------------------ persistence

    pub fn persist_collection(&self, id: &str) {
        if let Some(c) = self.collection(id) {
            self.persister.write(self.data.collection_path(&c.id), DataDir::collection_bytes(c));
        }
    }

    pub fn persist_environment(&self, id: &str) {
        if let Some(e) = self.environment(id) {
            self.persister.write(
                self.data.environment_path(&e.id),
                DataDir::environment_bytes(e, crate::formats::postman::Scope::Environment),
            );
        }
    }

    pub fn persist_globals(&self) {
        self.persister.write(
            self.data.globals_path(),
            DataDir::environment_bytes(&self.globals, crate::formats::postman::Scope::Globals),
        );
    }

    pub fn persist_settings(&mut self) {
        self.persister.write(self.data.settings_path(), crate::storage::to_json_bytes(&self.settings));
        self.executor.http.set_settings(self.settings.clone());
        self.history.limit = self.settings.history_limit;
        let days = self.history.trim();
        self.persist_history_days(days);
    }

    pub fn persist_cookies(&self) {
        self.persister.write(self.data.cookies_path(), crate::storage::to_json_bytes(&self.jar.snapshot()));
    }

    pub fn persist_tokens(&self) {
        self.persister.write(self.data.tokens_path(), crate::storage::to_json_bytes(&self.tokens));
    }

    pub fn persist_history_days(&self, days: BTreeSet<chrono::NaiveDate>) {
        for day in days {
            match self.history.day_file(day) {
                (path, Some(bytes)) => self.persister.write(path, bytes),
                (path, None) => self.persister.delete(path),
            }
        }
    }

    fn persist_workspace_if_changed(&mut self, force: bool) {
        if !force && self.last_workspace_check.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_workspace_check = Instant::now();
        let bytes = crate::storage::to_json_bytes(&self.workspace());
        if bytes != self.last_workspace {
            self.persister.write(self.data.workspace_path(), bytes.clone());
            self.last_workspace = bytes;
        }
    }

    /// Flush everything to disk (called on exit).
    pub fn shutdown(&mut self) {
        self.settings.ui_zoom = self.egui_ctx.zoom_factor();
        self.persister.write(self.data.settings_path(), crate::storage::to_json_bytes(&self.settings));
        self.persist_workspace_if_changed(true);
        self.persist_cookies();
        self.persister.flush();
    }

    // ------------------------------------------------------------ feedback

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            error: false,
            until: Instant::now() + Duration::from_secs(4),
            undo: None,
        });
    }

    pub fn toast_error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            error: true,
            until: Instant::now() + Duration::from_secs(8),
            undo: None,
        });
    }

    pub fn toast_undo(&mut self, text: impl Into<String>, undo: Undo) {
        self.toasts.push(Toast {
            text: text.into(),
            error: false,
            until: Instant::now() + Duration::from_secs(10),
            undo: Some(undo),
        });
    }

    pub fn log(&mut self, entries: &[ConsoleEntry]) {
        for e in entries {
            let level = match e.level.as_str() {
                "info" => ConsoleLevel::Info,
                "warn" => ConsoleLevel::Warn,
                "error" => ConsoleLevel::Error,
                "http" => ConsoleLevel::Http,
                _ => ConsoleLevel::Log,
            };
            self.console.push(ConsoleLine { time: Local::now(), level, text: e.message.clone() });
        }
        if self.console.len() > 5000 {
            let excess = self.console.len() - 5000;
            self.console.drain(..excess);
        }
    }

    // ------------------------------------------------------------ actions

    /// Send the request in tab `idx`. With `download`, stream the body to a file.
    pub fn send(&mut self, idx: usize, download: Option<PathBuf>) {
        let Some(Tab { id, kind: TabKind::Request(tab) }) = self.tabs.get(idx) else { return };
        let tab_id = *id;
        let mut item = tab.item.clone();
        if item.request.url.trim().is_empty() {
            self.toast_error("Enter a URL first");
            return;
        }
        if self.settings.trim_keys_and_values {
            item.request.url = item.request.url.trim().to_string();
        }
        let source = tab.source.clone();
        let collection = source.as_ref().and_then(|(c, _)| self.collection(c)).cloned();
        let vars = self.var_ctx(source.as_ref().map(|(c, _)| c.as_str()));
        let opts = ExecOptions {
            environment_name: self.active_environment().map(|e| e.name.clone()).unwrap_or_default(),
            download_to: download,
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        if let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get_mut(idx) {
            tab.resp = RespState::Loading { started: Instant::now(), cancel: cancel.clone() };
        }
        let executor = self.executor.clone();
        let tx = self.tx.clone();
        let ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            let exec = executor.execute(&item, collection.as_ref(), vars, &opts, &cancel).await;
            let _ = tx.send(AppEvent::Executed { tab: tab_id, exec: Box::new(exec), source });
            ctx.request_repaint();
        });
    }

    pub fn cancel_send(&mut self, idx: usize) {
        if let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get_mut(idx)
            && let RespState::Loading { cancel, .. } = &tab.resp
        {
            cancel.cancel();
        }
    }

    /// Write variable changes made by scripts back to their stores.
    pub fn apply_vars(&mut self, vars: &VarContext, collection_id: Option<&str>, env_id: Option<&str>) {
        if let (Some(env_vars), Some(env_id)) = (&vars.environment, env_id)
            && let Some(env) = self.environments.iter_mut().find(|e| e.id == env_id)
            && &env.variables != env_vars
        {
            env.variables = env_vars.clone();
            let id = env.id.clone();
            self.persist_environment(&id);
        }
        if let Some(cid) = collection_id
            && let Some(c) = self.collection_mut(cid)
            && c.variables != vars.collection
        {
            c.variables = vars.collection.clone();
            self.persist_collection(cid);
        }
        if self.globals.variables != vars.globals {
            self.globals.variables = vars.globals.clone();
            self.persist_globals();
        }
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AppEvent::Executed { tab, exec, source } => {
                    let env_id = self.active_env.clone();
                    self.apply_vars(&exec.vars, source.as_ref().map(|(c, _)| c.as_str()), env_id.as_deref());
                    self.log(&exec.console);
                    for e in &exec.script_errors {
                        self.log(&[ConsoleEntry { level: "error".into(), message: e.clone() }]);
                    }
                    if let Some(token) = &exec.refreshed_token {
                        self.tokens.add(token.clone());
                        self.persist_tokens();
                        if let Some((cid, rid)) = &source {
                            self.store_token_on_owner(cid, rid, token);
                        }
                    }
                    self.persist_cookies();
                    let name = self.tabs.iter().find(|t| t.id == tab).and_then(|t| match &t.kind {
                        TabKind::Request(r) => Some(r.item.name.clone()),
                        _ => None,
                    });
                    if !exec.skipped {
                        let entry = HistoryEntry::from_execution(
                            &name.unwrap_or_default(),
                            &exec,
                            source.as_ref().map(|s| s.0.clone()),
                            source.as_ref().map(|s| s.1.clone()),
                        );
                        let days = self.history.add(entry);
                        self.persist_history_days(days);
                    }
                    if let Some(downloaded) = exec.response.as_ref().and_then(|r| r.downloaded_to.clone()) {
                        self.toast(format!("Saved response to {}", downloaded.display()));
                    }
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab)
                        && let TabKind::Request(r) = &mut t.kind
                    {
                        if let Some(token) = &exec.refreshed_token
                            && r.item.request.auth.kind == AuthType::OAuth2
                        {
                            oauth2::use_token(&mut r.item.request.auth, token);
                        }
                        r.resp = RespState::Done(exec);
                    }
                }
                AppEvent::Token { tab, result } => match result {
                    Ok(token) => {
                        self.tokens.add(token.clone());
                        self.persist_tokens();
                        self.apply_token_to_tab(tab, &token);
                        self.toast(format!("Received access token “{}”", token.name));
                    }
                    Err(e) => self.toast_error(format!("Could not get token: {e}")),
                },
                AppEvent::Run { tab, event } => {
                    if let RunEvent::Console(entries) = &event {
                        self.log(entries);
                    }
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab)
                        && let TabKind::Runner(r) = &mut t.kind
                    {
                        r.on_event(event);
                    }
                }
                AppEvent::RunFinished { tab, result, vars, keep } => {
                    if keep {
                        let cid = result.config.as_ref().map(|c| c.collection_id.clone());
                        let env_id = result.config.as_ref().and_then(|c| c.environment_id.clone());
                        self.apply_vars(&vars, cid.as_deref(), env_id.as_deref());
                    }
                    self.persist_cookies();
                    self.save_run(&result);
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab)
                        && let TabKind::Runner(r) = &mut t.kind
                    {
                        r.finish(*result);
                    }
                }
            }
        }
    }

    fn save_run(&self, result: &RunResult) {
        let dir = self.data.runs_dir();
        self.persister.write(dir.join(format!("{}.json", result.id)), crate::storage::to_json_bytes(result));
        // Keep the 50 most recent runs.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<(std::time::SystemTime, PathBuf)> =
                rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
            files.sort();
            if files.len() > 50 {
                for (_, p) in files.iter().take(files.len() - 50) {
                    self.persister.delete(p.clone());
                }
            }
        }
    }

    /// Put a new token on the auth that owns it (request, folder or collection).
    fn store_token_on_owner(&mut self, collection_id: &str, request_id: &str, token: &OAuth2Token) {
        let Some(c) = self.collection_mut(collection_id) else { return };
        if let Some(r) = c.find_request_mut(request_id)
            && r.request.auth.kind == AuthType::OAuth2
        {
            oauth2::use_token(&mut r.request.auth, token);
            self.persist_collection(collection_id);
            return;
        }
        let folder_ids: Vec<String> =
            c.ancestors(request_id).unwrap_or_default().iter().rev().map(|f| f.id.clone()).collect();
        for fid in folder_ids {
            if let Some(Item::Folder(f)) = c.find_mut(&fid) {
                if f.auth.kind == AuthType::OAuth2 {
                    oauth2::use_token(&mut f.auth, token);
                    self.persist_collection(collection_id);
                    return;
                }
                if f.auth.kind != AuthType::Inherit {
                    return;
                }
            }
        }
        if c.auth.kind == AuthType::OAuth2 {
            oauth2::use_token(&mut c.auth, token);
            self.persist_collection(collection_id);
        }
    }

    fn apply_token_to_tab(&mut self, tab_id: u64, token: &OAuth2Token) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab_id) else { return };
        match &mut t.kind {
            TabKind::Request(r) => oauth2::use_token(&mut r.item.request.auth, token),
            TabKind::Node(n) => {
                let (cid, nid) = (n.collection_id.clone(), n.node_id.clone());
                if let Some(c) = self.collection_mut(&cid) {
                    match nid.as_deref().and_then(|id| c.find_mut(id)) {
                        Some(Item::Folder(f)) => oauth2::use_token(&mut f.auth, token),
                        _ => oauth2::use_token(&mut c.auth, token),
                    }
                    self.persist_collection(&cid);
                }
            }
            _ => {}
        }
    }

    /// Start an OAuth 2.0 token request for the auth shown in tab `tab_id`.
    pub fn fetch_token(&mut self, tab_id: u64, auth: Auth, collection_id: Option<String>) {
        let vars = self.var_ctx(collection_id.as_deref());
        let auth = crate::auth::resolve(&auth, &vars);
        let tx = self.tx.clone();
        let ctx = self.egui_ctx.clone();
        self.toast("Waiting for the authorization server…");
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let open = |url: &str| {
                let _ = platform::open(url);
            };
            let result =
                oauth2::fetch_token(&client, &auth, open, Duration::from_secs(300)).await.map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Token { tab: tab_id, result });
            ctx.request_repaint();
        });
    }

    /// Save the request tab back to its collection, or ask where to save it.
    pub fn save_tab_request(&mut self, idx: usize) {
        let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get(idx) else { return };
        match tab.source.clone() {
            Some((cid, rid)) => {
                let item = tab.item.clone();
                if let Some(c) = self.collection_mut(&cid) {
                    if let Some(r) = c.find_request_mut(&rid) {
                        *r = RequestItem { id: rid.clone(), ..item.clone() };
                    }
                    self.persist_collection(&cid);
                    if let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get_mut(idx) {
                        tab.saved = Some(tab.item.clone());
                    }
                    self.sync_open_tabs(&cid, &rid, idx);
                }
            }
            None => self.dialog = Some(Dialog::SaveAs(dialogs::SaveAsDialog::new(self, idx))),
        }
    }

    /// After a save, other tabs showing the same request get the new baseline.
    fn sync_open_tabs(&mut self, cid: &str, rid: &str, except: usize) {
        let item = self.collection(cid).and_then(|c| c.find_request(rid)).cloned();
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i == except {
                continue;
            }
            if let TabKind::Request(r) = &mut t.kind
                && r.source.as_ref().is_some_and(|(c, r2)| c == cid && r2 == rid)
                && let Some(item) = &item
            {
                if !r.is_dirty() {
                    r.item = item.clone();
                }
                r.saved = Some(item.clone());
            }
        }
    }

    /// Save `item` into a collection folder and link the tab to it.
    pub fn save_as(&mut self, tab_idx: usize, collection_id: &str, folder: Option<&str>, name: &str) {
        let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get(tab_idx) else { return };
        let mut item = tab.item.clone();
        item.id = new_id();
        item.name = name.to_string();
        let rid = item.id.clone();
        let Some(c) = self.collection_mut(collection_id) else { return };
        let len = match folder.and_then(|f| c.find(f)) {
            Some(Item::Folder(f)) => f.items.len(),
            _ => c.items.len(),
        };
        c.insert(folder, len, Item::Request(item.clone()));
        self.persist_collection(collection_id);
        if let Some(f) = folder {
            self.expanded.insert(f.to_string());
        }
        self.expanded.insert(collection_id.to_string());
        if let Some(Tab { kind: TabKind::Request(tab), .. }) = self.tabs.get_mut(tab_idx) {
            tab.source = Some((collection_id.to_string(), rid));
            tab.item = item.clone();
            tab.saved = Some(item);
        }
        self.toast(format!("Saved “{name}”"));
    }

    pub fn create_collection(&mut self, name: &str) -> String {
        let c = Collection::new(name);
        let id = c.id.clone();
        self.collections.push(c);
        self.persist_collection(&id);
        self.expanded.insert(id.clone());
        id
    }

    /// Add a folder or request under `parent` (or the collection root).
    pub fn add_item(&mut self, collection_id: &str, parent: Option<&str>, item: Item) {
        let id = item.id().to_string();
        let is_request = matches!(item, Item::Request(_));
        if let Some(c) = self.collection_mut(collection_id) {
            let len = match parent.and_then(|p| c.find(p)) {
                Some(Item::Folder(f)) => f.items.len(),
                _ => c.items.len(),
            };
            c.insert(parent, len, item);
        }
        self.persist_collection(collection_id);
        self.expanded.insert(collection_id.to_string());
        if let Some(p) = parent {
            self.expanded.insert(p.to_string());
        }
        if is_request {
            self.open_request(collection_id, &id);
        } else {
            self.renaming = Some((id.clone(), "New Folder".into()));
        }
    }

    pub fn delete_item(&mut self, collection_id: &str, item_id: &str) {
        let Some(c) = self.collection_mut(collection_id) else { return };
        let Some((parent, index)) = c.position(item_id) else { return };
        let Some(item) = c.remove(item_id) else { return };
        let name = item.name().to_string();
        self.persist_collection(collection_id);
        // Unlink open tabs that pointed into the deleted subtree.
        let mut ids = vec![item.id().to_string()];
        if let Item::Folder(f) = &item {
            let tmp = Collection { items: f.items.clone(), ..Collection::new("") };
            ids.extend(tmp.requests().iter().map(|r| r.id.clone()));
        }
        for t in &mut self.tabs {
            if let TabKind::Request(r) = &mut t.kind
                && r.source.as_ref().is_some_and(|(c, rid)| c == collection_id && ids.contains(rid))
            {
                r.source = None;
                r.saved = None;
            }
        }
        self.toast_undo(
            format!("Deleted “{name}”"),
            Undo::Item { collection_id: collection_id.into(), parent, index, item },
        );
    }

    pub fn delete_collection(&mut self, id: &str) {
        if let Some(pos) = self.collections.iter().position(|c| c.id == id) {
            let c = self.collections.remove(pos);
            self.persister.delete(self.data.collection_path(id));
            for t in &mut self.tabs {
                if let TabKind::Request(r) = &mut t.kind
                    && r.source.as_ref().is_some_and(|(cid, _)| cid == id)
                {
                    r.source = None;
                    r.saved = None;
                }
            }
            self.tabs.retain(|t| !matches!(&t.kind, TabKind::Node(n) if n.collection_id == id));
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
            if self.tabs.is_empty() {
                self.new_request_tab();
            }
            self.toast_undo(format!("Deleted collection “{}”", c.name), Undo::Collection(c));
        }
    }

    pub fn delete_environment(&mut self, id: &str) {
        if let Some(pos) = self.environments.iter().position(|e| e.id == id) {
            let e = self.environments.remove(pos);
            self.persister.delete(self.data.environment_path(id));
            if self.active_env.as_deref() == Some(id) {
                self.active_env = None;
            }
            self.tabs.retain(|t| !matches!(&t.kind, TabKind::Environment(env) if env.env_id.as_deref() == Some(id)));
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
            if self.tabs.is_empty() {
                self.new_request_tab();
            }
            self.toast_undo(format!("Deleted environment “{}”", e.name), Undo::Environment(e));
        }
    }

    pub fn undo(&mut self, undo: Undo) {
        match undo {
            Undo::Item { collection_id, parent, index, item } => {
                if let Some(c) = self.collection_mut(&collection_id) {
                    if !c.insert(parent.as_deref(), index, item.clone()) {
                        c.insert(None, index, item);
                    }
                    self.persist_collection(&collection_id);
                }
            }
            Undo::Collection(c) => {
                let id = c.id.clone();
                self.collections.push(c);
                self.collections.sort_by_key(|c| c.name.to_lowercase());
                self.persist_collection(&id);
            }
            Undo::Environment(e) => {
                let id = e.id.clone();
                self.environments.push(e);
                self.environments.sort_by_key(|e| e.name.to_lowercase());
                self.persist_environment(&id);
            }
        }
    }

    /// Move an item (drag and drop) to `parent` at `index`.
    pub fn move_item(
        &mut self,
        from_collection: &str,
        item_id: &str,
        to_collection: &str,
        parent: Option<&str>,
        index: usize,
    ) {
        if Some(item_id) == parent {
            return;
        }
        // Refuse to move a folder into its own subtree.
        if let (Some(c), Some(p)) = (self.collection(from_collection), parent)
            && from_collection == to_collection
            && c.ancestors(p).unwrap_or_default().iter().any(|f| f.id == item_id)
        {
            return;
        }
        let Some(c) = self.collection_mut(from_collection) else { return };
        let old_pos = c.position(item_id);
        let Some(item) = c.remove(item_id) else { return };
        let mut index = index;
        if let Some((old_parent, old_index)) = old_pos.clone()
            && from_collection == to_collection
            && old_parent.as_deref() == parent
            && old_index < index
        {
            index -= 1;
        }
        let moved_requests: Vec<String> = match &item {
            Item::Request(r) => vec![r.id.clone()],
            Item::Folder(f) => Collection { items: f.items.clone(), ..Collection::new("") }
                .requests()
                .iter()
                .map(|r| r.id.clone())
                .collect(),
        };
        let ok = self.collection_mut(to_collection).map(|c| c.insert(parent, index, item.clone())).unwrap_or(false);
        if !ok {
            // Put it back where it was.
            if let (Some(c), Some((p, i))) = (self.collection_mut(from_collection), old_pos) {
                c.insert(p.as_deref(), i, item);
            }
            return;
        }
        self.persist_collection(from_collection);
        if from_collection != to_collection {
            self.persist_collection(to_collection);
            for t in &mut self.tabs {
                if let TabKind::Request(r) = &mut t.kind
                    && let Some((c, rid)) = &mut r.source
                    && c == from_collection
                    && moved_requests.contains(rid)
                {
                    *c = to_collection.to_string();
                }
            }
        }
    }

    pub fn duplicate_item(&mut self, collection_id: &str, item_id: &str) {
        let Some(c) = self.collection_mut(collection_id) else { return };
        let Some((parent, index)) = c.position(item_id) else { return };
        let Some(item) = c.find(item_id) else { return };
        let mut copy = item.duplicate();
        copy.set_name(format!("{} Copy", item.name()));
        c.insert(parent.as_deref(), index + 1, copy);
        self.persist_collection(collection_id);
    }

    pub fn duplicate_collection(&mut self, id: &str) {
        let Some(c) = self.collection(id) else { return };
        let mut copy = c.clone();
        copy.id = new_id();
        copy.name = format!("{} Copy", c.name);
        copy.items = copy.items.iter().map(Item::duplicate).collect();
        let new = copy.id.clone();
        self.collections.push(copy);
        self.persist_collection(&new);
    }

    pub fn rename(&mut self, id: &str, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(c) = self.collections.iter_mut().find(|c| c.id == id) {
            c.name = name;
            self.persist_collection(id);
            return;
        }
        if let Some(e) = self.environments.iter_mut().find(|e| e.id == id) {
            e.name = name;
            self.persist_environment(id);
            return;
        }
        let owner = self.collections.iter().find(|c| c.find(id).is_some()).map(|c| c.id.clone());
        if let Some(cid) = owner {
            if let Some(item) = self.collection_mut(&cid).and_then(|c| c.find_mut(id)) {
                item.set_name(name.clone());
            }
            self.persist_collection(&cid);
            for t in &mut self.tabs {
                if let TabKind::Request(r) = &mut t.kind
                    && r.source.as_ref().is_some_and(|(_, rid)| rid == id)
                {
                    r.item.name = name.clone();
                    if let Some(s) = &mut r.saved {
                        s.name = name.clone();
                    }
                }
            }
        }
    }

    pub fn create_environment(&mut self, name: &str) -> String {
        let env = Environment::new(name);
        let id = env.id.clone();
        self.environments.push(env);
        self.persist_environment(&id);
        id
    }

    pub fn duplicate_environment(&mut self, id: &str) {
        if let Some(e) = self.environment(id) {
            let mut copy = e.clone();
            copy.id = new_id();
            copy.name = format!("{} Copy", e.name);
            let new = copy.id.clone();
            self.environments.push(copy);
            self.persist_environment(&new);
        }
    }

    /// Apply the result of an import.
    pub fn apply_import(&mut self, result: ImportResult) {
        let mut summary = Vec::new();
        for item in result.items {
            match item {
                Imported::Collection(mut c) => {
                    if self.collection(&c.id).is_some() {
                        c.id = new_id();
                    }
                    summary.push(format!("collection “{}”", c.name));
                    let id = c.id.clone();
                    self.collections.push(c);
                    self.persist_collection(&id);
                    self.expanded.insert(id);
                    self.sidebar = SidebarSection::Collections;
                }
                Imported::Environment(mut e) => {
                    if self.environment(&e.id).is_some() {
                        e.id = new_id();
                    }
                    summary.push(format!("environment “{}”", e.name));
                    let id = e.id.clone();
                    self.environments.push(e);
                    self.persist_environment(&id);
                }
                Imported::Globals(g) => {
                    for v in g.variables {
                        match self.globals.variables.iter_mut().find(|x| x.key == v.key) {
                            Some(existing) => *existing = v,
                            None => self.globals.variables.push(v),
                        }
                    }
                    self.persist_globals();
                    summary.push("globals".into());
                }
                Imported::Requests(reqs) => {
                    for r in reqs {
                        summary.push(format!("request {}", r.name));
                        self.push_tab(TabKind::Request(Box::new(RequestTab::new(None, r, None))));
                    }
                }
                Imported::Backup(b) => {
                    let n = b.collections.len();
                    for c in b.collections {
                        self.collections.retain(|x| x.id != c.id);
                        let id = c.id.clone();
                        self.collections.push(c);
                        self.persist_collection(&id);
                    }
                    for e in b.environments {
                        self.environments.retain(|x| x.id != e.id);
                        let id = e.id.clone();
                        self.environments.push(e);
                        self.persist_environment(&id);
                    }
                    if let Some(g) = b.globals {
                        self.globals = g;
                        self.persist_globals();
                    }
                    if let Some(s) = b.settings {
                        self.settings = s;
                        self.persist_settings();
                        theme::apply(&self.egui_ctx, self.settings.theme);
                    }
                    summary.push(format!("backup with {n} collections"));
                }
            }
        }
        self.collections.sort_by_key(|c| c.name.to_lowercase());
        self.environments.sort_by_key(|e| e.name.to_lowercase());
        for w in &result.warnings {
            self.log(&[ConsoleEntry { level: "warn".into(), message: format!("Import: {w}") }]);
        }
        if !summary.is_empty() {
            self.toast(format!("Imported {}", summary.join(", ")));
        }
        if !result.warnings.is_empty() {
            self.toast(format!("{} import warning(s) — see the console", result.warnings.len()));
        }
    }

    pub fn export_collection_to_file(&mut self, id: &str) {
        let Some(c) = self.collection(id) else { return };
        let text = crate::formats::postman::export_collection(c);
        let file = format!("{}.postman_collection.json", sanitize_file_name(&c.name));
        self.save_text_dialog(&file, text);
    }

    pub fn export_environment_to_file(&mut self, id: Option<&str>) {
        let (env, scope, file) = match id {
            Some(id) => match self.environment(id) {
                Some(e) => (
                    e.clone(),
                    crate::formats::postman::Scope::Environment,
                    format!("{}.postman_environment.json", sanitize_file_name(&e.name)),
                ),
                None => return,
            },
            None => (
                self.globals.clone(),
                crate::formats::postman::Scope::Globals,
                "globals.postman_globals.json".to_string(),
            ),
        };
        let text = crate::formats::postman::export_environment(&env, scope, false);
        self.save_text_dialog(&file, text);
    }

    pub fn save_text_dialog(&mut self, default_name: &str, text: String) {
        if let Some(path) = rfd::FileDialog::new().set_file_name(default_name).save_file() {
            match crate::storage::write_atomic(&path, text.as_bytes()) {
                Ok(()) => self.toast(format!("Exported to {}", path.display())),
                Err(e) => self.toast_error(format!("Export failed: {e}")),
            }
        }
    }

    // ------------------------------------------------------------ frame

    /// Draw one frame. Separate from `eframe::App` so tests can drive it.
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.handle_events();
        self.handle_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        let p = theme::palette_for(ctx.style().visuals.dark_mode);

        egui::TopBottomPanel::top("app_header")
            .exact_height(44.0)
            .show_separator_line(true)
            .frame(egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ctx, |ui| self.app_header(ui));
        egui::SidePanel::left("rail")
            .exact_width(64.0)
            .resizable(false)
            .show_separator_line(true)
            .frame(egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(8, 8)))
            .show(ctx, |ui| self.rail(ui));
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(28.0)
            .frame(egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(10, 2)))
            .show(ctx, |ui| self.status_bar(ui));
        if self.console_open {
            egui::TopBottomPanel::bottom("console")
                .resizable(true)
                .default_height(200.0)
                .min_height(90.0)
                .frame(egui::Frame::new().fill(p.canvas).inner_margin(egui::Margin::symmetric(14, 8)))
                .show(ctx, |ui| self.console_panel(ui));
        }
        if self.sidebar_open {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(300.0)
                .width_range(220.0..=600.0)
                .frame(egui::Frame::new().fill(p.canvas).inner_margin(egui::Margin {
                    left: 10,
                    right: 8,
                    top: 10,
                    bottom: 6,
                }))
                .show(ctx, |ui| sidebar::show(self, ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.canvas)).show(ctx, |ui| self.central(ui));
        dialogs::show(self, ctx);
        self.show_toasts(ctx);
        self.persist_workspace_if_changed(false);
        if self
            .tabs
            .iter()
            .any(|t| matches!(&t.kind, TabKind::Request(r) if matches!(r.resp, RespState::Loading { .. })))
            || !self.toasts.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            let text = match (&f.path, &f.bytes) {
                (Some(p), _) => std::fs::read_to_string(p).map_err(|e| e.to_string()),
                (None, Some(b)) => Ok(String::from_utf8_lossy(b).into_owned()),
                _ => Err("unreadable file".into()),
            };
            let mut dlg = dialogs::ImportDialog::default();
            match text {
                Ok(t) => dlg.set_text(t),
                Err(e) => dlg.error = Some(e),
            }
            self.dialog = Some(Dialog::Import(dlg));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let alt_cmd = Modifiers::COMMAND | Modifiers::ALT;
        let pressed = |k: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&k));
        if self.dialog.is_some() {
            return;
        }
        let idx = self.active_tab;
        if pressed(KeyboardShortcut::new(alt_cmd, Key::Enter))
            && let Some(Tab { kind: TabKind::Request(_), .. }) = self.tabs.get(idx)
            && let Some(path) = rfd::FileDialog::new().set_file_name("response").save_file()
        {
            self.send(idx, Some(path));
        }
        if pressed(KeyboardShortcut::new(cmd, Key::Enter))
            && let Some(Tab { kind: TabKind::Request(r), .. }) = self.tabs.get(idx)
        {
            if matches!(r.resp, RespState::Loading { .. }) {
                self.cancel_send(idx);
            } else {
                self.send(idx, None);
            }
        }
        if pressed(KeyboardShortcut::new(shift_cmd, Key::S)) {
            if let Some(Tab { kind: TabKind::Request(_), .. }) = self.tabs.get(idx) {
                self.dialog = Some(Dialog::SaveAs(dialogs::SaveAsDialog::new(self, idx)));
            }
        } else if pressed(KeyboardShortcut::new(cmd, Key::S)) {
            self.save_tab_request(idx);
        }
        if pressed(KeyboardShortcut::new(shift_cmd, Key::T)) {
            self.reopen_closed_tab();
        } else if pressed(KeyboardShortcut::new(cmd, Key::T)) {
            self.new_request_tab();
        }
        if pressed(KeyboardShortcut::new(cmd, Key::W)) {
            self.request_close_tab(idx);
        }
        if pressed(KeyboardShortcut::new(cmd, Key::N)) {
            self.dialog = Some(Dialog::NewItem);
        }
        if pressed(KeyboardShortcut::new(cmd, Key::O)) {
            self.dialog = Some(Dialog::Import(dialogs::ImportDialog::default()));
        }
        if pressed(KeyboardShortcut::new(cmd, Key::Comma)) {
            self.dialog = Some(Dialog::Settings(dialogs::SettingsDialog::new(&self.settings)));
        }
        if pressed(KeyboardShortcut::new(cmd, Key::K)) {
            self.focus_global_search = true;
        }
        if pressed(KeyboardShortcut::new(cmd, Key::L)) {
            self.focus_url = true;
        }
        if pressed(KeyboardShortcut::new(cmd, Key::F)) && self.sidebar_open {
            self.focus_sidebar_search = true;
        }
        if pressed(KeyboardShortcut::new(cmd, Key::Backslash)) {
            self.sidebar_open = !self.sidebar_open;
        }
        if pressed(KeyboardShortcut::new(cmd, Key::Slash)) {
            self.dialog = Some(Dialog::Shortcuts);
        }
        if pressed(KeyboardShortcut::new(alt_cmd, Key::C)) {
            self.console_open = !self.console_open;
        }
        if pressed(KeyboardShortcut::new(alt_cmd, Key::E)) {
            self.sidebar = SidebarSection::Environments;
            self.sidebar_open = true;
        }
        if pressed(KeyboardShortcut::new(alt_cmd, Key::V)) {
            self.settings.layout = match self.settings.layout {
                crate::settings::Layout::Stacked => crate::settings::Layout::SideBySide,
                crate::settings::Layout::SideBySide => crate::settings::Layout::Stacked,
            };
            self.persist_settings();
        }
        if pressed(KeyboardShortcut::new(shift_cmd, Key::R))
            && let Some(c) = self.collections.first()
        {
            let id = c.id.clone();
            self.open_runner(&id, None);
        }
        if pressed(KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)) && !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
        if pressed(KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)) && !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        }
        let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8];
        for (i, k) in digits.into_iter().enumerate() {
            if pressed(KeyboardShortcut::new(cmd, k)) && i < self.tabs.len() {
                self.active_tab = i;
            }
        }
        if pressed(KeyboardShortcut::new(cmd, Key::Num9)) && !self.tabs.is_empty() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    /// Vertical navigation rail with the Courier mark and section switches.
    fn rail(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            for (section, glyph, label) in [
                (SidebarSection::Collections, kit::icon::STACK, "Collections"),
                (SidebarSection::Environments, kit::icon::GLOBE, "Environments"),
                (SidebarSection::History, kit::icon::CLOCK_COUNTER_CLOCKWISE, "History"),
            ] {
                let selected = self.sidebar_open && self.sidebar == section;
                if rail_item(ui, glyph, label, selected).clicked() {
                    if selected {
                        self.sidebar_open = false;
                    } else {
                        self.sidebar = section;
                        self.sidebar_open = true;
                    }
                }
                ui.add_space(2.0);
            }
        });
    }

    /// Full-width bar at the top: brand and workspace on the left, request
    /// search in the middle, settings on the right.
    fn app_header(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let full = ui.max_rect();
        ui.horizontal_centered(|ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 6.0, p.accent);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                kit::icon::PAPER_PLANE_TILT,
                egui::FontId::proportional(14.0),
                p.on_accent,
            );
            if resp.on_hover_text("Toggle sidebar").clicked() {
                self.sidebar_open = !self.sidebar_open;
            }
            ui.label(egui::RichText::new("Courier").size(14.0).strong().color(p.text));
            ui.add_space(10.0);
            ui.label(egui::RichText::new(format!("{}  Local workspace", kit::icon::HARD_DRIVES)).color(p.text_muted))
                .on_hover_text(format!("Everything is stored in {}", self.data.root.display()));
        });
        // Centered search over every collection's requests.
        let search_w = 380.0_f32.min(full.width() * 0.34);
        let search_rect = egui::Rect::from_center_size(full.center(), egui::vec2(search_w, 30.0));
        let r = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(search_rect), |ui| {
            kit::search_field(ui, &mut self.global_search, &platform::keys("Search requests  (⌘K)"), search_w)
        });
        let search = r.inner;
        if self.focus_global_search {
            search.request_focus();
            self.focus_global_search = false;
        }
        self.global_search_results(ui, &search);
        let right = egui::Rect::from_min_max(egui::pos2(full.right() - 160.0, full.top()), full.max);
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if kit::icon_button(ui, kit::icon::GEAR, &platform::keys("Settings (⌘,)")).clicked() {
                    self.dialog = Some(Dialog::Settings(dialogs::SettingsDialog::new(&self.settings)));
                }
                let dark = ui.visuals().dark_mode;
                let (glyph, tip) = if dark {
                    (kit::icon::SUN, "Switch to light theme")
                } else {
                    (kit::icon::MOON, "Switch to dark theme")
                };
                if kit::icon_button(ui, glyph, tip).clicked() {
                    self.settings.theme =
                        if dark { crate::settings::Theme::Light } else { crate::settings::Theme::Dark };
                    theme::apply(&self.egui_ctx, self.settings.theme);
                    self.persist_settings();
                }
                if kit::icon_button(ui, kit::icon::KEYBOARD, &platform::keys("Keyboard shortcuts (⌘/)")).clicked() {
                    self.dialog = Some(Dialog::Shortcuts);
                }
            },
        );
    }

    /// Drop-down of requests matching the header search.
    fn global_search_results(&mut self, ui: &mut egui::Ui, field: &egui::Response) {
        let query = self.global_search.trim().to_lowercase();
        if query.is_empty()
            || !field.has_focus() && !ui.memory(|m| m.is_popup_open(egui::Id::new("global_search_popup")))
        {
            return;
        }
        let mut hits: Vec<(String, String, String, String, String)> = Vec::new();
        for c in &self.collections {
            for r in c.requests() {
                if r.name.to_lowercase().contains(&query) || r.request.url.to_lowercase().contains(&query) {
                    hits.push((c.id.clone(), r.id.clone(), r.request.method.clone(), r.name.clone(), c.name.clone()));
                }
            }
        }
        let p = theme::palette(ui);
        let mut open = None;
        let pos = field.rect.left_bottom() + egui::vec2(0.0, 4.0);
        egui::Area::new(egui::Id::new("global_search_area")).order(egui::Order::Foreground).fixed_pos(pos).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(field.rect.width() - 12.0);
                    if hits.is_empty() {
                        ui.label(egui::RichText::new("No requests match").color(p.text_faint));
                    }
                    for (cid, rid, method, name, coll) in hits.iter().take(12) {
                        let (resp, _) = kit::row(ui, egui::Id::new(("hit", rid)), false, 28.0, |ui| {
                            kit::method_tag(ui, method, 36.0);
                            ui.label(name);
                            ui.label(egui::RichText::new(coll).color(p.text_faint).size(11.5));
                        });
                        if resp.clicked() {
                            open = Some((cid.clone(), rid.clone()));
                        }
                    }
                });
            },
        );
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.global_search.clear();
        }
        if let Some((c, r)) = open {
            self.open_request(&c, &r);
            self.global_search.clear();
        }
    }

    fn header_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let height = 44.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.chrome);
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0_f32, p.border));
        let env_w = 240.0;
        let tabs_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - env_w - 60.0, rect.bottom()));
        let env_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - env_w - 52.0, rect.top()),
            egui::pos2(rect.right() - 10.0, rect.bottom()),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(tabs_rect), |ui| self.tab_strip(ui));
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(env_rect).layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                self.environment_quick_look(ui);
                self.environment_picker(ui, env_w);
            },
        );
    }

    fn environment_picker(&mut self, ui: &mut egui::Ui, width: f32) {
        let p = theme::palette(ui);
        let name = self.active_environment().map(|e| e.name.clone());
        let (text, color) = match &name {
            Some(n) => (n.clone(), p.text),
            None => ("No environment".to_string(), p.text_muted),
        };
        let leading = (kit::icon::GLOBE, if name.is_some() { p.accent } else { p.text_faint });
        let mut choice = None;
        let mut manage = false;
        kit::dropdown(ui, "env_picker", Some(leading), &text, Some(color), width, |ui| {
            if kit::menu_item(ui, self.active_env.is_none(), "No environment").clicked() {
                choice = Some(None);
            }
            for e in &self.environments {
                if kit::menu_item(ui, self.active_env.as_deref() == Some(e.id.as_str()), &e.name).clicked() {
                    choice = Some(Some(e.id.clone()));
                }
            }
            ui.separator();
            if kit::menu_item(ui, false, format!("{}  Manage environments", kit::icon::SLIDERS_HORIZONTAL)).clicked() {
                manage = true;
            }
        });
        if let Some(c) = choice {
            self.active_env = c;
        }
        if manage {
            self.sidebar = SidebarSection::Environments;
            self.sidebar_open = true;
        }
    }

    fn environment_quick_look(&mut self, ui: &mut egui::Ui) {
        let resp = kit::icon_button(ui, kit::icon::EYE, "Environment quick look");
        let popup = ui.make_persistent_id("env_quick_look");
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup));
        }
        egui::popup_below_widget(ui, popup, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(380.0);
            let p = theme::palette(ui);
            let mut open_env = None;
            let mut open_globals = false;
            let section = |ui: &mut egui::Ui, title: &str, vars: &[Variable]| -> bool {
                let mut edit = false;
                ui.horizontal(|ui| {
                    kit::section_label(ui, title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        edit =
                            kit::small_button(ui, kit::Kind::Ghost, Some(kit::icon::PENCIL_SIMPLE), "Edit").clicked();
                    });
                });
                if vars.iter().all(|v| !v.enabled) {
                    ui.label(egui::RichText::new("No variables").color(p.text_faint));
                }
                egui::Grid::new(("ql", title)).num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    for v in vars.iter().filter(|v| v.enabled) {
                        ui.label(egui::RichText::new(&v.key).monospace().color(p.text_muted));
                        ui.label(
                            egui::RichText::new(if v.secret {
                                "••••••••".to_string()
                            } else {
                                v.value().to_string()
                            })
                            .monospace(),
                        );
                        ui.end_row();
                    }
                });
                edit
            };
            match self.active_environment() {
                Some(e) => {
                    if section(ui, &e.name, &e.variables) {
                        open_env = Some(e.id.clone());
                    }
                }
                None => {
                    kit::section_label(ui, "Environment");
                    ui.label(egui::RichText::new("No environment selected").color(p.text_faint));
                }
            }
            ui.add_space(8.0);
            kit::divider(ui);
            ui.add_space(8.0);
            if section(ui, "Globals", &self.globals.variables) {
                open_globals = true;
            }
            if let Some(id) = open_env {
                self.open_environment(Some(&id));
                ui.memory_mut(|m| m.close_popup());
            }
            if open_globals {
                self.open_environment(None);
                ui.memory_mut(|m| m.close_popup());
            }
        });
    }

    /// Bottom bar: local status and console on the left; runner, cookies
    /// and layout switches on the right.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (dot, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.5, p.success);
            ui.label(egui::RichText::new("Local").size(11.5).color(p.text_muted))
                .on_hover_text(format!("No account or cloud. Everything is stored in {}", self.data.root.display()));
            ui.add_space(8.0);
            let errors = self.console.iter().filter(|l| l.level == ConsoleLevel::Error).count();
            let label = if errors > 0 { format!("Console ({errors})") } else { "Console".into() };
            let kind = if self.console_open { kit::Kind::Secondary } else { kit::Kind::Ghost };
            if kit::small_button(ui, kind, Some(kit::icon::TERMINAL_WINDOW), &label)
                .on_hover_text(platform::keys("⌥⌘C"))
                .clicked()
            {
                self.console_open = !self.console_open;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let side = self.settings.layout == crate::settings::Layout::SideBySide;
                if kit::icon_toggle(ui, kit::icon::COLUMNS, side, 24.0, &platform::keys("Side by side (⌥⌘V)")).clicked()
                    && !side
                {
                    self.settings.layout = crate::settings::Layout::SideBySide;
                    self.persist_settings();
                }
                if kit::icon_toggle(ui, kit::icon::ROWS, !side, 24.0, &platform::keys("Stacked (⌥⌘V)")).clicked()
                    && side
                {
                    self.settings.layout = crate::settings::Layout::Stacked;
                    self.persist_settings();
                }
                ui.add_space(6.0);
                if kit::small_button(ui, kit::Kind::Ghost, Some(kit::icon::COOKIE), "Cookies").clicked() {
                    self.dialog = Some(Dialog::Cookies(dialogs::CookieDialog::default()));
                }
                if kit::small_button(ui, kit::Kind::Ghost, Some(kit::icon::PLAY), "Runner")
                    .on_hover_text(platform::keys("⇧⌘R"))
                    .clicked()
                {
                    match self.collections.first() {
                        Some(c) => {
                            let id = c.id.clone();
                            self.open_runner(&id, None);
                        }
                        None => self.toast_error("Create or import a collection to run it"),
                    }
                }
            });
        });
    }

    fn console_panel(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            kit::section_label(ui, "Console");
            ui.add_space(8.0);
            kit::search_field(ui, &mut self.console_filter, "Filter logs", 220.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::icon_button(ui, kit::icon::X, "Close console").clicked() {
                    self.console_open = false;
                }
                if kit::small_button(ui, kit::Kind::Ghost, Some(kit::icon::TRASH), "Clear").clicked() {
                    self.console.clear();
                }
            });
        });
        ui.add_space(4.0);
        let filter = self.console_filter.to_lowercase();
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if self.console.is_empty() {
                ui.label(egui::RichText::new("Requests, script output and errors appear here.").color(p.text_faint));
            }
            for line in self.console.iter().filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter)) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(line.time.format("%H:%M:%S").to_string())
                            .monospace()
                            .size(11.0)
                            .color(p.text_faint),
                    );
                    let (tag, color) = match line.level {
                        ConsoleLevel::Error => ("ERR ", p.danger),
                        ConsoleLevel::Warn => ("WARN", p.warning),
                        ConsoleLevel::Http => ("HTTP", p.accent),
                        ConsoleLevel::Info => ("INFO", p.info),
                        ConsoleLevel::Log => ("LOG ", p.text_muted),
                    };
                    ui.label(egui::RichText::new(tag).monospace().size(11.0).color(color).strong());
                    let text_color = if line.level == ConsoleLevel::Error { p.danger } else { p.text };
                    ui.label(egui::RichText::new(&line.text).monospace().size(12.0).color(text_color));
                });
            }
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        self.header_bar(ui);
        let idx = self.active_tab;
        if idx >= self.tabs.len() {
            return;
        }
        let tab_id = self.tabs[idx].id;
        // Temporarily take the tab out so it can borrow the app mutably.
        let mut tab = std::mem::replace(&mut self.tabs[idx].kind, TabKind::Node(NodeTab::new("", None)));
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 14, bottom: 10 }).show(ui, |ui| {
            match &mut tab {
                TabKind::Request(r) => request_tab::show(self, ui, idx, tab_id, r),
                TabKind::Node(n) => tabs::show_node(self, ui, tab_id, n),
                TabKind::Environment(e) => tabs::show_env(self, ui, e),
                TabKind::Example(e) => tabs::show_example(self, ui, e),
                TabKind::Runner(r) => runner_tab::show(self, ui, tab_id, r),
            }
        });
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            t.kind = tab;
        }
        for d in std::mem::take(&mut self.deferred) {
            match d {
                Deferred::Send(i, download) => self.send(i, download),
                Deferred::Cancel(i) => self.cancel_send(i),
            }
        }
    }

    pub fn tab_title(&self, tab: &Tab) -> (String, String) {
        match &tab.kind {
            TabKind::Request(r) => (r.item.request.method.clone(), r.item.name.clone()),
            TabKind::Node(n) => {
                let name = self
                    .collection(&n.collection_id)
                    .map(|c| match n.node_id.as_deref().and_then(|id| c.find(id)) {
                        Some(i) => i.name().to_string(),
                        None => c.name.clone(),
                    })
                    .unwrap_or_default();
                (String::new(), name)
            }
            TabKind::Environment(e) => (
                String::new(),
                e.env_id
                    .as_deref()
                    .and_then(|id| self.environment(id))
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| "Globals".into()),
            ),
            TabKind::Example(e) => (String::new(), format!("{} › {}", e.request_name, e.example.name)),
            TabKind::Runner(r) => (
                String::new(),
                format!("Runner · {}", self.collection(&r.config.collection_id).map(|c| c.name.as_str()).unwrap_or("")),
            ),
        }
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let dark = p.dark;
        let mut close = None;
        let mut activate = None;
        let mut duplicate = None;
        let mut moved: Option<(usize, usize)> = None;
        egui::ScrollArea::horizontal()
            .id_salt("tab_strip")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(8.0);
                    for i in 0..self.tabs.len() {
                        let (method, title) = self.tab_title(&self.tabs[i]);
                        let dirty = self.tab_is_dirty(i);
                        let active = i == self.active_tab;
                        let tab_id = self.tabs[i].id;
                        let glyph = match &self.tabs[i].kind {
                            TabKind::Request(_) => None,
                            TabKind::Node(_) => Some(kit::icon::STACK),
                            TabKind::Environment(_) => Some(kit::icon::GLOBE),
                            TabKind::Example(_) => Some(kit::icon::BOOKMARK_SIMPLE),
                            TabKind::Runner(_) => Some(kit::icon::PLAY),
                        };
                        let mut t = title.clone();
                        if t.chars().count() > 26 {
                            t = t.chars().take(25).collect::<String>() + "…";
                        }
                        let font = egui::FontId::proportional(13.0);
                        let title_g = ui.painter().layout_no_wrap(t, font, p.text);
                        let lead_w = if !method.is_empty() {
                            38.0
                        } else if glyph.is_some() {
                            22.0
                        } else {
                            0.0
                        };
                        let width = (14.0 + lead_w + title_g.size().x + 30.0).max(110.0);
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(width, 44.0), egui::Sense::click_and_drag());
                        resp.dnd_set_drag_payload(i);
                        // Bring a newly activated tab into view.
                        let scroll_key = egui::Id::new("tab_strip_scrolled_to");
                        if active && ui.data(|d| d.get_temp::<u64>(scroll_key)) != Some(tab_id) {
                            ui.scroll_to_rect(rect, None);
                            ui.data_mut(|d| d.insert_temp(scroll_key, tab_id));
                        }
                        let hovered = resp.hovered() || resp.contains_pointer();
                        let tab_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 2.0, rect.top() + 6.0),
                            egui::pos2(rect.right() - 2.0, rect.bottom()),
                        );
                        if active {
                            ui.painter().rect(
                                tab_rect,
                                egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 },
                                p.canvas,
                                egui::Stroke::NONE,
                                egui::StrokeKind::Inside,
                            );
                            let line = egui::Rect::from_min_max(
                                tab_rect.left_top(),
                                egui::pos2(tab_rect.right(), tab_rect.top() + 2.0),
                            );
                            ui.painter().rect_filled(line, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.accent);
                            // Hide the header's bottom border under the active tab.
                            ui.painter().hline(
                                tab_rect.x_range(),
                                rect.bottom() - 0.5,
                                egui::Stroke::new(1.0_f32, p.canvas),
                            );
                        } else if hovered {
                            ui.painter().rect_filled(tab_rect.shrink2(egui::vec2(0.0, 3.0)), 7.0, p.surface_hover);
                        }
                        let cy = tab_rect.center().y;
                        let mut x = tab_rect.left() + 12.0;
                        if !method.is_empty() {
                            ui.painter().text(
                                egui::pos2(x, cy),
                                egui::Align2::LEFT_CENTER,
                                theme::method_short(&method),
                                egui::FontId::monospace(10.5),
                                theme::method_color(&method, dark),
                            );
                            x += lead_w;
                        } else if let Some(g) = glyph {
                            ui.painter().text(
                                egui::pos2(x, cy),
                                egui::Align2::LEFT_CENTER,
                                g,
                                egui::FontId::proportional(14.0),
                                if active { p.accent } else { p.text_muted },
                            );
                            x += lead_w;
                        }
                        let text_color = if active { p.text } else { p.text_muted };
                        ui.painter().galley(egui::pos2(x, cy - title_g.size().y / 2.0), title_g, text_color);
                        // Close button, or a dirty dot when not hovered.
                        let close_rect = egui::Rect::from_center_size(
                            egui::pos2(tab_rect.right() - 16.0, cy),
                            egui::vec2(20.0, 20.0),
                        );
                        let close_resp =
                            ui.interact(close_rect, egui::Id::new(("tab_close", tab_id)), egui::Sense::click());
                        if hovered || active || close_resp.hovered() {
                            if close_resp.hovered() {
                                ui.painter().rect_filled(close_rect, 5.0, p.surface_active);
                            }
                            let show_dot = dirty && !hovered && !close_resp.hovered();
                            if show_dot {
                                ui.painter().circle_filled(close_rect.center(), 4.0, p.accent);
                            } else {
                                ui.painter().text(
                                    close_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    kit::icon::X,
                                    egui::FontId::proportional(12.0),
                                    p.text_muted,
                                );
                            }
                        } else if dirty {
                            ui.painter().circle_filled(close_rect.center(), 4.0, p.accent);
                        }
                        if close_resp.on_hover_text(platform::keys("Close (⌘W)")).clicked() {
                            close = Some(i);
                        } else if resp.clicked() {
                            activate = Some(i);
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                        if let Some(from) = resp.dnd_release_payload::<usize>() {
                            moved = Some((*from, i));
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Close").clicked() {
                                close = Some(i);
                                ui.close_menu();
                            }
                            if matches!(self.tabs[i].kind, TabKind::Request(_)) && ui.button("Duplicate").clicked() {
                                duplicate = Some(i);
                                ui.close_menu();
                            }
                        });
                    }
                    ui.add_space(6.0);
                    if kit::icon_button(ui, kit::icon::PLUS, &platform::keys("New request (⌘T)")).clicked() {
                        self.new_request_tab();
                    }
                });
            });
        if let Some((from, to)) = moved
            && from != to
            && from < self.tabs.len()
        {
            let t = self.tabs.remove(from);
            let to = to.min(self.tabs.len());
            self.tabs.insert(to, t);
            self.active_tab = to;
        }
        if let Some(i) = duplicate {
            if let TabKind::Request(r) = &self.tabs[i].kind {
                let mut item = r.item.clone();
                item.id = new_id();
                self.push_tab(TabKind::Request(Box::new(RequestTab::new(None, item, None))));
            }
        } else if let Some(a) = activate {
            self.active_tab = a;
        }
        if let Some(c) = close {
            self.request_close_tab(c);
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        let mut undo = None;
        let mut dismiss = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -44.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let p = theme::palette(ui);
                for (i, t) in self.toasts.iter().enumerate() {
                    egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0_f32, p.border_strong))
                        .corner_radius(10)
                        .shadow(ui.visuals().popup_shadow)
                        .inner_margin(egui::Margin { left: 12, right: 8, top: 8, bottom: 8 })
                        .show(ui, |ui| {
                            ui.set_max_width(400.0);
                            ui.horizontal(|ui| {
                                let (glyph, color) = if t.error {
                                    (kit::icon::WARNING_CIRCLE, p.danger)
                                } else {
                                    (kit::icon::CHECK_CIRCLE, p.accent)
                                };
                                ui.label(egui::RichText::new(glyph).size(17.0).color(color));
                                ui.label(&t.text);
                                if t.undo.is_some()
                                    && kit::small_button(ui, kit::Kind::Secondary, None, "Undo").clicked()
                                {
                                    undo = Some(i);
                                }
                                if kit::icon_button(ui, kit::icon::X, "Dismiss").clicked() {
                                    dismiss = Some(i);
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = undo {
            let t = self.toasts.remove(i);
            if let Some(u) = t.undo {
                self.undo(u);
            }
        } else if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
    }
}

/// One item of the navigation rail: icon above a tiny caption.
fn rail_item(ui: &mut egui::Ui, glyph: &str, label: &str, selected: bool) -> egui::Response {
    let p = theme::palette(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
    let hovered = resp.hovered();
    if selected {
        ui.painter().rect_filled(rect, 10.0, p.accent_soft);
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() - 8.0, rect.top() + 12.0),
            egui::pos2(rect.left() - 5.0, rect.bottom() - 12.0),
        );
        ui.painter().rect_filled(bar, 2.0, p.accent);
    } else if hovered {
        ui.painter().rect_filled(rect, 10.0, p.surface_hover);
    }
    let color = if selected {
        p.accent
    } else if hovered {
        p.text
    } else {
        p.text_muted
    };
    ui.painter().text(
        rect.center() - egui::vec2(0.0, 7.0),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(19.0),
        color,
    );
    ui.painter().text(
        rect.center() + egui::vec2(0.0, 13.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(9.5),
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl eframe::App for CourierApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}

pub fn sanitize_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "export".into() } else { s }
}
