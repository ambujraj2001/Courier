//! A cookie jar following the main rules of RFC 6265 (domain, path, expiry,
//! Secure), persisted to `cookies.json`.

use crate::http::CookieStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lower-case domain without a leading dot.
    pub domain: String,
    /// Only send to exactly `domain` (no `Domain` attribute was given).
    #[serde(default)]
    pub host_only: bool,
    #[serde(default = "root_path")]
    pub path: String,
    #[serde(default)]
    pub expires: Option<DateTime<Utc>>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub same_site: Option<String>,
}

fn root_path() -> String {
    "/".into()
}

impl Cookie {
    pub fn new(name: &str, value: &str, domain: &str) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            domain: domain.trim_start_matches('.').to_lowercase(),
            host_only: false,
            path: "/".into(),
            expires: None,
            secure: false,
            http_only: false,
            same_site: None,
        }
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }

    fn domain_matches(&self, host: &str) -> bool {
        let host = host.to_lowercase();
        if self.host_only {
            host == self.domain
        } else {
            host == self.domain || host.ends_with(&format!(".{}", self.domain))
        }
    }

    fn path_matches(&self, path: &str) -> bool {
        path == self.path
            || (path.starts_with(&self.path) && (self.path.ends_with('/') || path[self.path.len()..].starts_with('/')))
    }

    /// Render as a `Set-Cookie` style string (for display and scripts).
    pub fn to_set_cookie(&self) -> String {
        let mut s = format!("{}={}; Path={}", self.name, self.value, self.path);
        if !self.host_only {
            s.push_str(&format!("; Domain={}", self.domain));
        }
        if let Some(e) = self.expires {
            s.push_str(&format!("; Expires={}", e.format("%a, %d %b %Y %H:%M:%S GMT")));
        }
        if self.secure {
            s.push_str("; Secure");
        }
        if self.http_only {
            s.push_str("; HttpOnly");
        }
        s
    }
}

/// Default cookie path for a request path (RFC 6265 §5.1.4).
fn default_path(path: &str) -> String {
    match path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => path[..i].to_string(),
    }
}

/// Parse one `Set-Cookie` header value received from `url`.
pub fn parse_set_cookie(header: &str, url: &url::Url, now: DateTime<Utc>) -> Option<Cookie> {
    let host = url.host_str()?.to_lowercase();
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut c = Cookie::new(name, value.trim().trim_matches('"'), &host);
    c.host_only = true;
    c.path = default_path(url.path());
    let mut max_age: Option<i64> = None;
    for attr in parts {
        let (k, v) = attr.split_once('=').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((attr.trim(), ""));
        match k.to_ascii_lowercase().as_str() {
            "domain" if !v.is_empty() => {
                let d = v.trim_start_matches('.').to_lowercase();
                // Reject cookies for domains the host does not belong to.
                if host != d && !host.ends_with(&format!(".{d}")) {
                    return None;
                }
                c.domain = d;
                c.host_only = false;
            }
            "path" if v.starts_with('/') => c.path = v.to_string(),
            "expires" => {
                if let Ok(t) = DateTime::parse_from_rfc2822(&v.replace('-', " ")) {
                    c.expires = Some(t.with_timezone(&Utc));
                } else if let Ok(t) = chrono::NaiveDateTime::parse_from_str(v, "%a, %d %b %Y %H:%M:%S GMT") {
                    c.expires = Some(t.and_utc());
                }
            }
            "max-age" => max_age = v.parse().ok(),
            "secure" => c.secure = true,
            "httponly" => c.http_only = true,
            "samesite" => c.same_site = Some(v.to_string()),
            _ => {}
        }
    }
    if let Some(secs) = max_age {
        c.expires = Some(now + Duration::seconds(secs));
    }
    Some(c)
}

/// What gets stored in `cookies.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CookieFile {
    pub cookies: Vec<Cookie>,
    /// Domains whose cookies scripts may access through `pm.cookies.jar()`.
    pub script_allowlist: Vec<String>,
}

#[derive(Debug, Default)]
pub struct CookieJar {
    inner: Mutex<CookieFile>,
}

impl CookieJar {
    pub fn from_file(file: CookieFile) -> Self {
        let jar = Self { inner: Mutex::new(file) };
        jar.purge_expired();
        jar
    }

    pub fn snapshot(&self) -> CookieFile {
        self.inner.lock().unwrap().clone()
    }

    pub fn purge_expired(&self) {
        let now = Utc::now();
        self.inner.lock().unwrap().cookies.retain(|c| !c.is_expired(now));
    }

    /// Insert or replace a cookie (matched on name, domain and path).
    pub fn upsert(&self, cookie: Cookie) {
        let mut inner = self.inner.lock().unwrap();
        inner.cookies.retain(|c| !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path));
        if !cookie.is_expired(Utc::now()) {
            inner.cookies.push(cookie);
        }
    }

    pub fn remove(&self, domain: &str, name: &str, path: &str) {
        self.inner.lock().unwrap().cookies.retain(|c| !(c.domain == domain && c.name == name && c.path == path));
    }

    pub fn clear_domain(&self, domain: &str) {
        self.inner.lock().unwrap().cookies.retain(|c| c.domain != domain);
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().cookies.clear();
    }

    pub fn domains(&self) -> Vec<String> {
        let mut d: Vec<String> = self.inner.lock().unwrap().cookies.iter().map(|c| c.domain.clone()).collect();
        d.sort();
        d.dedup();
        d
    }

    pub fn for_domain(&self, domain: &str) -> Vec<Cookie> {
        self.inner.lock().unwrap().cookies.iter().filter(|c| c.domain == domain).cloned().collect()
    }

    /// Cookies that would be sent to `url`, longest path first.
    pub fn matching(&self, url: &url::Url) -> Vec<Cookie> {
        let Some(host) = url.host_str() else { return Vec::new() };
        let now = Utc::now();
        let secure = url.scheme() == "https";
        let mut out: Vec<Cookie> = self
            .inner
            .lock()
            .unwrap()
            .cookies
            .iter()
            .filter(|c| {
                !c.is_expired(now) && c.domain_matches(host) && c.path_matches(url.path()) && (secure || !c.secure)
            })
            .cloned()
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        out
    }

    pub fn allowlist(&self) -> Vec<String> {
        self.inner.lock().unwrap().script_allowlist.clone()
    }

    pub fn set_allowlist(&self, domains: Vec<String>) {
        self.inner.lock().unwrap().script_allowlist = domains;
    }

    pub fn script_allowed(&self, host: &str) -> bool {
        let host = host.to_lowercase();
        self.inner
            .lock()
            .unwrap()
            .script_allowlist
            .iter()
            .any(|d| host == d.to_lowercase() || host.ends_with(&format!(".{}", d.to_lowercase())))
    }
}

impl CookieStore for CookieJar {
    fn cookie_header(&self, url: &url::Url) -> Option<String> {
        let cookies = self.matching(url);
        (!cookies.is_empty())
            .then(|| cookies.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; "))
    }

    fn store(&self, url: &url::Url, set_cookie: &[String]) {
        let now = Utc::now();
        for header in set_cookie {
            if let Some(c) = parse_set_cookie(header, url, now) {
                if c.is_expired(now) {
                    self.remove(&c.domain, &c.name, &c.path);
                } else {
                    self.upsert(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn host_only_and_domain_cookies() {
        let jar = CookieJar::default();
        jar.store(
            &u("http://api.example.com/v1/login"),
            &["sid=1; Path=/".into(), "wide=2; Domain=.example.com; Path=/".into()],
        );
        assert_eq!(jar.cookie_header(&u("http://api.example.com/x")).as_deref(), Some("sid=1; wide=2"));
        assert_eq!(jar.cookie_header(&u("http://www.example.com/v1/a")).as_deref(), Some("wide=2"));
        assert_eq!(jar.cookie_header(&u("http://other.com/")), None);
    }

    #[test]
    fn default_path_and_path_matching() {
        let jar = CookieJar::default();
        jar.store(&u("http://h/app/login"), &["a=1".into()]);
        assert!(jar.cookie_header(&u("http://h/app/x")).is_some());
        assert!(jar.cookie_header(&u("http://h/application")).is_none());
        assert!(jar.cookie_header(&u("http://h/")).is_none());
    }

    #[test]
    fn secure_expiry_and_deletion() {
        let jar = CookieJar::default();
        jar.store(
            &u("https://h/"),
            &["s=1; Secure".into(), "gone=1; Max-Age=0".into(), "old=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT".into()],
        );
        assert_eq!(jar.cookie_header(&u("http://h/")), None);
        assert_eq!(jar.cookie_header(&u("https://h/")).as_deref(), Some("s=1"));
        jar.store(&u("https://h/"), &["s=1; Max-Age=-1".into()]);
        assert_eq!(jar.cookie_header(&u("https://h/")), None);
    }

    #[test]
    fn rejects_foreign_domain() {
        let jar = CookieJar::default();
        jar.store(&u("http://evil.com/"), &["x=1; Domain=bank.com".into()]);
        assert!(jar.domains().is_empty());
    }

    #[test]
    fn upsert_replaces_and_persists() {
        let jar = CookieJar::default();
        jar.store(&u("http://h/"), &["a=1".into()]);
        jar.store(&u("http://h/"), &["a=2".into()]);
        assert_eq!(jar.for_domain("h").len(), 1);
        assert_eq!(jar.for_domain("h")[0].value, "2");
        let file = jar.snapshot();
        let restored = CookieJar::from_file(serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap());
        assert_eq!(restored.cookie_header(&u("http://h/")).as_deref(), Some("a=2"));
    }

    #[test]
    fn script_allowlist() {
        let jar = CookieJar::default();
        jar.set_allowlist(vec!["example.com".into()]);
        assert!(jar.script_allowed("api.example.com"));
        assert!(!jar.script_allowed("example.org"));
    }
}
