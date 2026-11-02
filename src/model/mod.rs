//! Courier's in-memory data model.
//!
//! The model is deliberately close to the Postman Collection v2.1 format so
//! that import/export is lossless, but uses typed Rust structures where the
//! application needs them. Anything Courier does not understand is carried in
//! `extra` maps and written back unchanged.

pub mod url;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type JsonMap = Map<String, Value>;

/// Generate a fresh random identifier.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A single row in a key/value table (params, headers, urlencoded fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn yes() -> bool {
    true
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into(), enabled: true, description: String::new() }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Parse "bulk edit" text (`key:value` per line, `//` prefix disables a row).
pub fn parse_bulk(text: &str) -> Vec<KeyValue> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (enabled, line) = match line.trim_start().strip_prefix("//") {
                Some(rest) => (false, rest),
                None => (true, line),
            };
            let (k, v) = line.split_once(':').unwrap_or((line, ""));
            KeyValue { key: k.trim().to_string(), value: v.trim().to_string(), enabled, description: String::new() }
        })
        .collect()
}

/// Render rows as "bulk edit" text; inverse of [`parse_bulk`].
pub fn to_bulk(rows: &[KeyValue]) -> String {
    rows.iter()
        .map(|r| format!("{}{}:{}", if r.enabled { "" } else { "//" }, r.key, r.value))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BodyMode {
    #[default]
    None,
    Raw,
    Urlencoded,
    Formdata,
    File,
    Graphql,
}

impl BodyMode {
    pub const ALL: [BodyMode; 6] =
        [BodyMode::None, BodyMode::Formdata, BodyMode::Urlencoded, BodyMode::Raw, BodyMode::File, BodyMode::Graphql];

    pub fn label(self) -> &'static str {
        match self {
            BodyMode::None => "none",
            BodyMode::Raw => "raw",
            BodyMode::Urlencoded => "x-www-form-urlencoded",
            BodyMode::Formdata => "form-data",
            BodyMode::File => "binary",
            BodyMode::Graphql => "GraphQL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RawLanguage {
    #[default]
    Text,
    Json,
    Javascript,
    Html,
    Xml,
}

impl RawLanguage {
    pub const ALL: [RawLanguage; 5] =
        [RawLanguage::Text, RawLanguage::Json, RawLanguage::Javascript, RawLanguage::Html, RawLanguage::Xml];

    pub fn label(self) -> &'static str {
        match self {
            RawLanguage::Text => "Text",
            RawLanguage::Json => "JSON",
            RawLanguage::Javascript => "JavaScript",
            RawLanguage::Html => "HTML",
            RawLanguage::Xml => "XML",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RawLanguage::Text => "text/plain",
            RawLanguage::Json => "application/json",
            RawLanguage::Javascript => "application/javascript",
            RawLanguage::Html => "text/html",
            RawLanguage::Xml => "application/xml",
        }
    }

    pub fn from_postman(s: &str) -> Self {
        match s {
            "json" => Self::Json,
            "javascript" => Self::Javascript,
            "html" => Self::Html,
            "xml" => Self::Xml,
            _ => Self::Text,
        }
    }

    pub fn postman_name(self) -> &'static str {
        match self {
            RawLanguage::Text => "text",
            RawLanguage::Json => "json",
            RawLanguage::Javascript => "javascript",
            RawLanguage::Html => "html",
            RawLanguage::Xml => "xml",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FormFieldKind {
    #[default]
    Text,
    File,
}

/// One row in a multipart form body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub kind: FormFieldKind,
    /// File paths for `kind == File`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GraphqlBody {
    pub query: String,
    pub variables: String,
}

/// Request body. Every mode's content is kept so switching modes is lossless
/// while editing; only `mode` decides what is sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Body {
    pub mode: BodyMode,
    pub raw: String,
    pub raw_language: RawLanguage,
    pub urlencoded: Vec<KeyValue>,
    pub formdata: Vec<FormField>,
    /// Path of the file sent in binary mode.
    pub file: Option<String>,
    pub graphql: GraphqlBody,
    #[serde(skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl Body {
    pub fn raw(text: impl Into<String>, language: RawLanguage) -> Self {
        Self { mode: BodyMode::Raw, raw: text.into(), raw_language: language, ..Default::default() }
    }
}

/// Authorization schemes. `Inherit` means "use the parent's auth".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    #[default]
    Inherit,
    NoAuth,
    ApiKey,
    Bearer,
    Basic,
    Digest,
    OAuth1,
    OAuth2,
    Jwt,
    Hawk,
    AwsV4,
    Ntlm,
    EdgeGrid,
    Asap,
    Other(String),
}

impl AuthType {
    /// Types shown in the auth dropdown, in display order.
    pub const SELECTABLE: [AuthType; 13] = [
        AuthType::Inherit,
        AuthType::NoAuth,
        AuthType::ApiKey,
        AuthType::Bearer,
        AuthType::Basic,
        AuthType::Digest,
        AuthType::OAuth1,
        AuthType::OAuth2,
        AuthType::Jwt,
        AuthType::Hawk,
        AuthType::AwsV4,
        AuthType::Ntlm,
        AuthType::EdgeGrid,
    ];

    pub fn label(&self) -> &str {
        match self {
            AuthType::Inherit => "Inherit auth from parent",
            AuthType::NoAuth => "No Auth",
            AuthType::ApiKey => "API Key",
            AuthType::Bearer => "Bearer Token",
            AuthType::Basic => "Basic Auth",
            AuthType::Digest => "Digest Auth",
            AuthType::OAuth1 => "OAuth 1.0",
            AuthType::OAuth2 => "OAuth 2.0",
            AuthType::Jwt => "JWT Bearer",
            AuthType::Hawk => "Hawk Authentication",
            AuthType::AwsV4 => "AWS Signature",
            AuthType::Ntlm => "NTLM Authentication",
            AuthType::EdgeGrid => "Akamai EdgeGrid",
            AuthType::Asap => "ASAP (Atlassian)",
            AuthType::Other(s) => s,
        }
    }

    /// Name used in the Postman v2.1 `auth.type` field.
    pub fn postman_name(&self) -> &str {
        match self {
            AuthType::Inherit => "inherit",
            AuthType::NoAuth => "noauth",
            AuthType::ApiKey => "apikey",
            AuthType::Bearer => "bearer",
            AuthType::Basic => "basic",
            AuthType::Digest => "digest",
            AuthType::OAuth1 => "oauth1",
            AuthType::OAuth2 => "oauth2",
            AuthType::Jwt => "jwt",
            AuthType::Hawk => "hawk",
            AuthType::AwsV4 => "awsv4",
            AuthType::Ntlm => "ntlm",
            AuthType::EdgeGrid => "edgegrid",
            AuthType::Asap => "asap",
            AuthType::Other(s) => s,
        }
    }

    pub fn from_postman(s: &str) -> Self {
        match s {
            "inherit" => AuthType::Inherit,
            "noauth" => AuthType::NoAuth,
            "apikey" => AuthType::ApiKey,
            "bearer" => AuthType::Bearer,
            "basic" => AuthType::Basic,
            "digest" => AuthType::Digest,
            "oauth1" => AuthType::OAuth1,
            "oauth2" => AuthType::OAuth2,
            "jwt" => AuthType::Jwt,
            "hawk" => AuthType::Hawk,
            "awsv4" => AuthType::AwsV4,
            "ntlm" => AuthType::Ntlm,
            "edgegrid" => AuthType::EdgeGrid,
            "asap" => AuthType::Asap,
            other => AuthType::Other(other.to_string()),
        }
    }

    /// Whether Courier can actually sign requests with this type.
    pub fn is_supported(&self) -> bool {
        !matches!(self, AuthType::Ntlm | AuthType::EdgeGrid | AuthType::Asap | AuthType::Other(_))
    }
}

/// Authorization settings: a type plus its parameters. Parameters are kept as
/// JSON values so every scheme (and unknown ones) round-trip losslessly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Auth {
    #[serde(rename = "type")]
    pub kind: AuthType,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub params: JsonMap,
}

impl Auth {
    pub fn new(kind: AuthType) -> Self {
        Self { kind, params: JsonMap::new() }
    }

    pub fn none() -> Self {
        Self::new(AuthType::NoAuth)
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_string(), value.into());
        self
    }

    /// String parameter (numbers and booleans are stringified).
    pub fn get(&self, key: &str) -> String {
        match self.params.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        }
    }

    pub fn get_bool(&self, key: &str) -> bool {
        match self.params.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "true",
            _ => false,
        }
    }

    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.params.insert(key.to_string(), value.into());
    }
}

/// Pre-request and post-response scripts attached to a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Scripts {
    pub pre_request: String,
    pub post_response: String,
    /// Events of types Courier does not run, kept for round-tripping.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_events: Vec<Value>,
}

/// A variable with an initial (shared) and current (local) value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub key: String,
    #[serde(default)]
    pub initial: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Variable {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            initial: value.into(),
            current: None,
            enabled: true,
            secret: false,
            description: String::new(),
        }
    }

    /// The value used when sending: current if set, otherwise initial.
    pub fn value(&self) -> &str {
        self.current.as_deref().unwrap_or(&self.initial)
    }
}

/// Operations shared by every variable list (environment, globals, collection).
pub trait VariableList {
    fn vars(&self) -> &Vec<Variable>;
    fn vars_mut(&mut self) -> &mut Vec<Variable>;

    fn get_var(&self, key: &str) -> Option<&str> {
        self.vars().iter().find(|v| v.enabled && v.key == key).map(|v| v.value())
    }

    /// Set the current value, creating the variable if needed.
    fn set_var(&mut self, key: &str, value: &str) {
        let vars = self.vars_mut();
        match vars.iter_mut().find(|v| v.key == key) {
            Some(v) => {
                v.current = Some(value.to_string());
                v.enabled = true;
            }
            None => vars.push(Variable { current: Some(value.to_string()), ..Variable::new(key, "") }),
        }
    }

    fn unset_var(&mut self, key: &str) {
        self.vars_mut().retain(|v| v.key != key);
    }

    /// Copy every current value into the initial value.
    fn persist_all(&mut self) {
        for v in self.vars_mut() {
            if let Some(c) = v.current.take() {
                v.initial = c;
            }
        }
    }

    /// Drop every current value so the initial value is used again.
    fn reset_all(&mut self) {
        for v in self.vars_mut() {
            v.current = None;
        }
    }
}

impl VariableList for Vec<Variable> {
    fn vars(&self) -> &Vec<Variable> {
        self
    }
    fn vars_mut(&mut self) -> &mut Vec<Variable> {
        self
    }
}

/// A named set of variables. Globals use the same structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl Environment {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), variables: Vec::new(), extra: JsonMap::new() }
    }
}

impl VariableList for Environment {
    fn vars(&self) -> &Vec<Variable> {
        &self.variables
    }
    fn vars_mut(&mut self) -> &mut Vec<Variable> {
        &mut self.variables
    }
}

/// An HTTP request definition (without name/scripts, which live on the item).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Request {
    pub method: String,
    /// Raw URL as typed, including the enabled query string.
    pub url: String,
    /// All query params, including disabled ones (which are not in `url`).
    pub query: Vec<KeyValue>,
    /// Values for `:name` path segments.
    pub path_variables: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body: Body,
    pub auth: Auth,
    /// `protocolProfileBehavior` settings.
    #[serde(skip_serializing_if = "JsonMap::is_empty")]
    pub settings: JsonMap,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            method: "GET".into(),
            url: String::new(),
            query: Vec::new(),
            path_variables: Vec::new(),
            headers: Vec::new(),
            body: Body::default(),
            auth: Auth::default(),
            settings: JsonMap::new(),
            description: String::new(),
            extra: JsonMap::new(),
        }
    }
}

impl Request {
    pub fn new(method: &str, url: &str) -> Self {
        let mut r = Self { method: method.to_uppercase(), ..Default::default() };
        r.set_url(url);
        r
    }

    /// Set the raw URL and re-derive query params and path variables.
    pub fn set_url(&mut self, raw: &str) {
        self.url = raw.to_string();
        self.query = url::merge_query(&self.query, &url::parse_query(raw));
        self.path_variables = url::merge_path_variables(&self.path_variables, raw);
    }

    /// Rebuild the raw URL after the query table changed.
    pub fn sync_url_from_query(&mut self) {
        self.url = url::with_query(&self.url, &self.query);
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.enabled && h.key.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }

    pub fn setting_bool(&self, key: &str) -> Option<bool> {
        self.settings.get(key).and_then(Value::as_bool)
    }

    pub fn setting_u64(&self, key: &str) -> Option<u64> {
        self.settings.get(key).and_then(Value::as_u64)
    }
}

/// A saved request/response pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Example {
    pub id: String,
    pub name: String,
    pub original_request: Option<Request>,
    pub status: String,
    pub code: u16,
    pub headers: Vec<KeyValue>,
    pub body: String,
    pub preview_language: Option<String>,
    #[serde(skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestItem {
    pub id: String,
    pub name: String,
    pub request: Request,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub examples: Vec<Example>,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl RequestItem {
    pub fn new(name: impl Into<String>, request: Request) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            request,
            scripts: Scripts::default(),
            examples: Vec::new(),
            extra: JsonMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            description: String::new(),
            auth: Auth::default(),
            scripts: Scripts::default(),
            items: Vec::new(),
            extra: JsonMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)]
pub enum Item {
    Folder(Folder),
    Request(RequestItem),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Folder(f) => &f.id,
            Item::Request(r) => &r.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Item::Folder(f) => &f.name,
            Item::Request(r) => &r.name,
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            Item::Folder(f) => f.name = name,
            Item::Request(r) => r.name = name,
        }
    }

    /// Deep clone with fresh ids throughout (used by "Duplicate").
    pub fn duplicate(&self) -> Item {
        let mut copy = self.clone();
        fn reassign(item: &mut Item) {
            match item {
                Item::Folder(f) => {
                    f.id = new_id();
                    f.items.iter_mut().for_each(reassign);
                }
                Item::Request(r) => {
                    r.id = new_id();
                    r.examples.iter_mut().for_each(|e| e.id = new_id());
                }
            }
        }
        reassign(&mut copy);
        copy
    }
}

/// A collection: the root of a tree of folders and requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default, skip_serializing_if = "JsonMap::is_empty")]
    pub extra: JsonMap,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            description: String::new(),
            auth: Auth::none(),
            scripts: Scripts::default(),
            variables: Vec::new(),
            items: Vec::new(),
            extra: JsonMap::new(),
        }
    }

    /// Find any item (folder or request) by id.
    pub fn find(&self, id: &str) -> Option<&Item> {
        find_in(&self.items, id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Item> {
        find_in_mut(&mut self.items, id)
    }

    pub fn find_request(&self, id: &str) -> Option<&RequestItem> {
        match self.find(id) {
            Some(Item::Request(r)) => Some(r),
            _ => None,
        }
    }

    pub fn find_request_mut(&mut self, id: &str) -> Option<&mut RequestItem> {
        match self.find_mut(id) {
            Some(Item::Request(r)) => Some(r),
            _ => None,
        }
    }

    /// Folders from the root down to (not including) the item with `id`.
    pub fn ancestors(&self, id: &str) -> Option<Vec<&Folder>> {
        fn walk<'a>(items: &'a [Item], id: &str, path: &mut Vec<&'a Folder>) -> bool {
            for item in items {
                if item.id() == id {
                    return true;
                }
                if let Item::Folder(f) = item {
                    path.push(f);
                    if walk(&f.items, id, path) {
                        return true;
                    }
                    path.pop();
                }
            }
            false
        }
        let mut path = Vec::new();
        walk(&self.items, id, &mut path).then_some(path)
    }

    /// Remove an item anywhere in the tree and return it.
    pub fn remove(&mut self, id: &str) -> Option<Item> {
        fn walk(items: &mut Vec<Item>, id: &str) -> Option<Item> {
            if let Some(pos) = items.iter().position(|i| i.id() == id) {
                return Some(items.remove(pos));
            }
            items.iter_mut().find_map(|i| match i {
                Item::Folder(f) => walk(&mut f.items, id),
                Item::Request(_) => None,
            })
        }
        walk(&mut self.items, id)
    }

    /// Insert an item into a folder (or the root when `parent` is `None`) at
    /// `index` (clamped). Returns false if the parent folder was not found.
    pub fn insert(&mut self, parent: Option<&str>, index: usize, item: Item) -> bool {
        let items = match parent {
            None => &mut self.items,
            Some(pid) => match self.find_mut(pid) {
                Some(Item::Folder(f)) => &mut f.items,
                _ => return false,
            },
        };
        let index = index.min(items.len());
        items.insert(index, item);
        true
    }

    /// Id of the folder containing `id` (`None` = root), and its index there.
    pub fn position(&self, id: &str) -> Option<(Option<String>, usize)> {
        fn walk(items: &[Item], parent: Option<&str>, id: &str) -> Option<(Option<String>, usize)> {
            if let Some(pos) = items.iter().position(|i| i.id() == id) {
                return Some((parent.map(str::to_string), pos));
            }
            items.iter().find_map(|i| match i {
                Item::Folder(f) => walk(&f.items, Some(&f.id), id),
                Item::Request(_) => None,
            })
        }
        walk(&self.items, None, id)
    }

    /// Every request in depth-first order (the order the runner uses).
    pub fn requests(&self) -> Vec<&RequestItem> {
        fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a RequestItem>) {
            for i in items {
                match i {
                    Item::Folder(f) => walk(&f.items, out),
                    Item::Request(r) => out.push(r),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }

    /// Requests below a folder, or the whole collection for `None`.
    pub fn requests_under(&self, folder: Option<&str>) -> Vec<&RequestItem> {
        match folder {
            None => self.requests(),
            Some(id) => match self.find(id) {
                Some(Item::Folder(f)) => {
                    let tmp = Collection { items: f.items.clone(), ..Collection::new("") };
                    let ids: Vec<String> = tmp.requests().iter().map(|r| r.id.clone()).collect();
                    ids.iter().filter_map(|id| self.find_request(id)).collect()
                }
                Some(Item::Request(r)) => vec![r],
                None => Vec::new(),
            },
        }
    }

    /// Resolve the effective auth for an item by walking up through
    /// `Inherit` settings. Returns the auth and the name of its owner.
    pub fn effective_auth(&self, request_id: &str) -> (Auth, String) {
        let Some(req) = self.find_request(request_id) else {
            return (self.auth.clone(), self.name.clone());
        };
        if req.request.auth.kind != AuthType::Inherit {
            return (req.request.auth.clone(), req.name.clone());
        }
        let ancestors = self.ancestors(request_id).unwrap_or_default();
        for f in ancestors.iter().rev() {
            if f.auth.kind != AuthType::Inherit {
                return (f.auth.clone(), f.name.clone());
            }
        }
        let auth = if self.auth.kind == AuthType::Inherit { Auth::none() } else { self.auth.clone() };
        (auth, self.name.clone())
    }

    /// Pre-request and post-response scripts for a request, outermost first.
    pub fn script_chain(&self, request_id: &str) -> (Vec<String>, Vec<String>) {
        let mut pre = vec![self.scripts.pre_request.clone()];
        let mut post = vec![self.scripts.post_response.clone()];
        for f in self.ancestors(request_id).unwrap_or_default() {
            pre.push(f.scripts.pre_request.clone());
            post.push(f.scripts.post_response.clone());
        }
        if let Some(r) = self.find_request(request_id) {
            pre.push(r.scripts.pre_request.clone());
            post.push(r.scripts.post_response.clone());
        }
        pre.retain(|s| !s.trim().is_empty());
        post.retain(|s| !s.trim().is_empty());
        (pre, post)
    }

    pub fn request_count(&self) -> usize {
        self.requests().len()
    }
}

impl VariableList for Collection {
    fn vars(&self) -> &Vec<Variable> {
        &self.variables
    }
    fn vars_mut(&mut self) -> &mut Vec<Variable> {
        &mut self.variables
    }
}

fn find_in<'a>(items: &'a [Item], id: &str) -> Option<&'a Item> {
    for i in items {
        if i.id() == id {
            return Some(i);
        }
        if let Item::Folder(f) = i
            && let Some(found) = find_in(&f.items, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_in_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Item> {
    for i in items.iter_mut() {
        if i.id() == id {
            return Some(i);
        }
        if let Item::Folder(f) = i
            && let Some(found) = find_in_mut(&mut f.items, id)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Collection {
        let mut c = Collection::new("Shop");
        c.auth = Auth::new(AuthType::Bearer).with("token", "root");
        let mut f = Folder::new("Users");
        f.scripts.pre_request = "console.log('folder')".into();
        let mut inner = Folder::new("Admin");
        inner.auth = Auth::new(AuthType::Basic).with("username", "a");
        let r1 = RequestItem::new("List", Request::new("get", "{{base}}/users"));
        let r2 = RequestItem::new("Delete", Request::new("DELETE", "{{base}}/users/:id"));
        inner.items.push(Item::Request(r2));
        f.items.push(Item::Request(r1));
        f.items.push(Item::Folder(inner));
        c.items.push(Item::Folder(f));
        c.items.push(Item::Request(RequestItem::new("Health", Request::new("GET", "/health"))));
        c
    }

    #[test]
    fn serde_round_trip() {
        let c = sample();
        let json = serde_json::to_string(&c).unwrap();
        let back: Collection = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn method_is_uppercased_and_path_vars_detected() {
        let c = sample();
        let names: Vec<_> = c.requests().iter().map(|r| r.request.method.clone()).collect();
        assert_eq!(names, ["GET", "DELETE", "GET"]);
        let del = c.requests()[1];
        assert_eq!(del.request.path_variables[0].key, "id");
    }

    #[test]
    fn auth_inheritance_walks_up() {
        let c = sample();
        let reqs = c.requests();
        let (auth, owner) = c.effective_auth(&reqs[0].id);
        assert_eq!(auth.kind, AuthType::Bearer);
        assert_eq!(owner, "Shop");
        let (auth, owner) = c.effective_auth(&reqs[1].id);
        assert_eq!(auth.kind, AuthType::Basic);
        assert_eq!(owner, "Admin");
    }

    #[test]
    fn script_chain_is_outermost_first() {
        let mut c = sample();
        c.scripts.pre_request = "root".into();
        let id = c.requests()[1].id.clone();
        c.find_request_mut(&id).unwrap().scripts.pre_request = "req".into();
        let (pre, post) = c.script_chain(&id);
        assert_eq!(pre, ["root", "console.log('folder')", "req"]);
        assert!(post.is_empty());
    }

    #[test]
    fn move_items_between_folders() {
        let mut c = sample();
        let health = c.requests()[2].id.clone();
        let users = c.items[0].id().to_string();
        let item = c.remove(&health).unwrap();
        assert!(c.insert(Some(&users), 0, item));
        assert_eq!(c.position(&health), Some((Some(users), 0)));
        assert_eq!(c.request_count(), 3);
    }

    #[test]
    fn duplicate_assigns_new_ids() {
        let c = sample();
        let copy = c.items[0].duplicate();
        assert_ne!(copy.id(), c.items[0].id());
        if let (Item::Folder(a), Item::Folder(b)) = (&copy, &c.items[0]) {
            assert_ne!(a.items[0].id(), b.items[0].id());
            assert_eq!(a.items[0].name(), b.items[0].name());
        }
    }

    #[test]
    fn variables_current_overrides_initial() {
        let mut env = Environment::new("Dev");
        env.variables.push(Variable::new("host", "a"));
        assert_eq!(env.get_var("host"), Some("a"));
        env.set_var("host", "b");
        assert_eq!(env.get_var("host"), Some("b"));
        env.reset_all();
        assert_eq!(env.get_var("host"), Some("a"));
        env.set_var("new", "x");
        env.persist_all();
        assert_eq!(env.variables[1].initial, "x");
        env.unset_var("host");
        assert_eq!(env.get_var("host"), None);
    }

    #[test]
    fn bulk_edit_round_trip() {
        let rows = vec![KeyValue::new("a", "1"), KeyValue::new("b", "x:y").disabled()];
        let text = to_bulk(&rows);
        assert_eq!(text, "a:1\n//b:x:y");
        assert_eq!(parse_bulk(&text), rows);
    }

    #[test]
    fn requests_under_folder() {
        let c = sample();
        let users = c.items[0].id().to_string();
        let names: Vec<_> = c.requests_under(Some(&users)).iter().map(|r| r.name.clone()).collect();
        assert_eq!(names, ["List", "Delete"]);
    }
}
