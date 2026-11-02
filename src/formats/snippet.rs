//! Code snippets for a request: cURL and raw HTTP.

use crate::auth::{self, AuthContext};
use crate::http::{PreparedBody, PreparedPart, PreparedRequest, auto_headers, prepare};
use crate::model::{Auth, Request};
use crate::settings::Settings;
use crate::vars::VarContext;

/// Build the request as it would be sent, minus Courier's automatic headers.
/// With `resolve == false`, `{{variables}}` are kept as placeholders.
pub fn snippet_request(
    req: &Request,
    auth: &Auth,
    vars: &VarContext,
    settings: &Settings,
    resolve: bool,
) -> PreparedRequest {
    let empty = VarContext::default();
    let vars = if resolve { vars } else { &empty };
    let mut p = prepare(req, vars, settings);
    let autos = auto_headers(req, settings);
    p.headers.retain(|(n, _)| {
        req.header(n).is_some()
            || n.eq_ignore_ascii_case("content-type")
            || !autos.iter().any(|(a, _)| a.eq_ignore_ascii_case(n))
    });
    let auth = if resolve { auth::resolve(auth, vars) } else { auth.clone() };
    let _ = auth::apply(&auth, &mut p, &AuthContext::default());
    p
}

/// Quote a word for POSIX shells.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn to_curl(p: &PreparedRequest) -> String {
    let has_body = !matches!(p.body, PreparedBody::None);
    let mut lines =
        vec![format!("curl{} {}", if p.options.follow_redirects { " --location" } else { "" }, shell_quote(&p.url))];
    let implied = (p.method == "GET" && !has_body) || (p.method == "POST" && has_body);
    if !implied {
        lines.push(format!("--request {}", p.method));
    }
    for (n, v) in &p.headers {
        lines.push(format!("--header {}", shell_quote(&format!("{n}: {v}"))));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Bytes(b) => lines.push(format!("--data-raw {}", shell_quote(&String::from_utf8_lossy(b)))),
        PreparedBody::File(path) => {
            lines.push(format!("--data-binary {}", shell_quote(&format!("@{}", path.display()))))
        }
        PreparedBody::Multipart(parts) => {
            for part in parts {
                match part {
                    PreparedPart::Text { name, value, .. } => {
                        lines.push(format!("--form {}", shell_quote(&format!("{name}={value}"))))
                    }
                    PreparedPart::Files { name, paths, content_type } => {
                        for path in paths {
                            let ct = content_type.as_ref().map(|c| format!(";type={c}")).unwrap_or_default();
                            lines.push(format!(
                                "--form {}",
                                shell_quote(&format!("{name}=@\"{}\"{ct}", path.display()))
                            ));
                        }
                    }
                }
            }
        }
    }
    if !p.options.strict_ssl {
        lines.push("--insecure".into());
    }
    lines.join(" \\\n")
}

pub fn to_raw_http(p: &PreparedRequest) -> String {
    let (target, host) = match url::Url::parse(&p.url) {
        Ok(u) => {
            let mut t = u.path().to_string();
            if let Some(q) = u.query() {
                t.push('?');
                t.push_str(q);
            }
            let mut h = u.host_str().unwrap_or_default().to_string();
            if let Some(port) = u.port() {
                h.push_str(&format!(":{port}"));
            }
            (t, h)
        }
        Err(_) => (p.url.clone(), String::new()),
    };
    let mut out = format!("{} {target} HTTP/1.1\nHost: {host}\n", p.method);
    for (n, v) in &p.headers {
        out.push_str(&format!("{n}: {v}\n"));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Bytes(b) => {
            out.push('\n');
            out.push_str(&String::from_utf8_lossy(b));
        }
        PreparedBody::File(path) => out.push_str(&format!("\n<file contents: {}>", path.display())),
        PreparedBody::Multipart(parts) => {
            out.push_str("Content-Type: multipart/form-data; boundary=----CourierBoundary\n");
            for part in parts {
                out.push_str("\n------CourierBoundary\n");
                match part {
                    PreparedPart::Text { name, value, .. } => {
                        out.push_str(&format!("Content-Disposition: form-data; name=\"{name}\"\n\n{value}"))
                    }
                    PreparedPart::Files { name, paths, .. } => {
                        for path in paths {
                            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                            out.push_str(&format!(
                                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\n\n<{}>",
                                path.display()
                            ));
                        }
                    }
                }
            }
            out.push_str("\n------CourierBoundary--");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::curl;
    use crate::model::*;

    fn vars() -> VarContext {
        let mut v = VarContext::default();
        v.local.insert("host".into(), "api.x.io".into());
        v
    }

    #[test]
    fn curl_export_round_trips_through_import() {
        let mut req = Request::new("PUT", "https://{{host}}/items/1?x=1");
        req.headers.push(KeyValue::new("X-It's", "quote'd"));
        req.body = Body::raw("{\"a\": \"b c\"}", RawLanguage::Json);
        let auth = Auth::new(AuthType::Basic).with("username", "u").with("password", "p");
        let p = snippet_request(&req, &auth, &vars(), &Settings::default(), true);
        let cmd = to_curl(&p);
        assert!(cmd.starts_with("curl --location 'https://api.x.io/items/1?x=1'"), "{cmd}");
        assert!(!cmd.contains("User-Agent"), "auto headers are not exported");
        let back = curl::parse(&cmd).unwrap().requests.remove(0).request;
        assert_eq!(back.method, "PUT");
        assert_eq!(back.url, "https://api.x.io/items/1?x=1");
        assert_eq!(back.header("x-it's"), Some("quote'd"));
        assert_eq!(back.header("authorization"), Some("Basic dTpw"));
        assert_eq!(back.body.raw, "{\"a\": \"b c\"}");
        assert_eq!(back.body.raw_language, RawLanguage::Json);
    }

    #[test]
    fn placeholders_can_be_kept() {
        let req = Request::new("GET", "https://{{host}}/a");
        let p = snippet_request(&req, &Auth::none(), &vars(), &Settings::default(), false);
        assert!(to_curl(&p).contains("https://{{host}}/a"));
    }

    #[test]
    fn form_and_urlencoded_round_trip() {
        let mut req = Request::new("POST", "http://x.io/f");
        req.body.mode = BodyMode::Formdata;
        req.body.formdata = vec![
            FormField { key: "a".into(), value: "1".into(), enabled: true, ..Default::default() },
            FormField {
                key: "f".into(),
                kind: FormFieldKind::File,
                files: vec!["/tmp/x y.png".into()],
                enabled: true,
                ..Default::default()
            },
        ];
        let cmd = to_curl(&snippet_request(&req, &Auth::none(), &vars(), &Settings::default(), true));
        let back = curl::parse(&cmd).unwrap().requests.remove(0).request;
        assert_eq!(back.body.formdata[1].files, ["/tmp/x y.png"]);

        req.body.mode = BodyMode::Urlencoded;
        req.body.urlencoded = vec![KeyValue::new("q", "a&b c")];
        let cmd = to_curl(&snippet_request(&req, &Auth::none(), &vars(), &Settings::default(), true));
        let back = curl::parse(&cmd).unwrap().requests.remove(0).request;
        assert_eq!(back.body.urlencoded, vec![KeyValue::new("q", "a&b c")]);
    }

    #[test]
    fn raw_http_snippet() {
        let mut req = Request::new("POST", "http://x.io:8080/a?b=1");
        req.body = Body::raw("hi", RawLanguage::Text);
        let text = to_raw_http(&snippet_request(&req, &Auth::none(), &vars(), &Settings::default(), true));
        assert!(text.starts_with("POST /a?b=1 HTTP/1.1\nHost: x.io:8080\n"));
        assert!(text.ends_with("\n\nhi"));
    }
}
