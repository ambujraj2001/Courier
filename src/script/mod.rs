//! Pre-request and post-response scripts run in an embedded QuickJS
//! sandbox. The `pm` API itself is written in JavaScript (`prelude.js`);
//! this module passes state in and out as JSON and provides a few native
//! helpers (HTTP, cookies, hashing).

use crate::cookies::{Cookie, CookieJar};
use crate::http::HttpResponse;
use crate::model::{Request, Variable};
use crate::vars::VarContext;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rquickjs::{CatchResultExt, Context, Function, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PRELUDE: &str = include_str!("prelude.js");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    #[serde(rename = "prerequest")]
    PreRequest,
    #[serde(rename = "test")]
    Test,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Event::PreRequest => "prerequest",
            Event::Test => "test",
        }
    }
}

/// `pm.info` contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScriptInfo {
    pub request_name: String,
    pub request_id: String,
    pub iteration: usize,
    pub iteration_count: usize,
    pub event_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub event: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub message: String,
}

/// What `pm.execution.setNextRequest` asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum NextRequest {
    #[default]
    Unset,
    Stop,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptResult {
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleEntry>,
    /// The request after pre-request scripts modified it.
    pub request: Request,
    pub vars: VarContext,
    pub next_request: NextRequest,
    pub skip_request: bool,
    pub visualizer: Option<Value>,
    /// Script error (syntax error, uncaught exception or timeout).
    pub error: Option<String>,
}

/// Request spec passed to the host by `pm.sendRequest`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SendSpec {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub header: Vec<SendHeader>,
    #[serde(default)]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SendHeader {
    pub key: String,
    pub value: String,
}

impl SendSpec {
    /// Convert to a Courier request definition.
    pub fn to_request(&self) -> Request {
        use crate::model::{Body, BodyMode, KeyValue, RawLanguage};
        let mut req = Request::new(&self.method, &self.url);
        req.headers = self.header.iter().map(|h| KeyValue::new(&h.key, &h.value)).collect();
        if let Some(body) = &self.body {
            match body.get("mode").and_then(Value::as_str) {
                Some("raw") => {
                    let raw = body.get("raw").and_then(Value::as_str).unwrap_or_default();
                    let lang =
                        if serde_json::from_str::<Value>(raw).is_ok() { RawLanguage::Json } else { RawLanguage::Text };
                    req.body = Body::raw(raw, lang);
                }
                Some("urlencoded") => {
                    req.body.mode = BodyMode::Urlencoded;
                    req.body.urlencoded = body["urlencoded"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|kv| {
                                    KeyValue::new(
                                        kv["key"].as_str().unwrap_or_default(),
                                        kv["value"].as_str().unwrap_or_default(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                _ => {}
            }
        }
        req
    }
}

pub type SendFn = Arc<dyn Fn(SendSpec) -> Result<HttpResponse, String> + Send + Sync>;

/// Capabilities the sandbox can call back into.
#[derive(Clone)]
pub struct ScriptHost {
    pub send: SendFn,
    pub jar: Option<Arc<CookieJar>>,
    pub timeout: Duration,
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self {
            send: Arc::new(|_| Err("pm.sendRequest is not available here".into())),
            jar: None,
            timeout: Duration::from_secs(10),
        }
    }
}

/// Everything a script chain needs.
#[derive(Debug, Clone, Default)]
pub struct ScriptInput {
    pub request: Request,
    pub response: Option<HttpResponse>,
    /// Cookies matching the request URL (for `pm.cookies`).
    pub cookies: Vec<Cookie>,
    pub vars: VarContext,
    pub environment_name: String,
    pub info: ScriptInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsResponse {
    code: u16,
    status: String,
    headers: Vec<(String, String)>,
    body: String,
    response_time: f64,
    response_size: u64,
    cookies: Vec<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsVars<'a> {
    local: &'a BTreeMap<String, String>,
    data: &'a BTreeMap<String, String>,
    environment: &'a Option<Vec<Variable>>,
    environment_name: &'a str,
    collection: &'a Vec<Variable>,
    globals: &'a Vec<Variable>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsInput<'a> {
    event: &'static str,
    request: &'a Request,
    response: Option<JsResponse>,
    variables: JsVars<'a>,
    info: ScriptInfo,
    cookies: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsOutput {
    tests: Vec<TestResult>,
    console: Vec<ConsoleEntry>,
    next_set: bool,
    next: Option<String>,
    skip: bool,
    visualizer: Option<Value>,
    request: Request,
    variables: JsOutVars,
}

#[derive(Deserialize)]
struct JsOutVars {
    local: BTreeMap<String, Value>,
    environment: Option<Vec<Variable>>,
    collection: Vec<Variable>,
    globals: Vec<Variable>,
}

fn response_for_script(res: &HttpResponse, cookies: &[Cookie]) -> JsResponse {
    JsResponse {
        code: res.status,
        status: res.status_text.clone(),
        headers: res.headers.clone(),
        body: res.text(),
        response_time: res.timings.total_ms.round(),
        response_size: res.body_size,
        cookies: cookies.iter().map(|c| serde_json::json!({ "name": c.name, "value": c.value })).collect(),
    }
}

fn hash_hex(alg: &str, data: &str, key: Option<String>) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Digest;
    let bytes = data.as_bytes();
    macro_rules! digest {
        ($h:ty) => {
            match key {
                Some(k) => {
                    let mut m = <Hmac<$h> as Mac>::new_from_slice(k.as_bytes()).expect("any key length");
                    m.update(bytes);
                    crate::auth::hex(&m.finalize().into_bytes())
                }
                None => crate::auth::hex(&<$h>::digest(bytes)),
            }
        };
    }
    match alg {
        "md5" => digest!(md5::Md5),
        "sha1" => digest!(sha1::Sha1),
        "sha256" => digest!(sha2::Sha256),
        "sha512" => digest!(sha2::Sha512),
        _ => crate::auth::hex(bytes),
    }
}

fn from_hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

fn b64(mode: &str, data: &str) -> String {
    match mode {
        "encode" => B64.encode(data.as_bytes()),
        "decode" => B64.decode(data.trim()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
        "hex2b64" => B64.encode(from_hex(data)),
        "b642hex" => B64.decode(data.trim()).map(|b| crate::auth::hex(&b)).unwrap_or_default(),
        _ => String::new(),
    }
}

fn jar_op(jar: &Option<Arc<CookieJar>>, json: &str) -> String {
    #[derive(Deserialize)]
    struct Op {
        op: String,
        url: String,
        name: Option<String>,
        value: Option<String>,
    }
    let result = (|| -> Result<Value, String> {
        let jar = jar.as_ref().ok_or("the cookie jar is not available")?;
        let op: Op = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let url = url::Url::parse(&crate::http::prepare::ensure_scheme(&op.url)).map_err(|e| e.to_string())?;
        let host = url.host_str().unwrap_or_default().to_string();
        if !jar.script_allowed(&host) {
            return Err(format!("domain {host} is not in the cookie allowlist for scripts"));
        }
        let name = op.name.unwrap_or_default();
        Ok(match op.op.as_str() {
            "get" => jar
                .matching(&url)
                .into_iter()
                .find(|c| c.name == name)
                .map(|c| Value::String(c.value))
                .unwrap_or(Value::Null),
            "getAll" => Value::Array(
                jar.matching(&url)
                    .into_iter()
                    .map(
                        |c| serde_json::json!({ "name": c.name, "value": c.value, "domain": c.domain, "path": c.path }),
                    )
                    .collect(),
            ),
            "set" => {
                let mut c = Cookie::new(&name, &op.value.unwrap_or_default(), &host);
                c.host_only = true;
                jar.upsert(c);
                Value::Null
            }
            "unset" => {
                for c in jar.matching(&url).into_iter().filter(|c| c.name == name) {
                    jar.remove(&c.domain, &c.name, &c.path);
                }
                Value::Null
            }
            "clear" => {
                for c in jar.matching(&url) {
                    jar.remove(&c.domain, &c.name, &c.path);
                }
                Value::Null
            }
            other => return Err(format!("unknown cookie jar operation {other}")),
        })
    })();
    match result {
        Ok(v) => serde_json::json!({ "result": v }).to_string(),
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    }
}

fn to_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Run a chain of scripts (outermost first) for one event.
pub fn run(scripts: &[String], event: Event, input: &ScriptInput, host: &ScriptHost) -> ScriptResult {
    let mut result = ScriptResult { request: input.request.clone(), vars: input.vars.clone(), ..Default::default() };
    if scripts.iter().all(|s| s.trim().is_empty()) {
        return result;
    }
    match run_inner(scripts, event, input, host) {
        Ok((out, error)) => {
            result.tests = out.tests;
            result.console = out.console;
            result.request = out.request;
            if result.request.url != input.request.url {
                result.request.path_variables =
                    crate::model::url::merge_path_variables(&input.request.path_variables, &result.request.url);
            }
            result.next_request = match (out.next_set, out.next) {
                (false, _) => NextRequest::Unset,
                (true, None) => NextRequest::Stop,
                (true, Some(n)) => NextRequest::Named(n),
            };
            result.skip_request = out.skip;
            result.visualizer = out.visualizer;
            result.vars.local = out.variables.local.iter().map(|(k, v)| (k.clone(), to_value(v))).collect();
            if input.vars.environment.is_some() {
                result.vars.environment = out.variables.environment;
            }
            result.vars.collection = out.variables.collection;
            result.vars.globals = out.variables.globals;
            result.error = error;
        }
        Err(e) => result.error = Some(e),
    }
    result
}

fn run_inner(
    scripts: &[String],
    event: Event,
    input: &ScriptInput,
    host: &ScriptHost,
) -> Result<(JsOutput, Option<String>), String> {
    let js_input = JsInput {
        event: event.name(),
        request: &input.request,
        response: input.response.as_ref().map(|r| response_for_script(r, &input.cookies)),
        variables: JsVars {
            local: &input.vars.local,
            data: &input.vars.data,
            environment: &input.vars.environment,
            environment_name: &input.environment_name,
            collection: &input.vars.collection,
            globals: &input.vars.globals,
        },
        info: ScriptInfo { event_name: event.name().into(), ..input.info.clone() },
        cookies: input.cookies.iter().map(|c| serde_json::json!({ "name": c.name, "value": c.value })).collect(),
    };
    let input_json = serde_json::to_string(&js_input).map_err(|e| e.to_string())?;

    let rt = Runtime::new().map_err(|e| format!("script engine: {e}"))?;
    rt.set_memory_limit(64 * 1024 * 1024);
    rt.set_max_stack_size(1024 * 1024);
    let deadline = Instant::now() + host.timeout;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let ctx = Context::full(&rt).map_err(|e| format!("script engine: {e}"))?;
    let timed_out = || Instant::now() > deadline;
    let timeout_msg = format!("Script timed out after {} ms", host.timeout.as_millis());

    ctx.with(|ctx| -> Result<(), String> {
        let g = ctx.globals();
        let err = |e: rquickjs::Error| e.to_string();
        g.set("__courier_input", input_json).map_err(err)?;
        let send = host.send.clone();
        g.set(
            "__courier_send",
            Function::new(ctx.clone(), move |json: String| -> String {
                let out =
                    serde_json::from_str::<SendSpec>(&json).map_err(|e| e.to_string()).and_then(|spec| send(spec));
                match out {
                    Ok(res) => serde_json::json!({ "response": response_for_script(&res, &[]) }).to_string(),
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                }
            })
            .map_err(err)?,
        )
        .map_err(err)?;
        let jar = host.jar.clone();
        g.set("__courier_jar", Function::new(ctx.clone(), move |json: String| jar_op(&jar, &json)).map_err(err)?)
            .map_err(err)?;
        g.set(
            "__courier_hash",
            Function::new(ctx.clone(), |alg: String, data: String, key: Option<String>| hash_hex(&alg, &data, key))
                .map_err(err)?,
        )
        .map_err(err)?;
        g.set(
            "__courier_b64",
            Function::new(ctx.clone(), |mode: String, data: String| b64(&mode, &data)).map_err(err)?,
        )
        .map_err(err)?;
        g.set(
            "__courier_dynamic",
            Function::new(ctx.clone(), |name: String| crate::vars::dynamic::generate(&name)).map_err(err)?,
        )
        .map_err(err)?;
        g.set("__courier_uuid", Function::new(ctx.clone(), || uuid::Uuid::new_v4().to_string()).map_err(err)?)
            .map_err(err)?;
        ctx.eval::<(), _>(PRELUDE).catch(&ctx).map_err(|e| format!("script prelude failed: {e}"))?;
        Ok(())
    })?;

    let mut error = None;
    for (i, script) in scripts.iter().enumerate() {
        if script.trim().is_empty() {
            continue;
        }
        // Keep user line numbers intact: the wrapper starts on line 1.
        let wrapped =
            format!("try {{ (function () {{ {script}\n}}).call(globalThis); }} catch (__e) {{ __courier_fail(__e); }}");
        let res = ctx.with(|ctx| match ctx.eval::<(), _>(wrapped.as_bytes()).catch(&ctx) {
            Ok(()) => match ctx.eval::<String, _>("__courier_take_error()") {
                Ok(e) if !e.is_empty() => Err(e),
                _ => Ok(()),
            },
            Err(e) => {
                // Uncatchable (timeout) or a parse error: describe it as best we can.
                let raw = e.to_string();
                let line = raw.split("eval_script:").nth(1).and_then(|r| r.split(':').next()).map(str::to_string);
                let check = ctx
                    .globals()
                    .get::<_, Function>("__courier_syntax_check")
                    .and_then(|f| f.call::<_, String>((script.as_str(),)));
                Err(match (check, line) {
                    (Ok(msg), Some(line)) if !msg.is_empty() => format!("{msg} (line {line})"),
                    (Ok(msg), None) if !msg.is_empty() => msg,
                    _ => raw.lines().next().unwrap_or_default().to_string(),
                })
            }
        });
        // Run promise jobs and timers until both queues are empty.
        loop {
            while rt.is_job_pending() {
                if rt.execute_pending_job().is_err() || timed_out() {
                    break;
                }
            }
            if timed_out() {
                break;
            }
            let more = ctx.with(|ctx| ctx.eval::<bool, _>("__courier_drain_timers()").unwrap_or(false));
            if !more && !rt.is_job_pending() {
                break;
            }
        }
        let _ = ctx.with(|ctx| ctx.eval::<(), _>("__courier_collect_legacy()"));
        if let Err(e) = res {
            let label =
                if scripts.len() > 1 { format!(" (script {} of {})", i + 1, scripts.len()) } else { String::new() };
            error = Some(if timed_out() { timeout_msg.clone() } else { format!("{}{label}", e.trim()) });
            break;
        }
        if timed_out() {
            error = Some(timeout_msg.clone());
            break;
        }
    }

    // Reading the output must not be cut short by the interrupt handler.
    rt.set_interrupt_handler(None);
    let json: String = ctx
        .with(|ctx| ctx.eval::<String, _>("__courier_output()").catch(&ctx).map_err(|e| e.to_string()))
        .map_err(|e| format!("could not read script results: {e}"))?;
    let out: JsOutput = serde_json::from_str(&json).map_err(|e| format!("could not read script results: {e}"))?;
    Ok((out, error))
}

#[cfg(test)]
mod tests;
