//! Executes one request end to end: pre-request scripts, variable
//! resolution, auth (with OAuth 2.0 refresh and Digest challenges), the HTTP
//! send, cookies and post-response scripts.

use crate::auth::{self, AuthContext, oauth2};
use crate::cookies::CookieJar;
use crate::http::{CookieStore, HttpEngine, HttpResponse, PreparedRequest, prepare};
use crate::model::{Auth, AuthType, Collection, Request, RequestItem};
use crate::script::{
    self, ConsoleEntry, Event, NextRequest, ScriptHost, ScriptInfo, ScriptInput, SendSpec, TestResult,
};
use crate::settings::Settings;
use crate::vars::VarContext;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The outcome of executing one request.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    /// The request after pre-request scripts ran (still unresolved).
    pub request: Request,
    /// What was actually sent (resolved, signed).
    pub sent: Option<PreparedRequest>,
    pub response: Option<HttpResponse>,
    /// Network, auth or pre-request script failure.
    pub error: Option<String>,
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleEntry>,
    /// Script errors from the pre-request and post-response phases.
    pub script_errors: Vec<String>,
    pub vars: VarContext,
    pub next_request: NextRequest,
    pub skipped: bool,
    pub auth_owner: String,
    /// A token obtained by auto-refresh, to be stored by the caller.
    pub refreshed_token: Option<oauth2::OAuth2Token>,
    pub visualizer: Option<serde_json::Value>,
}

impl Execution {
    pub fn passed_tests(&self) -> usize {
        self.tests.iter().filter(|t| t.passed).count()
    }

    pub fn failed_tests(&self) -> usize {
        self.tests.iter().filter(|t| !t.passed && !t.skipped).count()
    }
}

/// Per-execution inputs beyond the request itself.
#[derive(Debug, Clone, Default)]
pub struct ExecOptions {
    pub environment_name: String,
    pub iteration: usize,
    pub iteration_count: usize,
    pub download_to: Option<PathBuf>,
}

/// Console line written by Courier itself (not by a script).
fn note(level: &str, message: impl Into<String>) -> ConsoleEntry {
    ConsoleEntry { level: level.into(), message: message.into() }
}

/// The auth that applies to `req`, walking up the collection when it
/// inherits. Returns the auth and the name of the node that owns it.
pub fn effective_auth(item: &RequestItem, collection: Option<&Collection>) -> (Auth, String) {
    if item.request.auth.kind != AuthType::Inherit {
        return (item.request.auth.clone(), item.name.clone());
    }
    let Some(c) = collection else { return (Auth::none(), String::new()) };
    for f in c.ancestors(&item.id).unwrap_or_default().iter().rev() {
        if f.auth.kind != AuthType::Inherit {
            return (f.auth.clone(), f.name.clone());
        }
    }
    let auth = if c.auth.kind == AuthType::Inherit { Auth::none() } else { c.auth.clone() };
    (auth, c.name.clone())
}

/// Script chains (outermost first) for a possibly-unsaved item.
pub fn script_chain(item: &RequestItem, collection: Option<&Collection>) -> (Vec<String>, Vec<String>) {
    let mut pre = Vec::new();
    let mut post = Vec::new();
    if let Some(c) = collection {
        pre.push(c.scripts.pre_request.clone());
        post.push(c.scripts.post_response.clone());
        for f in c.ancestors(&item.id).unwrap_or_default() {
            pre.push(f.scripts.pre_request.clone());
            post.push(f.scripts.post_response.clone());
        }
    }
    pre.push(item.scripts.pre_request.clone());
    post.push(item.scripts.post_response.clone());
    pre.retain(|s| !s.trim().is_empty());
    post.retain(|s| !s.trim().is_empty());
    (pre, post)
}

/// Shared executor: HTTP engine, cookie jar and settings.
#[derive(Clone)]
pub struct Executor {
    pub http: HttpEngine,
    pub jar: Arc<CookieJar>,
    token_client: reqwest::Client,
}

impl Executor {
    pub fn new(settings: Settings, jar: Arc<CookieJar>) -> Self {
        Self { http: HttpEngine::new(settings), jar, token_client: reqwest::Client::new() }
    }

    fn script_host(&self, vars: &VarContext) -> ScriptHost {
        let http = self.http.clone();
        let jar = self.jar.clone();
        let settings = self.http.settings();
        let vars = vars.clone();
        let handle = tokio::runtime::Handle::current();
        let send_jar = jar.clone();
        ScriptHost {
            send: Arc::new(move |spec: SendSpec| {
                let prepared = prepare(&spec.to_request(), &vars, &settings);
                let cancel = CancellationToken::new();
                handle
                    .block_on(http.send(&prepared, Some(send_jar.as_ref() as &dyn CookieStore), &cancel))
                    .map_err(|e| e.to_string())
            }),
            jar: Some(jar),
            timeout: Duration::from_millis(self.http.settings().script_timeout_ms.max(100)),
        }
    }

    async fn run_scripts(&self, scripts: Vec<String>, event: Event, input: ScriptInput) -> script::ScriptResult {
        if scripts.is_empty() {
            return script::ScriptResult {
                request: input.request.clone(),
                vars: input.vars.clone(),
                ..Default::default()
            };
        }
        let host = self.script_host(&input.vars);
        tokio::task::spawn_blocking(move || script::run(&scripts, event, &input, &host)).await.unwrap_or_else(|e| {
            script::ScriptResult { error: Some(format!("script crashed: {e}")), ..Default::default() }
        })
    }

    /// Execute `item` in the context of `collection` (if any).
    pub async fn execute(
        &self,
        item: &RequestItem,
        collection: Option<&Collection>,
        vars: VarContext,
        opts: &ExecOptions,
        cancel: &CancellationToken,
    ) -> Execution {
        let settings = self.http.settings();
        let (pre_scripts, post_scripts) = script_chain(item, collection);
        let info = ScriptInfo {
            request_name: item.name.clone(),
            request_id: item.id.clone(),
            iteration: opts.iteration,
            iteration_count: opts.iteration_count.max(1),
            event_name: String::new(),
        };
        let mut exec = Execution { request: item.request.clone(), vars, ..Default::default() };

        // 1. Pre-request scripts.
        if !pre_scripts.is_empty() {
            let input = ScriptInput {
                request: exec.request.clone(),
                response: None,
                cookies: Vec::new(),
                vars: exec.vars.clone(),
                environment_name: opts.environment_name.clone(),
                info: info.clone(),
            };
            let r = self.run_scripts(pre_scripts, Event::PreRequest, input).await;
            exec.tests.extend(r.tests);
            exec.console.extend(r.console);
            exec.request = r.request;
            exec.vars = r.vars;
            exec.next_request = r.next_request;
            if let Some(e) = r.error {
                exec.script_errors.push(format!("Pre-request script: {e}"));
                exec.error = Some(format!("There was an error in the pre-request script: {e}"));
                return exec;
            }
            if r.skip_request {
                exec.skipped = true;
                exec.console.push(note("info", format!("{} was skipped by a pre-request script", item.name)));
                return exec;
            }
        }

        // 2. Resolve and prepare.
        let scripted_item = RequestItem { request: exec.request.clone(), ..item.clone() };
        let (raw_auth, owner) = effective_auth(&scripted_item, collection);
        exec.auth_owner = owner;
        let mut auth = auth::resolve(&raw_auth, &exec.vars);
        if auth.kind == AuthType::OAuth2 && oauth2::needs_refresh(&auth, chrono::Utc::now()) {
            match oauth2::refresh(&self.token_client, &auth).await {
                Ok(token) => {
                    oauth2::use_token(&mut auth, &token);
                    exec.console.push(note("info", "OAuth 2.0 access token refreshed"));
                    exec.refreshed_token = Some(token);
                }
                Err(e) => exec.console.push(note("warn", format!("OAuth 2.0 token refresh failed: {e}"))),
            }
        }
        if !auth.kind.is_supported() {
            exec.console
                .push(note("warn", format!("{} is not supported; the request is sent without it", auth.kind.label())));
        }
        let mut prepared = prepare(&exec.request, &exec.vars, &settings);
        prepared.options.download_to = opts.download_to.clone();
        let ctx = AuthContext::default();
        if auth.kind.is_supported()
            && let Err(e) = auth::apply(&auth, &mut prepared, &ctx)
        {
            exec.error = Some(format!("Authorization failed: {e}"));
            return exec;
        }
        let (_, missing) = exec.vars.resolve_report(&exec.request.url);
        for name in missing {
            exec.console.push(note("warn", format!("Unresolved variable {{{{{name}}}}} in URL")));
        }

        // 3. Send (retrying once for a Digest challenge).
        let jar: &dyn CookieStore = self.jar.as_ref();
        let mut result = self.http.send(&prepared, Some(jar), cancel).await;
        let challenge = match &result {
            Ok(res) if res.status == 401 && auth.kind == AuthType::Digest && !auth.get_bool("disableRetryRequest") => {
                res.header_all("www-authenticate").find_map(auth::digest::Challenge::parse)
            }
            _ => None,
        };
        if let Some(ch) = challenge {
            match auth::digest::authorization(&auth, &ch, &prepared, &ctx) {
                Ok(h) => {
                    prepared.set_header("Authorization", h);
                    result = self.http.send(&prepared, Some(jar), cancel).await;
                }
                Err(e) => exec.console.push(note("warn", format!("Digest auth: {e}"))),
            }
        }
        exec.console.push(note(
            "http",
            match &result {
                Ok(r) => format!(
                    "{} {} → {} {} ({:.0} ms)",
                    prepared.method, prepared.url, r.status, r.status_text, r.timings.total_ms
                ),
                Err(e) => format!("{} {} → {e}", prepared.method, prepared.url),
            },
        ));
        exec.sent = Some(prepared);
        let response = match result {
            Ok(r) => r,
            Err(e) => {
                exec.error = Some(e.to_string());
                return exec;
            }
        };

        // 4. Post-response scripts.
        if !post_scripts.is_empty() {
            let cookies = url::Url::parse(&response.final_url).map(|u| self.jar.matching(&u)).unwrap_or_default();
            let input = ScriptInput {
                request: exec.request.clone(),
                response: Some(response.clone()),
                cookies,
                vars: exec.vars.clone(),
                environment_name: opts.environment_name.clone(),
                info,
            };
            let r = self.run_scripts(post_scripts, Event::Test, input).await;
            exec.tests.extend(r.tests);
            exec.console.extend(r.console);
            exec.vars = r.vars;
            if r.next_request != NextRequest::Unset {
                exec.next_request = r.next_request;
            }
            exec.visualizer = r.visualizer;
            if let Some(e) = r.error {
                exec.script_errors.push(format!("Post-response script: {e}"));
            }
        }
        exec.response = Some(response);
        exec
    }
}
