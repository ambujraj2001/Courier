//! Turning a [`Request`] definition into a concrete, variable-resolved
//! [`PreparedRequest`] ready to be signed and sent.

use super::{HttpVersion, PreparedBody, PreparedPart, PreparedRequest, SendOptions};
use crate::model::{BodyMode, FormFieldKind, RawLanguage, Request, url};
use crate::settings::Settings;
use crate::vars::VarContext;
use std::path::{Path, PathBuf};

pub const USER_AGENT: &str = concat!("Courier/", env!("CARGO_PKG_VERSION"));

/// Methods whose body is dropped unless `disableBodyPruning` is set.
const BODYLESS: &[&str] = &["GET", "HEAD", "OPTIONS", "COPY", "PURGE", "UNLOCK", "LINK", "UNLINK", "VIEW"];

/// Headers Courier adds automatically, as (name, value). The user can turn
/// each off through the request's `disabledSystemHeaders` setting.
pub fn auto_headers(req: &Request, settings: &Settings) -> Vec<(String, String)> {
    let mut out = vec![("User-Agent".to_string(), USER_AGENT.to_string()), ("Accept".to_string(), "*/*".to_string())];
    if settings.send_no_cache_header {
        out.push(("Cache-Control".into(), "no-cache".into()));
    }
    out.push(("Accept-Encoding".into(), "gzip, deflate, br".into()));
    out.push(("Connection".into(), "keep-alive".into()));
    if let Some(ct) = default_content_type(req) {
        out.push(("Content-Type".into(), ct));
    }
    let disabled = req.settings.get("disabledSystemHeaders").and_then(|v| v.as_object());
    out.retain(|(name, _)| {
        !disabled.is_some_and(|d| d.get(&name.to_lowercase()).and_then(|v| v.as_bool()).unwrap_or(false))
    });
    out
}

fn default_content_type(req: &Request) -> Option<String> {
    match req.body.mode {
        BodyMode::Raw => Some(req.body.raw_language.content_type().to_string()),
        BodyMode::Urlencoded => Some("application/x-www-form-urlencoded".into()),
        BodyMode::Graphql => Some("application/json".into()),
        // multipart boundaries are set by the HTTP client; binary has none.
        BodyMode::Formdata | BodyMode::File | BodyMode::None => None,
    }
}

/// Add `http://` when the URL has no scheme.
pub fn ensure_scheme(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.contains("://") { trimmed.to_string() } else { format!("http://{trimmed}") }
}

/// Remove `//` and `/* */` comments outside of string literals.
pub fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn resolve_path(path: &str, settings: &Settings) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() || settings.working_dir.is_empty() {
        p.to_path_buf()
    } else {
        Path::new(&settings.working_dir).join(p)
    }
}

/// Request options from global settings plus per-request overrides.
pub fn send_options(req: &Request, settings: &Settings) -> SendOptions {
    let timeout = req.setting_u64("timeout").unwrap_or(settings.request_timeout_ms);
    SendOptions {
        timeout_ms: timeout,
        follow_redirects: req.setting_bool("followRedirects").unwrap_or(settings.follow_redirects),
        max_redirects: req.setting_u64("maxRedirects").map(|n| n as u32).unwrap_or(settings.max_redirects),
        strict_ssl: req.setting_bool("strictSSL").unwrap_or(settings.ssl_verification),
        follow_original_method: req.setting_bool("followOriginalHttpMethod").unwrap_or(false),
        follow_auth_header: req.setting_bool("followAuthorizationHeader").unwrap_or(false),
        remove_referer: req.setting_bool("removeRefererHeaderOnRedirect").unwrap_or(false),
        disable_cookies: req.setting_bool("disableCookies").unwrap_or(false),
        http_version: match req.settings.get("protocolVersion").and_then(|v| v.as_str()) {
            Some("http1") => HttpVersion::Http1,
            Some("http2") => HttpVersion::Http2,
            _ => HttpVersion::Auto,
        },
        max_response_bytes: settings.max_response_bytes(),
        download_to: None,
    }
}

/// Resolve variables and build the outgoing request (before auth).
pub fn prepare(req: &Request, vars: &VarContext, settings: &Settings) -> PreparedRequest {
    let trim = |s: String| if settings.trim_keys_and_values { s.trim().to_string() } else { s };
    let r = |s: &str| trim(vars.resolve(s));

    // URL: resolve, then substitute resolved path variables.
    let path_vars: Vec<_> =
        req.path_variables.iter().map(|p| crate::model::KeyValue { value: r(&p.value), ..p.clone() }).collect();
    let url = ensure_scheme(&url::substitute_path_variables(&r(&req.url), &path_vars));

    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in auto_headers(req, settings) {
        if req.header(&name).is_none() {
            headers.push((name, value));
        }
    }
    for h in req.headers.iter().filter(|h| h.enabled && !h.key.trim().is_empty()) {
        headers.push((r(&h.key), r(&h.value)));
    }
    // Encoding is negotiated by the client; drop our informational entry.
    headers.retain(|(n, _)| !n.eq_ignore_ascii_case("accept-encoding") || req.header(n).is_some());

    let method = req.method.trim().to_uppercase();
    let prune = BODYLESS.contains(&method.as_str()) && !req.setting_bool("disableBodyPruning").unwrap_or(false);
    let body = if prune {
        PreparedBody::None
    } else {
        match req.body.mode {
            BodyMode::None => PreparedBody::None,
            BodyMode::Raw => {
                let text = r(&req.body.raw);
                let text = if req.body.raw_language == RawLanguage::Json { strip_json_comments(&text) } else { text };
                PreparedBody::Bytes(text.into_bytes())
            }
            BodyMode::Urlencoded => {
                let mut ser = ::url::form_urlencoded::Serializer::new(String::new());
                for kv in req.body.urlencoded.iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
                    ser.append_pair(&r(&kv.key), &r(&kv.value));
                }
                PreparedBody::Bytes(ser.finish().into_bytes())
            }
            BodyMode::Formdata => PreparedBody::Multipart(
                req.body
                    .formdata
                    .iter()
                    .filter(|f| f.enabled && !f.key.is_empty())
                    .map(|f| match f.kind {
                        FormFieldKind::Text => PreparedPart::Text {
                            name: r(&f.key),
                            value: r(&f.value),
                            content_type: f.content_type.clone(),
                        },
                        FormFieldKind::File => PreparedPart::Files {
                            name: r(&f.key),
                            paths: f.files.iter().map(|p| resolve_path(&r(p), settings)).collect(),
                            content_type: f.content_type.clone(),
                        },
                    })
                    .collect(),
            ),
            BodyMode::File => match &req.body.file {
                Some(p) if !p.is_empty() => PreparedBody::File(resolve_path(&r(p), settings)),
                _ => PreparedBody::None,
            },
            BodyMode::Graphql => {
                let vars_text = r(&req.body.graphql.variables);
                let variables: serde_json::Value = if vars_text.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&strip_json_comments(&vars_text)).unwrap_or(serde_json::json!({}))
                };
                let payload = serde_json::json!({ "query": r(&req.body.graphql.query), "variables": variables });
                PreparedBody::Bytes(payload.to_string().into_bytes())
            }
        }
    };
    if matches!(body, PreparedBody::None) {
        headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type") || req.header(n).is_some());
    }

    PreparedRequest { method, url, headers, body, options: send_options(req, settings) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Body, FormField, KeyValue, Variable};

    fn vars() -> VarContext {
        let mut v = VarContext::default();
        v.environment = Some(vec![Variable::new("host", "api.test"), Variable::new("id", "42")]);
        v
    }

    #[test]
    fn resolves_url_path_vars_and_headers() {
        let mut req = Request::new("get", "{{host}}/users/:id?x={{id}}");
        req.path_variables[0].value = "{{id}}".into();
        req.headers.push(KeyValue::new("X-Id", "{{id}}"));
        req.headers.push(KeyValue::new("X-Off", "1").disabled());
        let p = prepare(&req, &vars(), &Settings::default());
        assert_eq!(p.method, "GET");
        assert_eq!(p.url, "http://api.test/users/42?x=42");
        assert!(p.headers.contains(&("X-Id".into(), "42".into())));
        assert!(!p.headers.iter().any(|(n, _)| n == "X-Off"));
        assert!(p.headers.iter().any(|(n, v)| n == "User-Agent" && v.starts_with("Courier/")));
        assert!(p.headers.iter().any(|(n, _)| n == "Cache-Control"));
    }

    #[test]
    fn user_header_overrides_auto_header() {
        let mut req = Request::new("POST", "http://x");
        req.body = Body::raw("{}", RawLanguage::Json);
        req.headers.push(KeyValue::new("content-type", "application/vnd.api+json"));
        let p = prepare(&req, &vars(), &Settings::default());
        let cts: Vec<_> = p.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("content-type")).collect();
        assert_eq!(cts.len(), 1);
        assert_eq!(cts[0].1, "application/vnd.api+json");
    }

    #[test]
    fn system_headers_can_be_disabled() {
        let mut req = Request::new("GET", "http://x");
        req.settings.insert("disabledSystemHeaders".into(), serde_json::json!({ "user-agent": true }));
        let p = prepare(&req, &vars(), &Settings::default());
        assert!(!p.headers.iter().any(|(n, _)| n == "User-Agent"));
    }

    #[test]
    fn json_comments_are_stripped_but_strings_kept() {
        let src = "{\n  // note\n  \"url\": \"http://a//b\", /* x */ \"n\": 1\n}";
        let out: serde_json::Value = serde_json::from_str(&strip_json_comments(src)).unwrap();
        assert_eq!(out["url"], "http://a//b");
        assert_eq!(out["n"], 1);
    }

    #[test]
    fn body_modes() {
        let s = Settings::default();
        let mut req = Request::new("POST", "http://x");
        req.body.mode = BodyMode::Urlencoded;
        req.body.urlencoded = vec![KeyValue::new("a b", "{{id}}&"), KeyValue::new("off", "1").disabled()];
        let PreparedBody::Bytes(b) = prepare(&req, &vars(), &s).body else { panic!() };
        assert_eq!(String::from_utf8(b).unwrap(), "a+b=42%26");

        req.body.mode = BodyMode::Graphql;
        req.body.graphql.query = "{ user(id: {{id}}) { name } }".into();
        req.body.graphql.variables = "{\"a\": 1}".into();
        let PreparedBody::Bytes(b) = prepare(&req, &vars(), &s).body else { panic!() };
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["query"], "{ user(id: 42) { name } }");
        assert_eq!(v["variables"]["a"], 1);

        req.body.mode = BodyMode::Formdata;
        req.body.formdata = vec![FormField {
            key: "f".into(),
            kind: FormFieldKind::File,
            files: vec!["a.txt".into()],
            enabled: true,
            ..Default::default()
        }];
        let s2 = Settings { working_dir: "/work".into(), ..Settings::default() };
        let PreparedBody::Multipart(parts) = prepare(&req, &vars(), &s2).body else { panic!() };
        assert!(matches!(&parts[0], PreparedPart::Files { paths, .. } if paths[0] == Path::new("/work/a.txt")));
    }

    #[test]
    fn get_body_is_pruned_unless_disabled() {
        let mut req = Request::new("GET", "http://x");
        req.body = Body::raw("hi", RawLanguage::Text);
        assert!(matches!(prepare(&req, &vars(), &Settings::default()).body, PreparedBody::None));
        req.settings.insert("disableBodyPruning".into(), true.into());
        assert!(matches!(prepare(&req, &vars(), &Settings::default()).body, PreparedBody::Bytes(_)));
    }

    #[test]
    fn per_request_settings_override_globals() {
        let mut req = Request::new("GET", "x");
        req.settings.insert("followRedirects".into(), false.into());
        req.settings.insert("maxRedirects".into(), 3.into());
        req.settings.insert("protocolVersion".into(), "http1".into());
        let o = send_options(&req, &Settings::default());
        assert!(!o.follow_redirects);
        assert_eq!(o.max_redirects, 3);
        assert_eq!(o.http_version, HttpVersion::Http1);
        assert!(o.strict_ssl);
    }
}
