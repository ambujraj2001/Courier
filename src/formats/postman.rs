//! Postman Collection Format v2.1 / v2.0, environment and globals files.
//!
//! Implemented from the public collection JSON schema. Unknown fields are
//! preserved in the model's `extra` maps and written back on export.

use crate::model::*;
use serde_json::{Map, Value, json};
use thiserror::Error;

pub const SCHEMA_V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
pub const EXPORTED_USING: &str = concat!("Courier/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Postman collection: {0}")]
    NotACollection(String),
    #[error("not an environment file: {0}")]
    NotAnEnvironment(String),
}

type Result<T> = std::result::Result<T, FormatError>;

// ---------------------------------------------------------------- helpers

fn take_str(obj: &mut Map<String, Value>, key: &str) -> String {
    match obj.remove(key) {
        Some(Value::String(s)) => s,
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Descriptions may be a string or `{ content, type }`.
fn take_description(obj: &mut Map<String, Value>) -> String {
    match obj.remove("description") {
        Some(Value::String(s)) => s,
        Some(Value::Object(mut d)) => take_str(&mut d, "content"),
        _ => String::new(),
    }
}

fn kv_from(v: &Value) -> KeyValue {
    let o = v.as_object();
    let get = |k: &str| o.and_then(|o| o.get(k)).map(value_to_string).unwrap_or_default();
    KeyValue {
        key: get("key"),
        value: get("value"),
        enabled: !o.and_then(|o| o.get("disabled")).and_then(Value::as_bool).unwrap_or(false),
        description: o
            .and_then(|o| o.get("description"))
            .map(|d| match d {
                Value::Object(m) => m.get("content").map(value_to_string).unwrap_or_default(),
                other => value_to_string(other),
            })
            .unwrap_or_default(),
    }
}

fn kv_list(v: Option<Value>) -> Vec<KeyValue> {
    match v {
        Some(Value::Array(a)) => a.iter().map(kv_from).collect(),
        Some(Value::String(s)) => {
            s.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| KeyValue::new(k.trim(), v.trim())).collect()
        }
        _ => Vec::new(),
    }
}

fn kv_to(kv: &KeyValue, with_type: bool) -> Value {
    let mut o = Map::new();
    o.insert("key".into(), kv.key.clone().into());
    o.insert("value".into(), kv.value.clone().into());
    if with_type {
        o.insert("type".into(), "text".into());
    }
    if !kv.enabled {
        o.insert("disabled".into(), true.into());
    }
    if !kv.description.is_empty() {
        o.insert("description".into(), kv.description.clone().into());
    }
    Value::Object(o)
}

// ---------------------------------------------------------------- auth

fn auth_from(v: Option<Value>) -> Option<Auth> {
    let mut obj = match v? {
        Value::Object(o) => o,
        _ => return None,
    };
    let kind_name = take_str(&mut obj, "type");
    let kind = AuthType::from_postman(&kind_name);
    let mut params = JsonMap::new();
    match obj.remove(&kind_name) {
        // v2.1: [{ key, value, type }]
        Some(Value::Array(list)) => {
            for p in list {
                if let Some(key) = p.get("key").and_then(Value::as_str) {
                    params.insert(key.to_string(), p.get("value").cloned().unwrap_or(Value::Null));
                }
            }
        }
        // v2.0: { key: value }
        Some(Value::Object(map)) => params = map,
        _ => {}
    }
    Some(Auth { kind, params })
}

fn auth_to(auth: &Auth) -> Value {
    let name = auth.kind.postman_name().to_string();
    let mut o = Map::new();
    o.insert("type".into(), name.clone().into());
    if !auth.params.is_empty() {
        let list: Vec<Value> = auth
            .params
            .iter()
            .map(|(k, v)| {
                let ty = match v {
                    Value::String(_) => "string",
                    Value::Bool(_) => "boolean",
                    _ => "any",
                };
                json!({ "key": k, "value": v, "type": ty })
            })
            .collect();
        o.insert(name, Value::Array(list));
    }
    Value::Object(o)
}

// ---------------------------------------------------------------- scripts

fn scripts_from(v: Option<Value>) -> Scripts {
    let mut scripts = Scripts::default();
    let Some(Value::Array(events)) = v else { return scripts };
    for ev in events {
        let listen = ev.get("listen").and_then(Value::as_str).unwrap_or_default();
        let exec = ev.get("script").and_then(|s| s.get("exec"));
        let code = match exec {
            Some(Value::Array(lines)) => lines.iter().map(value_to_string).collect::<Vec<_>>().join("\n"),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        match listen {
            "prerequest" if scripts.pre_request.is_empty() => scripts.pre_request = code,
            "test" if scripts.post_response.is_empty() => scripts.post_response = code,
            _ => scripts.other_events.push(ev),
        }
    }
    scripts
}

fn scripts_to(scripts: &Scripts) -> Option<Value> {
    let mut events = Vec::new();
    for (listen, code) in [("prerequest", &scripts.pre_request), ("test", &scripts.post_response)] {
        if !code.is_empty() {
            let exec: Vec<Value> = code.split('\n').map(|l| Value::String(l.to_string())).collect();
            events.push(json!({ "listen": listen, "script": { "type": "text/javascript", "exec": exec } }));
        }
    }
    events.extend(scripts.other_events.iter().cloned());
    (!events.is_empty()).then_some(Value::Array(events))
}

// ---------------------------------------------------------------- variables

fn variables_from(v: Option<Value>) -> Vec<Variable> {
    let Some(Value::Array(list)) = v else { return Vec::new() };
    list.iter()
        .map(|item| {
            let kv = kv_from(item);
            let secret = item.get("type").and_then(Value::as_str) == Some("secret");
            let enabled = kv.enabled && item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let current = item.get("currentValue").map(value_to_string);
            Variable { key: kv.key, initial: kv.value, current, enabled, secret, description: kv.description }
        })
        .collect()
}

fn variable_to(v: &Variable, value: &str, with_current: bool, enabled_key: bool) -> Value {
    let mut o = Map::new();
    o.insert("key".into(), v.key.clone().into());
    o.insert("value".into(), value.into());
    o.insert("type".into(), if v.secret { "secret" } else { "default" }.into());
    if enabled_key {
        o.insert("enabled".into(), v.enabled.into());
    } else if !v.enabled {
        o.insert("disabled".into(), true.into());
    }
    if !v.description.is_empty() {
        o.insert("description".into(), v.description.clone().into());
    }
    if with_current && let Some(c) = &v.current {
        o.insert("currentValue".into(), c.clone().into());
    }
    Value::Object(o)
}

// ---------------------------------------------------------------- url

fn url_from(v: Option<Value>) -> (String, Option<Vec<KeyValue>>, Option<Vec<KeyValue>>) {
    match v {
        Some(Value::String(s)) => (s, None, None),
        Some(Value::Object(mut o)) => {
            let raw = match o.remove("raw") {
                Some(Value::String(s)) => s,
                _ => build_raw(&o),
            };
            let query = o.remove("query").map(|q| kv_list(Some(q)));
            let vars = o.remove("variable").map(|q| kv_list(Some(q)));
            (raw, query, vars)
        }
        _ => (String::new(), None, None),
    }
}

/// Assemble a raw URL from its parts when `raw` is missing.
fn build_raw(o: &Map<String, Value>) -> String {
    let join = |key: &str, sep: &str| match o.get(key) {
        Some(Value::Array(a)) => a.iter().map(value_to_string).collect::<Vec<_>>().join(sep),
        Some(v) => value_to_string(v),
        None => String::new(),
    };
    let mut raw = String::new();
    if let Some(p) = o.get("protocol").and_then(Value::as_str) {
        raw.push_str(p);
        raw.push_str("://");
    }
    raw.push_str(&join("host", "."));
    if let Some(port) = o.get("port") {
        raw.push(':');
        raw.push_str(&value_to_string(port));
    }
    let path = join("path", "/");
    if !path.is_empty() {
        raw.push('/');
        raw.push_str(&path);
    }
    if let Some(Value::Array(q)) = o.get("query") {
        let parts: Vec<String> = q
            .iter()
            .map(kv_from)
            .filter(|kv| kv.enabled)
            .map(|kv| if kv.value.is_empty() { kv.key } else { format!("{}={}", kv.key, kv.value) })
            .collect();
        if !parts.is_empty() {
            raw.push('?');
            raw.push_str(&parts.join("&"));
        }
    }
    raw
}

fn url_to(req: &Request) -> Value {
    let (base, _, fragment) = crate::model::url::split(&req.url);
    let mut o = Map::new();
    o.insert("raw".into(), req.url.clone().into());
    let mut rest = base;
    if let Some(i) = rest.find("://") {
        o.insert("protocol".into(), rest[..i].into());
        rest = &rest[i + 3..];
    }
    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let (host, port) = match host_port.rfind(':') {
        Some(i) if !host_port[i + 1..].contains('}') && !host_port[i + 1..].is_empty() => {
            (&host_port[..i], Some(&host_port[i + 1..]))
        }
        _ => (host_port, None),
    };
    if !host.is_empty() {
        o.insert("host".into(), host.split('.').map(|s| Value::String(s.into())).collect::<Vec<_>>().into());
    }
    if let Some(p) = port {
        o.insert("port".into(), p.into());
    }
    if !path.is_empty() {
        o.insert("path".into(), path.split('/').map(|s| Value::String(s.into())).collect::<Vec<_>>().into());
    }
    if !req.query.is_empty() {
        o.insert("query".into(), req.query.iter().map(|q| kv_to(q, false)).collect::<Vec<_>>().into());
    }
    if !req.path_variables.is_empty() {
        o.insert("variable".into(), req.path_variables.iter().map(|q| kv_to(q, false)).collect::<Vec<_>>().into());
    }
    if let Some(f) = fragment {
        o.insert("hash".into(), f.into());
    }
    Value::Object(o)
}

// ---------------------------------------------------------------- body

fn body_from(v: Option<Value>) -> Body {
    let Some(Value::Object(mut o)) = v else { return Body::default() };
    let mut body = Body::default();
    let mode = take_str(&mut o, "mode");
    body.mode = match mode.as_str() {
        "raw" => BodyMode::Raw,
        "urlencoded" => BodyMode::Urlencoded,
        "formdata" => BodyMode::Formdata,
        "file" => BodyMode::File,
        "graphql" => BodyMode::Graphql,
        _ => BodyMode::None,
    };
    body.raw = take_str(&mut o, "raw");
    body.urlencoded = kv_list(o.remove("urlencoded"));
    if let Some(Value::Array(fields)) = o.remove("formdata") {
        body.formdata = fields
            .iter()
            .map(|f| {
                let kv = kv_from(f);
                let is_file = f.get("type").and_then(Value::as_str) == Some("file");
                let files = match f.get("src") {
                    Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
                    Some(Value::Array(a)) => a.iter().map(value_to_string).collect(),
                    _ => Vec::new(),
                };
                FormField {
                    key: kv.key,
                    value: kv.value,
                    kind: if is_file { FormFieldKind::File } else { FormFieldKind::Text },
                    files,
                    content_type: f.get("contentType").and_then(Value::as_str).map(str::to_string),
                    enabled: kv.enabled,
                    description: kv.description,
                }
            })
            .collect();
    }
    if let Some(Value::Object(file)) = o.remove("file") {
        body.file = file.get("src").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(Value::Object(mut g)) = o.remove("graphql") {
        body.graphql.query = take_str(&mut g, "query");
        body.graphql.variables = take_str(&mut g, "variables");
    }
    if let Some(Value::Object(mut options)) = o.remove("options") {
        if let Some(Value::Object(mut raw)) = options.remove("raw") {
            body.raw_language = RawLanguage::from_postman(&take_str(&mut raw, "language"));
            if !raw.is_empty() {
                options.insert("raw".into(), Value::Object(raw));
            }
        }
        if !options.is_empty() {
            o.insert("options".into(), Value::Object(options));
        }
    }
    body.extra = o;
    body
}

fn body_to(body: &Body) -> Option<Value> {
    let mut o = body.extra.clone();
    match body.mode {
        BodyMode::None => return None,
        BodyMode::Raw => {
            o.insert("mode".into(), "raw".into());
            o.insert("raw".into(), body.raw.clone().into());
            let options = o.entry("options").or_insert_with(|| json!({}));
            if let Some(map) = options.as_object_mut() {
                map.insert("raw".into(), json!({ "language": body.raw_language.postman_name() }));
            }
        }
        BodyMode::Urlencoded => {
            o.insert("mode".into(), "urlencoded".into());
            o.insert("urlencoded".into(), body.urlencoded.iter().map(|kv| kv_to(kv, true)).collect::<Vec<_>>().into());
        }
        BodyMode::Formdata => {
            o.insert("mode".into(), "formdata".into());
            let fields: Vec<Value> = body
                .formdata
                .iter()
                .map(|f| {
                    let mut m = Map::new();
                    m.insert("key".into(), f.key.clone().into());
                    match f.kind {
                        FormFieldKind::Text => {
                            m.insert("value".into(), f.value.clone().into());
                            m.insert("type".into(), "text".into());
                        }
                        FormFieldKind::File => {
                            m.insert("type".into(), "file".into());
                            let src = match f.files.as_slice() {
                                [one] => Value::String(one.clone()),
                                many => many.iter().cloned().map(Value::String).collect::<Vec<_>>().into(),
                            };
                            m.insert("src".into(), src);
                        }
                    }
                    if let Some(ct) = &f.content_type {
                        m.insert("contentType".into(), ct.clone().into());
                    }
                    if !f.enabled {
                        m.insert("disabled".into(), true.into());
                    }
                    if !f.description.is_empty() {
                        m.insert("description".into(), f.description.clone().into());
                    }
                    Value::Object(m)
                })
                .collect();
            o.insert("formdata".into(), fields.into());
        }
        BodyMode::File => {
            o.insert("mode".into(), "file".into());
            o.insert("file".into(), json!({ "src": body.file.clone().unwrap_or_default() }));
        }
        BodyMode::Graphql => {
            o.insert("mode".into(), "graphql".into());
            o.insert("graphql".into(), json!({ "query": body.graphql.query, "variables": body.graphql.variables }));
        }
    }
    Some(Value::Object(o))
}

// ---------------------------------------------------------------- request

fn request_from(v: Option<Value>, settings: Option<Value>) -> (Request, String) {
    let mut req = Request::default();
    let mut obj = match v {
        Some(Value::Object(o)) => o,
        Some(Value::String(url)) => {
            req.set_url(&url);
            return (req, String::new());
        }
        _ => Map::new(),
    };
    let method = take_str(&mut obj, "method");
    req.method = if method.is_empty() { "GET".into() } else { method.to_uppercase() };
    let (raw, query, vars) = url_from(obj.remove("url"));
    req.set_url(&raw);
    if let Some(q) = query {
        req.query = q;
    }
    if let Some(v) = vars {
        req.path_variables = v;
    }
    req.headers = kv_list(obj.remove("header"));
    req.body = body_from(obj.remove("body"));
    req.auth = auth_from(obj.remove("auth")).unwrap_or_default();
    let description = take_description(&mut obj);
    if let Some(Value::Object(s)) = settings {
        req.settings = s;
    }
    req.extra = obj;
    (req, description)
}

fn request_to(req: &Request) -> Value {
    let mut o = req.extra.clone();
    o.insert("method".into(), req.method.clone().into());
    if req.auth.kind != AuthType::Inherit {
        o.insert("auth".into(), auth_to(&req.auth));
    }
    o.insert("header".into(), req.headers.iter().map(|h| kv_to(h, true)).collect::<Vec<_>>().into());
    if let Some(b) = body_to(&req.body) {
        o.insert("body".into(), b);
    }
    o.insert("url".into(), url_to(req));
    if !req.description.is_empty() {
        o.insert("description".into(), req.description.clone().into());
    }
    Value::Object(o)
}

// ---------------------------------------------------------------- examples

fn example_from(v: Value) -> Example {
    let Value::Object(mut o) = v else { return Example::default() };
    let id = take_str(&mut o, "id");
    let original_request = o.remove("originalRequest").map(|r| request_from(Some(r), None).0);
    let headers = kv_list(o.remove("header"));
    Example {
        id: if id.is_empty() { new_id() } else { id },
        name: take_str(&mut o, "name"),
        original_request,
        status: take_str(&mut o, "status"),
        code: o.remove("code").and_then(|c| c.as_u64()).unwrap_or(0) as u16,
        headers,
        body: take_str(&mut o, "body"),
        preview_language: o.remove("_postman_previewlanguage").and_then(|v| v.as_str().map(str::to_string)),
        extra: o,
    }
}

fn example_to(e: &Example) -> Value {
    let mut o = e.extra.clone();
    o.insert("id".into(), e.id.clone().into());
    o.insert("name".into(), e.name.clone().into());
    if let Some(r) = &e.original_request {
        o.insert("originalRequest".into(), request_to(r));
    }
    o.insert("status".into(), e.status.clone().into());
    o.insert("code".into(), e.code.into());
    if let Some(l) = &e.preview_language {
        o.insert("_postman_previewlanguage".into(), l.clone().into());
    }
    o.insert("header".into(), e.headers.iter().map(|h| kv_to(h, false)).collect::<Vec<_>>().into());
    o.insert("body".into(), e.body.clone().into());
    Value::Object(o)
}

// ---------------------------------------------------------------- items

fn item_from(v: Value) -> Item {
    let Value::Object(mut o) = v else { return Item::Request(RequestItem::new("", Request::default())) };
    let id = take_str(&mut o, "id");
    let id = if id.is_empty() { new_id() } else { id };
    let name = take_str(&mut o, "name");
    if let Some(children) = o.remove("item") {
        let items = match children {
            Value::Array(a) => a.into_iter().map(item_from).collect(),
            _ => Vec::new(),
        };
        return Item::Folder(Folder {
            id,
            name,
            description: take_description(&mut o),
            auth: auth_from(o.remove("auth")).unwrap_or_default(),
            scripts: scripts_from(o.remove("event")),
            items,
            extra: o,
        });
    }
    let (mut request, description) = request_from(o.remove("request"), o.remove("protocolProfileBehavior"));
    if request.description.is_empty() {
        request.description = description;
    }
    let examples = match o.remove("response") {
        Some(Value::Array(a)) => a.into_iter().map(example_from).collect(),
        _ => Vec::new(),
    };
    Item::Request(RequestItem { id, name, request, scripts: scripts_from(o.remove("event")), examples, extra: o })
}

fn item_to(item: &Item) -> Value {
    match item {
        Item::Folder(f) => {
            let mut o = f.extra.clone();
            o.insert("id".into(), f.id.clone().into());
            o.insert("name".into(), f.name.clone().into());
            if !f.description.is_empty() {
                o.insert("description".into(), f.description.clone().into());
            }
            o.insert("item".into(), f.items.iter().map(item_to).collect::<Vec<_>>().into());
            if f.auth.kind != AuthType::Inherit {
                o.insert("auth".into(), auth_to(&f.auth));
            }
            if let Some(e) = scripts_to(&f.scripts) {
                o.insert("event".into(), e);
            }
            Value::Object(o)
        }
        Item::Request(r) => {
            let mut o = r.extra.clone();
            o.insert("id".into(), r.id.clone().into());
            o.insert("name".into(), r.name.clone().into());
            if let Some(e) = scripts_to(&r.scripts) {
                o.insert("event".into(), e);
            }
            if !r.request.settings.is_empty() {
                o.insert("protocolProfileBehavior".into(), Value::Object(r.request.settings.clone()));
            }
            o.insert("request".into(), request_to(&r.request));
            o.insert("response".into(), r.examples.iter().map(example_to).collect::<Vec<_>>().into());
            Value::Object(o)
        }
    }
}

// ---------------------------------------------------------------- collection

/// Whether a JSON value looks like a v2.x collection.
pub fn is_collection(v: &Value) -> bool {
    v.get("info").is_some() && v.get("item").is_some_and(Value::is_array)
}

pub fn import_collection(text: &str) -> Result<Collection> {
    collection_from_value(serde_json::from_str(text)?)
}

pub fn collection_from_value(v: Value) -> Result<Collection> {
    if !is_collection(&v) {
        return Err(FormatError::NotACollection("missing `info` or `item`".into()));
    }
    let Value::Object(mut o) = v else { unreachable!() };
    let Some(Value::Object(mut info)) = o.remove("info") else {
        return Err(FormatError::NotACollection("`info` is not an object".into()));
    };
    let schema = info.get("schema").and_then(Value::as_str).unwrap_or_default().to_string();
    if !schema.is_empty() && !schema.contains("v2.") {
        return Err(FormatError::NotACollection(format!("unsupported schema {schema}")));
    }
    let id = take_str(&mut info, "_postman_id");
    let name = take_str(&mut info, "name");
    let description = take_description(&mut info);
    info.remove("schema");
    let items = match o.remove("item") {
        Some(Value::Array(a)) => a.into_iter().map(item_from).collect(),
        _ => Vec::new(),
    };
    let mut extra = o;
    let auth = auth_from(extra.remove("auth")).unwrap_or_else(Auth::none);
    let scripts = scripts_from(extra.remove("event"));
    let variables = variables_from(extra.remove("variable"));
    if !info.is_empty() {
        extra.insert("_info".into(), Value::Object(info));
    }
    Ok(Collection {
        id: if id.is_empty() { new_id() } else { id },
        name: if name.is_empty() { "Imported collection".into() } else { name },
        description,
        auth,
        scripts,
        variables,
        items,
        extra,
    })
}

/// Export a collection as v2.1 JSON. With `include_current`, variable
/// current values are kept in a `currentValue` field (used for local storage).
pub fn collection_to_value(c: &Collection, include_current: bool) -> Value {
    let mut extra = c.extra.clone();
    let mut info = match extra.remove("_info") {
        Some(Value::Object(i)) => i,
        _ => Map::new(),
    };
    info.insert("_postman_id".into(), c.id.clone().into());
    info.insert("name".into(), c.name.clone().into());
    if !c.description.is_empty() {
        info.insert("description".into(), c.description.clone().into());
    }
    info.insert("schema".into(), SCHEMA_V21.into());
    let mut o = Map::new();
    o.insert("info".into(), Value::Object(info));
    o.insert("item".into(), c.items.iter().map(item_to).collect::<Vec<_>>().into());
    if !matches!(c.auth.kind, AuthType::NoAuth | AuthType::Inherit) || !c.auth.params.is_empty() {
        o.insert("auth".into(), auth_to(&c.auth));
    }
    if let Some(e) = scripts_to(&c.scripts) {
        o.insert("event".into(), e);
    }
    if !c.variables.is_empty() {
        let vars: Vec<Value> = c.variables.iter().map(|v| variable_to(v, &v.initial, include_current, false)).collect();
        o.insert("variable".into(), vars.into());
    }
    for (k, v) in extra {
        o.insert(k, v);
    }
    Value::Object(o)
}

pub fn export_collection(c: &Collection) -> String {
    serde_json::to_string_pretty(&collection_to_value(c, false)).expect("serializable")
}

// ---------------------------------------------------------------- environment

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Environment,
    Globals,
}

pub fn is_environment(v: &Value) -> Option<Scope> {
    match v.get("_postman_variable_scope").and_then(Value::as_str) {
        Some("environment") => Some(Scope::Environment),
        Some("globals") => Some(Scope::Globals),
        _ if v.get("values").is_some_and(Value::is_array) && v.get("info").is_none() => Some(Scope::Environment),
        _ => None,
    }
}

pub fn import_environment(text: &str) -> Result<(Environment, Scope)> {
    environment_from_value(serde_json::from_str(text)?)
}

pub fn environment_from_value(v: Value) -> Result<(Environment, Scope)> {
    let scope = is_environment(&v).ok_or_else(|| FormatError::NotAnEnvironment("missing `values`".into()))?;
    let Value::Object(mut o) = v else { unreachable!() };
    let id = take_str(&mut o, "id");
    let name = take_str(&mut o, "name");
    let variables = variables_from(o.remove("values"));
    for k in ["_postman_variable_scope", "_postman_exported_at", "_postman_exported_using"] {
        o.remove(k);
    }
    let env = Environment {
        id: if id.is_empty() { new_id() } else { id },
        name: if name.is_empty() && scope == Scope::Globals { "Globals".into() } else { name },
        variables,
        extra: o,
    };
    Ok((env, scope))
}

/// Serialize an environment or globals file. `use_current` writes current
/// values as `value` (export); `with_current` adds `currentValue` (storage).
pub fn environment_to_value(env: &Environment, scope: Scope, use_current: bool, with_current: bool) -> Value {
    let mut o = env.extra.clone();
    o.insert("id".into(), env.id.clone().into());
    o.insert("name".into(), env.name.clone().into());
    let values: Vec<Value> = env
        .variables
        .iter()
        .map(|v| {
            let value = if use_current { v.value() } else { v.initial.as_str() };
            variable_to(v, value, with_current, true)
        })
        .collect();
    o.insert("values".into(), values.into());
    let scope_name = match scope {
        Scope::Environment => "environment",
        Scope::Globals => "globals",
    };
    o.insert("_postman_variable_scope".into(), scope_name.into());
    if !with_current {
        o.insert("_postman_exported_at".into(), chrono::Utc::now().to_rfc3339().into());
        o.insert("_postman_exported_using".into(), EXPORTED_USING.into());
    }
    Value::Object(o)
}

pub fn export_environment(env: &Environment, scope: Scope, use_current: bool) -> String {
    serde_json::to_string_pretty(&environment_to_value(env, scope, use_current, false)).expect("serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../../tests/fixtures/collection_v21.json");

    #[test]
    fn imports_fixture() {
        let c = import_collection(FIXTURE).unwrap();
        assert_eq!(c.name, "Courier Fixture");
        assert_eq!(c.auth.kind, AuthType::Bearer);
        assert_eq!(c.auth.get("token"), "{{token}}");
        assert_eq!(c.variables[0].key, "baseUrl");
        assert!(c.scripts.pre_request.contains("collection pre"));
        let reqs = c.requests();
        assert_eq!(reqs.len(), 7);

        let list = reqs[0];
        assert_eq!(list.request.method, "GET");
        assert_eq!(list.request.url, "{{baseUrl}}/users?page=2");
        assert_eq!(list.request.query.len(), 2);
        assert!(!list.request.query[1].enabled);
        assert_eq!(list.request.auth.kind, AuthType::Inherit);
        assert_eq!(list.examples.len(), 1);
        assert_eq!(list.examples[0].code, 200);
        assert_eq!(list.scripts.post_response, "pm.test('ok', () => {\n  pm.response.to.have.status(200);\n});");

        let create = reqs[1];
        assert_eq!(create.request.body.mode, BodyMode::Raw);
        assert_eq!(create.request.body.raw_language, RawLanguage::Json);
        assert_eq!(create.request.auth.kind, AuthType::NoAuth);
        assert_eq!(create.request.setting_bool("followRedirects"), Some(false));

        let upload = reqs[2];
        assert_eq!(upload.request.body.formdata[1].kind, FormFieldKind::File);
        assert_eq!(upload.request.body.formdata[1].files, ["/tmp/a.png"]);
        assert_eq!(upload.request.path_variables[0].key, "id");
        assert_eq!(upload.request.path_variables[0].value, "7");

        assert_eq!(reqs[3].request.body.mode, BodyMode::Urlencoded);
        assert_eq!(reqs[4].request.body.mode, BodyMode::Graphql);
        assert_eq!(reqs[5].request.body.file.as_deref(), Some("/tmp/blob.bin"));
        // String URL + v2.0-style auth object.
        assert_eq!(reqs[6].request.url, "https://example.com/legacy");
        assert_eq!(reqs[6].request.auth.kind, AuthType::Basic);
        assert_eq!(reqs[6].request.auth.get("username"), "u");

        // Folder auth and unknown fields survive.
        let Item::Folder(folder) = &c.items[0] else { panic!("folder") };
        assert_eq!(folder.auth.kind, AuthType::Inherit);
        assert!(c.extra.contains_key("customTopLevel"));
    }

    #[test]
    fn round_trip_is_lossless() {
        let first = import_collection(FIXTURE).unwrap();
        let exported = export_collection(&first);
        let second = import_collection(&exported).unwrap();
        assert_eq!(first, second);
        // And the export itself is stable.
        assert_eq!(exported, export_collection(&second));
        let v: Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(v["info"]["schema"], SCHEMA_V21);
        assert_eq!(v["customTopLevel"]["kept"], true);
    }

    #[test]
    fn url_object_parts() {
        let req = Request::new("GET", "https://api.example.com:8443/v1/users/:id?x=1#frag");
        let v = url_to(&req);
        assert_eq!(v["protocol"], "https");
        assert_eq!(v["host"], json!(["api", "example", "com"]));
        assert_eq!(v["port"], "8443");
        assert_eq!(v["path"], json!(["v1", "users", ":id"]));
        assert_eq!(v["query"][0]["key"], "x");
        assert_eq!(v["variable"][0]["key"], "id");
        assert_eq!(v["hash"], "frag");
    }

    #[test]
    fn url_from_parts_without_raw() {
        let (raw, _, _) = url_from(Some(json!({
            "protocol": "http", "host": ["localhost"], "port": "3000", "path": ["a", "b"],
            "query": [{ "key": "q", "value": "1" }, { "key": "off", "value": "2", "disabled": true }]
        })));
        assert_eq!(raw, "http://localhost:3000/a/b?q=1");
    }

    #[test]
    fn environment_round_trip() {
        let text = r#"{
            "id": "env-1", "name": "Dev", "_postman_variable_scope": "environment",
            "values": [
                { "key": "host", "value": "localhost", "type": "default", "enabled": true },
                { "key": "token", "value": "s3cret", "type": "secret", "enabled": false }
            ]
        }"#;
        let (mut env, scope) = import_environment(text).unwrap();
        assert_eq!(scope, Scope::Environment);
        assert!(env.variables[1].secret);
        assert!(!env.variables[1].enabled);
        env.variables[0].current = Some("127.0.0.1".into());

        let exported = export_environment(&env, Scope::Environment, false);
        let (back, _) = import_environment(&exported).unwrap();
        assert_eq!(back.variables[0].initial, "localhost");
        assert_eq!(back.variables[0].current, None);

        let current = export_environment(&env, Scope::Environment, true);
        assert!(current.contains("127.0.0.1"));

        let stored = environment_to_value(&env, Scope::Environment, false, true);
        let (restored, _) = environment_from_value(stored).unwrap();
        assert_eq!(restored, env);
    }

    #[test]
    fn globals_scope_detected() {
        let (g, scope) =
            import_environment(r#"{"values":[{"key":"a","value":1}],"_postman_variable_scope":"globals"}"#).unwrap();
        assert_eq!(scope, Scope::Globals);
        assert_eq!(g.name, "Globals");
        assert_eq!(g.variables[0].initial, "1");
    }

    #[test]
    fn rejects_non_collections() {
        assert!(import_collection("{}").is_err());
        assert!(import_collection("nope").is_err());
        assert!(
            import_collection(
                r#"{"info":{"schema":"https://schema.getpostman.com/json/collection/v1.0.0/x"},"item":[]}"#
            )
            .is_err()
        );
    }
}
