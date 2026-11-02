//! Parse cURL command lines into requests.

use crate::model::*;

/// Result of parsing one or more cURL commands.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedCurl {
    pub requests: Vec<RequestItem>,
    pub warnings: Vec<String>,
}

/// Split a shell command line into words, honouring quotes, escapes and
/// line continuations (POSIX `\` and Windows `^`).
pub fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let text = input.replace("\r\n", "\n").replace("\\\n", " ").replace("^\n", " ");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
                if c == '\n' {
                    out.push("\n".into());
                }
            }
            '\'' => {
                has_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("unterminated single quote".into()),
                    }
                }
            }
            '"' => {
                has_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\' | '$' | '`')) => cur.push(e),
                            Some('\n') => {}
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err("unterminated double quote".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("unterminated double quote".into()),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                has_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some('x') => {
                                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    cur.push(b as char);
                                }
                            }
                            Some(e) => cur.push(e),
                            None => return Err("unterminated $'...' string".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("unterminated $'...' string".into()),
                    }
                }
            }
            '\\' => {
                has_token = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            _ => {
                has_token = true;
                cur.push(c);
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    Ok(out)
}

const SHORT_WITH_VALUE: &str = "AbcCdDeEFHKmorPQtTuUwxXyYz";

const LONG_WITH_VALUE: &[&str] = &[
    "request",
    "url",
    "header",
    "data",
    "data-raw",
    "data-ascii",
    "data-binary",
    "data-urlencode",
    "form",
    "form-string",
    "user",
    "user-agent",
    "referer",
    "cookie",
    "cookie-jar",
    "max-time",
    "max-redirs",
    "output",
    "upload-file",
    "proxy",
    "proxy-user",
    "connect-timeout",
    "cert",
    "key",
    "cacert",
    "capath",
    "write-out",
    "range",
    "retry",
    "retry-delay",
    "retry-max-time",
    "oauth2-bearer",
    "json",
    "aws-sigv4",
    "resolve",
    "connect-to",
    "interface",
    "limit-rate",
    "config",
    "dump-header",
    "trace",
    "trace-ascii",
    "stderr",
    "cert-type",
    "key-type",
    "pass",
    "ciphers",
    "tls-max",
    "proxy-header",
    "request-target",
    "unix-socket",
    "variable",
    "expect100-timeout",
    "keepalive-time",
    "local-port",
    "max-filesize",
    "speed-limit",
    "speed-time",
];

fn short_to_long(c: char) -> &'static str {
    match c {
        'A' => "user-agent",
        'b' => "cookie",
        'c' => "cookie-jar",
        'd' => "data",
        'e' => "referer",
        'F' => "form",
        'G' => "get",
        'H' => "header",
        'I' => "head",
        'k' => "insecure",
        'L' => "location",
        'm' => "max-time",
        'o' => "output",
        'T' => "upload-file",
        'u' => "user",
        'x' => "proxy",
        'X' => "request",
        'i' => "include",
        's' => "silent",
        'S' => "show-error",
        'v' => "verbose",
        'f' => "fail",
        'g' => "globoff",
        'O' => "remote-name",
        'J' => "remote-header-name",
        'N' => "no-buffer",
        'n' => "netrc",
        'Z' => "parallel",
        _ => "",
    }
}

#[derive(Default)]
struct Builder {
    method: Option<String>,
    url: Option<String>,
    headers: Vec<KeyValue>,
    data: Vec<(String, String)>, // (kind, value)
    form: Vec<FormField>,
    upload: Option<String>,
    get: bool,
    head: bool,
    auth: Option<Auth>,
    auth_kind: Option<AuthType>,
    settings: JsonMap,
    json: bool,
}

fn split_header(h: &str) -> Option<KeyValue> {
    if let Some((k, v)) = h.split_once(':') {
        let v = v.trim();
        // `Name:` with no value removes a header in curl; skip it.
        if v.is_empty() {
            return None;
        }
        Some(KeyValue::new(k.trim(), v))
    } else {
        h.strip_suffix(';').map(|k| KeyValue::new(k.trim(), ""))
    }
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned()
}

fn looks_like_json(s: &str) -> bool {
    let t = s.trim_start();
    (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<serde_json::Value>(s).is_ok()
}

impl Builder {
    fn finish(mut self, warnings: &mut Vec<String>) -> Option<RequestItem> {
        let url = self.url.take()?;
        let mut req = Request::default();
        req.headers = self.headers;
        let has_data = !self.data.is_empty() || !self.form.is_empty() || self.upload.is_some();

        if self.json {
            if req.header("content-type").is_none() {
                req.headers.push(KeyValue::new("Content-Type", "application/json"));
            }
            if req.header("accept").is_none() {
                req.headers.push(KeyValue::new("Accept", "application/json"));
            }
        }

        let mut url = url;
        if self.get && !self.data.is_empty() {
            let q: Vec<String> = self.data.iter().map(|(_, v)| v.clone()).collect();
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&q.join("&"));
            self.data.clear();
        }
        req.set_url(&url);

        if !self.form.is_empty() {
            req.body.mode = BodyMode::Formdata;
            req.body.formdata = self.form;
        } else if let Some(path) = self.upload {
            req.body.mode = BodyMode::File;
            req.body.file = Some(path);
        } else if !self.data.is_empty() {
            let file_ref = self.data.iter().find_map(|(_, v)| v.strip_prefix('@').map(str::to_string));
            let content_type = req.header("content-type").unwrap_or("").to_ascii_lowercase();
            if let (Some(path), 1) = (file_ref, self.data.len()) {
                req.body.mode = BodyMode::File;
                req.body.file = Some(path);
            } else if self.data.iter().all(|(kind, _)| kind == "data-urlencode") {
                // Values were given unencoded; curl encodes the part after '='.
                req.body.mode = BodyMode::Urlencoded;
                req.body.urlencoded = self
                    .data
                    .iter()
                    .map(|(_, v)| {
                        let (k, val) = v.split_once('=').unwrap_or(("", v));
                        KeyValue::new(k, val)
                    })
                    .collect();
            } else {
                let joined = self.data.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join("&");
                let form_like =
                    content_type.is_empty() || content_type.starts_with("application/x-www-form-urlencoded");
                let pairs_ok = joined.split('&').all(|p| p.contains('=') && !p.trim_start().starts_with('{'));
                if form_like && pairs_ok && !looks_like_json(&joined) {
                    req.body.mode = BodyMode::Urlencoded;
                    req.body.urlencoded = joined
                        .split('&')
                        .filter(|p| !p.is_empty())
                        .map(|p| {
                            let (k, v) = p.split_once('=').unwrap_or((p, ""));
                            KeyValue::new(decode(k), decode(v))
                        })
                        .collect();
                } else {
                    let language = if content_type.contains("json") || looks_like_json(&joined) {
                        RawLanguage::Json
                    } else if content_type.contains("xml") {
                        RawLanguage::Xml
                    } else if content_type.contains("html") {
                        RawLanguage::Html
                    } else if content_type.contains("javascript") {
                        RawLanguage::Javascript
                    } else {
                        RawLanguage::Text
                    };
                    req.body = Body::raw(joined, language);
                }
            }
        }

        req.method = match self.method {
            Some(m) => m.to_uppercase(),
            None if self.head => "HEAD".into(),
            None if req.body.mode == BodyMode::File && req.body.file.is_some() && self.data.is_empty() => "PUT".into(),
            None if has_data && !self.get => "POST".into(),
            None => "GET".into(),
        };
        if let Some(mut auth) = self.auth {
            if let (Some(kind), AuthType::Basic) = (self.auth_kind.clone(), &auth.kind) {
                auth.kind = kind;
                if !auth.kind.is_supported() {
                    warnings.push(format!("{} auth was imported but is not supported for sending", auth.kind.label()));
                }
            }
            req.auth = auth;
        }
        req.settings = self.settings;
        let name = {
            let path = crate::model::url::split(&req.url).0;
            let after_host = path.split_once("://").map(|(_, r)| r).unwrap_or(path);
            match after_host.split_once('/') {
                Some((_, p)) if !p.is_empty() => format!("{} /{p}", req.method),
                _ => format!("{} {after_host}", req.method),
            }
        };
        Some(RequestItem::new(name, req))
    }
}

/// Parse text containing one or more `curl` commands.
pub fn parse(input: &str) -> Result<ParsedCurl, String> {
    let tokens = tokenize(input.trim())?;
    let mut parsed = ParsedCurl::default();
    let mut b: Option<Builder> = None;
    let mut iter = tokens.into_iter().peekable();
    let mut expect_curl = true;

    while let Some(tok) = iter.next() {
        if tok == "\n" {
            expect_curl = true;
            continue;
        }
        if expect_curl || b.is_none() {
            if tok == "curl" || tok.ends_with("/curl") || tok == "curl.exe" {
                if let Some(done) = b.take() {
                    parsed.requests.extend(done.finish(&mut parsed.warnings));
                }
                b = Some(Builder::default());
                expect_curl = false;
                continue;
            }
            if b.is_none() {
                if tok.starts_with('$') || tok == "sudo" {
                    continue;
                }
                return Err("input does not start with `curl`".into());
            }
        }
        expect_curl = false;
        let builder = b.as_mut().expect("builder exists");

        // Normalise to (long name, inline value).
        let (name, inline): (String, Option<String>) = if let Some(long) = tok.strip_prefix("--") {
            match long.split_once('=') {
                Some((n, v)) if LONG_WITH_VALUE.contains(&n) => (n.to_string(), Some(v.to_string())),
                _ => (long.to_string(), None),
            }
        } else if tok.len() > 1 && tok.starts_with('-') && !tok.starts_with("--") {
            let flags: Vec<char> = tok[1..].chars().collect();
            let mut result = (String::new(), None);
            for (i, c) in flags.iter().enumerate() {
                if SHORT_WITH_VALUE.contains(*c) {
                    let rest: String = flags[i + 1..].iter().collect();
                    let long = short_to_long(*c);
                    let long = if long.is_empty() { "unknown-short" } else { long };
                    result = (long.to_string(), (!rest.is_empty()).then_some(rest));
                    break;
                }
                let long = short_to_long(*c);
                if i == flags.len() - 1 {
                    result = (long.to_string(), None);
                } else {
                    apply_flag(builder, long, None, &mut parsed.warnings);
                }
            }
            result
        } else {
            if builder.url.is_none() {
                builder.url = Some(tok);
            } else {
                parsed.warnings.push(format!("ignored extra argument `{tok}`"));
            }
            continue;
        };

        let needs_value = LONG_WITH_VALUE.contains(&name.as_str()) || name == "unknown-short";
        let value = if needs_value {
            match inline {
                Some(v) => Some(v),
                None => match iter.peek() {
                    Some(t) if t != "\n" => iter.next(),
                    _ => None,
                },
            }
        } else {
            None
        };
        if name == "next" {
            let done = b.take().expect("builder");
            parsed.requests.extend(done.finish(&mut parsed.warnings));
            b = Some(Builder::default());
            continue;
        }
        apply_flag(b.as_mut().expect("builder"), &name, value, &mut parsed.warnings);
    }
    if let Some(done) = b.take() {
        parsed.requests.extend(done.finish(&mut parsed.warnings));
    }
    if parsed.requests.is_empty() {
        return Err("no URL found in the cURL command".into());
    }
    Ok(parsed)
}

fn apply_flag(b: &mut Builder, name: &str, value: Option<String>, warnings: &mut Vec<String>) {
    let v = value.clone().unwrap_or_default();
    match name {
        "request" => b.method = Some(v),
        "url" => b.url = Some(v),
        "header" => {
            if let Some(h) = split_header(&v) {
                b.headers.push(h);
            }
        }
        "user-agent" => b.headers.push(KeyValue::new("User-Agent", v)),
        "referer" => b.headers.push(KeyValue::new("Referer", v)),
        "cookie" => {
            if v.contains('=') {
                b.headers.push(KeyValue::new("Cookie", v));
            } else {
                warnings.push(format!("cookie file `{v}` was not imported"));
            }
        }
        "data" | "data-ascii" | "data-raw" | "data-binary" | "data-urlencode" => {
            b.data.push((name.to_string(), v));
        }
        "json" => {
            b.json = true;
            b.data.push(("json".into(), v));
        }
        "form" | "form-string" => {
            let (key, rest) = v.split_once('=').unwrap_or((&v, ""));
            let (val, ct) = match rest.split_once(";type=") {
                Some((a, t)) => (a.to_string(), Some(t.to_string())),
                None => (rest.to_string(), None),
            };
            let file = if name == "form" { val.strip_prefix('@').or_else(|| val.strip_prefix('<')) } else { None };
            b.form.push(match file {
                Some(path) => FormField {
                    key: key.into(),
                    kind: FormFieldKind::File,
                    files: vec![path.trim_matches('"').to_string()],
                    content_type: ct,
                    enabled: true,
                    ..Default::default()
                },
                None => {
                    FormField { key: key.into(), value: val, content_type: ct, enabled: true, ..Default::default() }
                }
            });
        }
        "get" => b.get = true,
        "head" => b.head = true,
        "user" => {
            let (u, p) = v.split_once(':').unwrap_or((&v, ""));
            b.auth = Some(Auth::new(AuthType::Basic).with("username", u).with("password", p));
        }
        "basic" => b.auth_kind = Some(AuthType::Basic),
        "digest" => b.auth_kind = Some(AuthType::Digest),
        "ntlm" | "ntlm-wb" => b.auth_kind = Some(AuthType::Ntlm),
        "oauth2-bearer" => b.auth = Some(Auth::new(AuthType::Bearer).with("token", v)),
        "insecure" => {
            b.settings.insert("strictSSL".into(), false.into());
        }
        "location" | "location-trusted" => {
            b.settings.insert("followRedirects".into(), true.into());
            if name == "location-trusted" {
                b.settings.insert("followAuthorizationHeader".into(), true.into());
            }
        }
        "max-redirs" => {
            if let Ok(n) = v.parse::<u64>() {
                b.settings.insert("maxRedirects".into(), n.into());
            }
        }
        "max-time" => {
            if let Ok(secs) = v.parse::<f64>() {
                b.settings.insert("timeout".into(), ((secs * 1000.0) as u64).into());
            }
        }
        "compressed" => {
            if !b.headers.iter().any(|h| h.key.eq_ignore_ascii_case("accept-encoding")) {
                b.headers.push(KeyValue::new("Accept-Encoding", "deflate, gzip, br"));
            }
        }
        "http1.1" | "http1.0" => {
            b.settings.insert("protocolVersion".into(), "http1".into());
        }
        "http2" | "http2-prior-knowledge" => {
            b.settings.insert("protocolVersion".into(), "http2".into());
        }
        "upload-file" => b.upload = Some(v),
        "include" | "silent" | "show-error" | "verbose" | "fail" | "globoff" | "no-buffer" | "progress-bar"
        | "output" | "remote-name" | "remote-header-name" | "write-out" | "dump-header" | "trace" | "trace-ascii"
        | "stderr" | "netrc" | "path-as-is" | "fail-with-body" | "no-progress-meter" | "parallel" => {}
        "" => {}
        other => {
            let suffix = value.map(|v| format!(" {v}")).unwrap_or_default();
            warnings.push(format!("ignored unsupported option --{other}{suffix}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(cmd: &str) -> Request {
        parse(cmd).unwrap().requests.remove(0).request
    }

    #[test]
    fn tokenizer_handles_quotes_and_continuations() {
        let t = tokenize("curl 'a b' \"c \\\"d\\\"\" $'e\\nf' g\\ h \\\n  --x").unwrap();
        assert_eq!(t, ["curl", "a b", "c \"d\"", "e\nf", "g h", "--x"]);
        assert!(tokenize("curl 'oops").is_err());
    }

    #[test]
    fn simple_get_with_headers() {
        let r = one("curl https://api.example.com/users?page=2 -H 'Accept: application/json' -H \"X-Empty;\"");
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "https://api.example.com/users?page=2");
        assert_eq!(r.query[0].key, "page");
        assert_eq!(r.header("accept"), Some("application/json"));
        assert_eq!(r.headers[1].key, "X-Empty");
    }

    #[test]
    fn json_post() {
        let r = one(r#"curl -X POST https://x.io/a -H 'Content-Type: application/json' --data-raw '{"a": 1}'"#);
        assert_eq!(r.method, "POST");
        assert_eq!(r.body.mode, BodyMode::Raw);
        assert_eq!(r.body.raw_language, RawLanguage::Json);
        assert_eq!(r.body.raw, r#"{"a": 1}"#);
        // Method is implied by data.
        assert_eq!(one(r#"curl https://x.io -d '{"a":1}'"#).body.raw_language, RawLanguage::Json);
    }

    #[test]
    fn urlencoded_and_get_flag() {
        let r = one("curl https://x.io/login -d 'user=a%40b.c' -d 'pass=p+w'");
        assert_eq!(r.method, "POST");
        assert_eq!(r.body.mode, BodyMode::Urlencoded);
        assert_eq!(r.body.urlencoded, vec![KeyValue::new("user", "a@b.c"), KeyValue::new("pass", "p w")]);
        let r = one("curl -G https://x.io/search -d q=rust -d page=2");
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "https://x.io/search?q=rust&page=2");
        let r = one("curl https://x.io --data-urlencode 'msg=hello world&more'");
        assert_eq!(r.body.urlencoded[0].value, "hello world&more");
    }

    #[test]
    fn forms_files_and_upload() {
        let r = one("curl https://x.io/up -F 'caption=hi' -F 'file=@/tmp/a.png;type=image/png'");
        assert_eq!(r.body.mode, BodyMode::Formdata);
        assert_eq!(r.body.formdata[1].kind, FormFieldKind::File);
        assert_eq!(r.body.formdata[1].files, ["/tmp/a.png"]);
        assert_eq!(r.body.formdata[1].content_type.as_deref(), Some("image/png"));
        let r = one("curl https://x.io/blob --data-binary @/tmp/b.bin");
        assert_eq!(r.body.mode, BodyMode::File);
        assert_eq!(r.body.file.as_deref(), Some("/tmp/b.bin"));
        assert_eq!(r.method, "POST");
        let r = one("curl -T ./f.txt https://x.io/put");
        assert_eq!(r.method, "PUT");
    }

    #[test]
    fn auth_and_settings_flags() {
        let r = one("curl -sSLk -u alice:pw --max-time 2.5 --max-redirs 3 --http1.1 https://x.io");
        assert_eq!(r.auth.kind, AuthType::Basic);
        assert_eq!(r.auth.get("username"), "alice");
        assert_eq!(r.setting_bool("strictSSL"), Some(false));
        assert_eq!(r.setting_bool("followRedirects"), Some(true));
        assert_eq!(r.setting_u64("timeout"), Some(2500));
        assert_eq!(r.setting_u64("maxRedirects"), Some(3));
        assert_eq!(r.settings["protocolVersion"], "http1");
        assert_eq!(one("curl --digest -u a:b https://x.io").auth.kind, AuthType::Digest);
        assert_eq!(one("curl --oauth2-bearer tok https://x.io").auth.get("token"), "tok");
        assert_eq!(one("curl -XDELETE https://x.io/1").method, "DELETE");
        assert_eq!(one("curl -I https://x.io").method, "HEAD");
        assert_eq!(one("curl --request=PATCH --url=https://x.io").method, "PATCH");
    }

    #[test]
    fn multiple_commands_and_warnings() {
        let parsed = parse("curl https://a.io\ncurl https://b.io --foo-bar\n").unwrap();
        assert_eq!(parsed.requests.len(), 2);
        assert_eq!(parsed.requests[1].request.url, "https://b.io");
        assert!(parsed.warnings.iter().any(|w| w.contains("foo-bar")));
        let next = parse("curl https://a.io --next https://b.io").unwrap();
        assert_eq!(next.requests.len(), 2);
    }

    #[test]
    fn windows_continuations_and_prompt() {
        let r = one("$ curl ^\n  \"https://x.io/a\" ^\n  -H \"A: b\"");
        assert_eq!(r.url, "https://x.io/a");
        assert_eq!(r.header("a"), Some("b"));
    }

    #[test]
    fn errors() {
        assert!(parse("wget https://x").is_err());
        assert!(parse("curl -H 'A: b'").is_err());
    }
}
