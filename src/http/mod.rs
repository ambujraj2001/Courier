//! HTTP engine built on `reqwest`: client configuration, manual redirect
//! handling, cookies, size limits, timing, cancellation and downloads.

pub mod prepare;

use crate::settings::Settings;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub use prepare::{auto_headers, prepare, send_options};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum HttpVersion {
    #[default]
    Auto,
    Http1,
    Http2,
}

/// Per-send options derived from settings and the request's own overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct SendOptions {
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub strict_ssl: bool,
    pub follow_original_method: bool,
    pub follow_auth_header: bool,
    pub remove_referer: bool,
    pub disable_cookies: bool,
    pub http_version: HttpVersion,
    pub max_response_bytes: usize,
    /// Stream the body to this file instead of keeping it in memory.
    pub download_to: Option<PathBuf>,
}

impl Default for SendOptions {
    fn default() -> Self {
        prepare::send_options(&crate::model::Request::default(), &Settings::default())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreparedPart {
    Text { name: String, value: String, content_type: Option<String> },
    Files { name: String, paths: Vec<PathBuf>, content_type: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreparedBody {
    None,
    Bytes(Vec<u8>),
    Multipart(Vec<PreparedPart>),
    File(PathBuf),
}

impl PreparedBody {
    /// Body bytes when they are known up front (used by signers).
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            PreparedBody::Bytes(b) => Some(b),
            PreparedBody::None => Some(&[]),
            _ => None,
        }
    }
}

/// A fully resolved request ready to go on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    pub options: SendOptions,
}

impl PreparedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Set a header, replacing any existing values with the same name.
    pub fn set_header(&mut self, name: &str, value: impl Into<String>) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.into()));
    }

    pub fn remove_header(&mut self, name: &str) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    }

    /// Append a query parameter (already-encoded `key=value`).
    pub fn add_query(&mut self, key: &str, value: &str) {
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, QUERY_ENCODE).to_string();
        let sep = if self.url.contains('?') { '&' } else { '?' };
        let (base, frag) = match self.url.find('#') {
            Some(i) => (self.url[..i].to_string(), self.url[i..].to_string()),
            None => (self.url.clone(), String::new()),
        };
        self.url = format!("{base}{sep}{}={}{frag}", enc(key), enc(value));
    }
}

const QUERY_ENCODE: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Time until response headers arrived.
    pub ttfb_ms: f64,
    /// Time spent downloading the body.
    pub download_ms: f64,
    pub total_ms: f64,
}

/// A received response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    #[serde(with = "body_b64")]
    pub body: Vec<u8>,
    /// True when the body exceeded the size limit and was cut off.
    pub truncated: bool,
    /// Total body bytes received (may exceed `body.len()` when truncated).
    pub body_size: u64,
    pub header_size: u64,
    pub timings: Timings,
    pub final_url: String,
    /// URLs visited before the final one when redirects were followed.
    pub redirects: Vec<String>,
    /// Where the body was saved for "Send and Download".
    pub downloaded_to: Option<PathBuf>,
}

mod body_b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(bytes) {
            Ok(text) => s.serialize_str(text),
            Err(_) => s.serialize_str(&format!("base64:{}", STANDARD.encode(bytes))),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.strip_prefix("base64:") {
            Some(b) => STANDARD.decode(b).unwrap_or_else(|_| s.clone().into_bytes()),
            None => s.into_bytes(),
        })
    }
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn header_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers.iter().filter(move |(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }

    pub fn total_size(&self) -> u64 {
        self.body_size + self.header_size
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum HttpError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("invalid header {0}")]
    InvalidHeader(String),
    #[error("could not read file {0}: {1}")]
    File(String, String),
    #[error("request timed out after {0} ms")]
    Timeout(u64),
    #[error("request cancelled")]
    Cancelled,
    #[error("too many redirects (limit {0})")]
    TooManyRedirects(u32),
    #[error("{0}")]
    Network(String),
}

/// Cookie storage used during a send. Implemented by [`crate::cookies::CookieJar`].
pub trait CookieStore: Send + Sync {
    fn cookie_header(&self, url: &url::Url) -> Option<String>;
    fn store(&self, url: &url::Url, set_cookie: &[String]);
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    strict_ssl: bool,
    http_version: HttpVersion,
    proxy: String,
    use_env_proxy: bool,
    ca_cert: Option<String>,
    identity: Option<(String, String)>,
}

/// Builds and caches `reqwest` clients for each distinct configuration.
#[derive(Clone, Default)]
pub struct HttpEngine {
    clients: Arc<Mutex<HashMap<ClientKey, reqwest::Client>>>,
    settings: Arc<Mutex<Settings>>,
}

impl HttpEngine {
    pub fn new(settings: Settings) -> Self {
        Self { clients: Default::default(), settings: Arc::new(Mutex::new(settings)) }
    }

    pub fn set_settings(&self, settings: Settings) {
        *self.settings.lock().unwrap() = settings;
        self.clients.lock().unwrap().clear();
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn client_for(&self, url: &url::Url, opts: &SendOptions) -> Result<reqwest::Client, HttpError> {
        let settings = self.settings();
        let host = url.host_str().unwrap_or_default();
        let identity = settings
            .client_certs
            .iter()
            .find(|c| {
                c.enabled
                    && c.host.eq_ignore_ascii_case(host)
                    && c.port.is_none_or(|p| Some(p) == url.port_or_known_default())
            })
            .map(|c| (c.cert_path.clone(), c.key_path.clone()));
        let proxy = if settings.proxy.custom_enabled { settings.proxy.custom_url.clone() } else { String::new() };
        let key = ClientKey {
            strict_ssl: opts.strict_ssl,
            http_version: opts.http_version,
            proxy,
            use_env_proxy: settings.proxy.use_env_vars || settings.proxy.use_system,
            ca_cert: (settings.ca_cert_enabled && !settings.ca_cert_path.is_empty())
                .then(|| settings.ca_cert_path.clone()),
            identity,
        };
        if let Some(c) = self.clients.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let mut b = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(!key.strict_ssl)
            .pool_idle_timeout(Duration::from_secs(60));
        b = match key.http_version {
            HttpVersion::Http1 => b.http1_only(),
            HttpVersion::Http2 => b.http2_prior_knowledge(),
            HttpVersion::Auto => b,
        };
        if !key.proxy.is_empty() {
            let mut p = reqwest::Proxy::all(&key.proxy).map_err(|e| HttpError::Network(format!("proxy: {e}")))?;
            if !settings.proxy.username.is_empty() {
                p = p.basic_auth(&settings.proxy.username, &settings.proxy.password);
            }
            if !settings.proxy.bypass.trim().is_empty() {
                p = p.no_proxy(reqwest::NoProxy::from_string(&settings.proxy.bypass));
            }
            b = b.proxy(p);
        } else if !key.use_env_proxy {
            b = b.no_proxy();
        }
        if let Some(path) = &key.ca_cert {
            let pem = std::fs::read(path).map_err(|e| HttpError::File(path.clone(), e.to_string()))?;
            let cert =
                reqwest::Certificate::from_pem(&pem).map_err(|e| HttpError::Network(format!("CA certificate: {e}")))?;
            b = b.add_root_certificate(cert);
        }
        if let Some((cert, key_path)) = &key.identity {
            let mut pem = std::fs::read(cert).map_err(|e| HttpError::File(cert.clone(), e.to_string()))?;
            if !key_path.is_empty() {
                pem.push(b'\n');
                pem.extend(std::fs::read(key_path).map_err(|e| HttpError::File(key_path.clone(), e.to_string()))?);
            }
            let id = reqwest::Identity::from_pem(&pem)
                .map_err(|e| HttpError::Network(format!("client certificate: {e}")))?;
            b = b.identity(id);
        }
        let client = b.build().map_err(|e| HttpError::Network(e.to_string()))?;
        self.clients.lock().unwrap().insert(key, client.clone());
        Ok(client)
    }

    /// Send a prepared request, following redirects according to its options.
    pub async fn send(
        &self,
        req: &PreparedRequest,
        cookies: Option<&dyn CookieStore>,
        cancel: &CancellationToken,
    ) -> Result<HttpResponse, HttpError> {
        let started = Instant::now();
        let opts = &req.options;
        let fut = self.send_inner(req, cookies, started);
        let timed = async {
            if opts.timeout_ms > 0 {
                match tokio::time::timeout(Duration::from_millis(opts.timeout_ms), fut).await {
                    Ok(r) => r,
                    Err(_) => Err(HttpError::Timeout(opts.timeout_ms)),
                }
            } else {
                fut.await
            }
        };
        tokio::select! {
            r = timed => r,
            _ = cancel.cancelled() => Err(HttpError::Cancelled),
        }
    }

    async fn send_inner(
        &self,
        req: &PreparedRequest,
        cookies: Option<&dyn CookieStore>,
        started: Instant,
    ) -> Result<HttpResponse, HttpError> {
        let opts = &req.options;
        let mut url = url::Url::parse(&req.url).map_err(|e| HttpError::InvalidUrl(format!("{}: {e}", req.url)))?;
        let original_host = url.host_str().map(str::to_string);
        let mut method = req.method.clone();
        let mut body = Some(req.body.clone());
        let mut headers = req.headers.clone();
        let mut redirects = Vec::new();
        loop {
            let client = self.client_for(&url, opts)?;
            let m = reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|_| HttpError::InvalidUrl(format!("invalid method {method}")))?;
            let mut header_map = HeaderMap::new();
            for (n, v) in &headers {
                let name = HeaderName::from_bytes(n.as_bytes()).map_err(|_| HttpError::InvalidHeader(n.clone()))?;
                let value = HeaderValue::from_str(v).map_err(|_| HttpError::InvalidHeader(format!("{n}: {v}")))?;
                header_map.append(name, value);
            }
            if !opts.disable_cookies
                && !header_map.contains_key(reqwest::header::COOKIE)
                && let Some(c) = cookies.and_then(|c| c.cookie_header(&url))
                && let Ok(v) = HeaderValue::from_str(&c)
            {
                header_map.insert(reqwest::header::COOKIE, v);
            }
            let mut rb = client.request(m, url.clone()).headers(header_map);
            rb = match body.as_ref().unwrap_or(&PreparedBody::None) {
                PreparedBody::None => rb,
                PreparedBody::Bytes(b) => rb.body(b.clone()),
                PreparedBody::File(p) => {
                    let data = tokio::fs::read(p)
                        .await
                        .map_err(|e| HttpError::File(p.display().to_string(), e.to_string()))?;
                    rb.body(data)
                }
                PreparedBody::Multipart(parts) => rb.multipart(build_form(parts).await?),
            };
            let resp = rb.send().await.map_err(|e| classify(&e))?;
            let ttfb = started.elapsed();

            if !opts.disable_cookies
                && let Some(store) = cookies
            {
                let set: Vec<String> = resp
                    .headers()
                    .get_all(reqwest::header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_string))
                    .collect();
                if !set.is_empty() {
                    store.store(&url, &set);
                }
            }

            let status = resp.status();
            if status.is_redirection()
                && opts.follow_redirects
                && let Some(loc) = resp.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok())
            {
                if redirects.len() as u32 >= opts.max_redirects {
                    return Err(HttpError::TooManyRedirects(opts.max_redirects));
                }
                let next = url.join(loc).map_err(|e| HttpError::InvalidUrl(format!("{loc}: {e}")))?;
                redirects.push(url.to_string());
                let code = status.as_u16();
                let keep_method = code == 307 || code == 308 || opts.follow_original_method;
                if !keep_method && method != "HEAD" {
                    method = "GET".into();
                    body = None;
                    headers.retain(|(n, _)| {
                        !n.eq_ignore_ascii_case("content-type") && !n.eq_ignore_ascii_case("content-length")
                    });
                }
                if next.host_str().map(str::to_string) != original_host && !opts.follow_auth_header {
                    headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
                }
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("cookie") && !n.eq_ignore_ascii_case("host"));
                if opts.remove_referer {
                    headers.retain(|(n, _)| !n.eq_ignore_ascii_case("referer"));
                } else {
                    headers.retain(|(n, _)| !n.eq_ignore_ascii_case("referer"));
                    headers.push(("Referer".into(), url.to_string()));
                }
                url = next;
                continue;
            }
            return read_response(resp, opts, started, ttfb, redirects).await;
        }
    }
}

async fn build_form(parts: &[PreparedPart]) -> Result<reqwest::multipart::Form, HttpError> {
    let mut form = reqwest::multipart::Form::new();
    for part in parts {
        match part {
            PreparedPart::Text { name, value, content_type } => {
                let mut p = reqwest::multipart::Part::text(value.clone());
                if let Some(ct) = content_type {
                    p = p.mime_str(ct).map_err(|e| HttpError::InvalidHeader(format!("{ct}: {e}")))?;
                }
                form = form.part(name.clone(), p);
            }
            PreparedPart::Files { name, paths, content_type } => {
                for path in paths {
                    let data = tokio::fs::read(path)
                        .await
                        .map_err(|e| HttpError::File(path.display().to_string(), e.to_string()))?;
                    let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut p = reqwest::multipart::Part::bytes(data).file_name(file_name);
                    let ct = content_type.clone().unwrap_or_else(|| guess_mime(path).to_string());
                    p = p.mime_str(&ct).map_err(|e| HttpError::InvalidHeader(format!("{ct}: {e}")))?;
                    form = form.part(name.clone(), p);
                }
            }
        }
    }
    Ok(form)
}

/// Minimal extension-based MIME guess for uploaded files.
pub fn guess_mime(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("xml") => "application/xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn classify(e: &reqwest::Error) -> HttpError {
    let mut msg = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        msg = format!("{msg}: {s}");
        source = s.source();
    }
    HttpError::Network(msg)
}

fn version_name(v: reqwest::Version) -> &'static str {
    match v {
        reqwest::Version::HTTP_09 => "HTTP/0.9",
        reqwest::Version::HTTP_10 => "HTTP/1.0",
        reqwest::Version::HTTP_11 => "HTTP/1.1",
        reqwest::Version::HTTP_2 => "HTTP/2",
        reqwest::Version::HTTP_3 => "HTTP/3",
        _ => "HTTP",
    }
}

async fn read_response(
    resp: reqwest::Response,
    opts: &SendOptions,
    started: Instant,
    ttfb: Duration,
    redirects: Vec<String>,
) -> Result<HttpResponse, HttpError> {
    let status = resp.status();
    let http_version = version_name(resp.version()).to_string();
    let final_url = resp.url().to_string();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(n, v)| (n.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let header_size = headers.iter().map(|(n, v)| (n.len() + v.len() + 4) as u64).sum::<u64>()
        + (http_version.len() + 6 + status.canonical_reason().unwrap_or("").len()) as u64;

    let mut body = Vec::new();
    let mut body_size = 0u64;
    let mut truncated = false;
    let mut file = match &opts.download_to {
        Some(p) => Some(
            tokio::fs::File::create(p).await.map_err(|e| HttpError::File(p.display().to_string(), e.to_string()))?,
        ),
        None => None,
    };
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify(&e))?;
        body_size += chunk.len() as u64;
        if let Some(f) = file.as_mut() {
            f.write_all(&chunk).await.map_err(|e| HttpError::File("download".into(), e.to_string()))?;
            continue;
        }
        let room = opts.max_response_bytes.saturating_sub(body.len());
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            truncated = true;
        } else {
            body.extend_from_slice(&chunk);
        }
    }
    if let Some(mut f) = file {
        f.flush().await.map_err(|e| HttpError::File("download".into(), e.to_string()))?;
    }
    let total = started.elapsed();
    Ok(HttpResponse {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        http_version,
        headers,
        body,
        truncated,
        body_size,
        header_size,
        timings: Timings {
            ttfb_ms: ttfb.as_secs_f64() * 1000.0,
            download_ms: (total - ttfb).as_secs_f64() * 1000.0,
            total_ms: total.as_secs_f64() * 1000.0,
        },
        final_url,
        redirects,
        downloaded_to: opts.download_to.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_query_encodes_and_keeps_fragment() {
        let mut p = PreparedRequest {
            method: "GET".into(),
            url: "http://x/a?b=1#f".into(),
            headers: vec![],
            body: PreparedBody::None,
            options: SendOptions::default(),
        };
        p.add_query("api key", "a&b");
        assert_eq!(p.url, "http://x/a?b=1&api%20key=a%26b#f");
        p.set_header("X-A", "1");
        p.set_header("x-a", "2");
        assert_eq!(p.header("X-A"), Some("2"));
        assert_eq!(p.headers.len(), 1);
    }

    #[test]
    fn response_body_serializes_binary_safely() {
        let r = HttpResponse { body: vec![0xff, 0x00, 0x10], ..Default::default() };
        let json = serde_json::to_string(&r).unwrap();
        let back: HttpResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.body, r.body);
        let t = HttpResponse { body: b"hello".to_vec(), ..Default::default() };
        assert!(serde_json::to_string(&t).unwrap().contains("\"hello\""));
    }
}
