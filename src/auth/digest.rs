//! HTTP Digest authentication (RFC 7616).

use super::{AuthContext, AuthError, hex};
use crate::http::PreparedRequest;
use crate::model::Auth;
use md5::Md5;
use sha2::{Digest as _, Sha256, Sha512_256};

/// Server challenge values (from `WWW-Authenticate` or the auth params).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub qop: String,
    pub algorithm: String,
    pub opaque: String,
}

impl Challenge {
    pub fn from_auth(auth: &Auth) -> Self {
        Self {
            realm: auth.get("realm"),
            nonce: auth.get("nonce"),
            qop: auth.get("qop"),
            algorithm: auth.get("algorithm"),
            opaque: auth.get("opaque"),
        }
    }

    /// Parse a `WWW-Authenticate: Digest ...` header value.
    pub fn parse(header: &str) -> Option<Self> {
        let rest = header.trim().strip_prefix("Digest").or_else(|| header.trim().strip_prefix("digest"))?;
        let mut c = Challenge::default();
        for (k, v) in parse_params(rest) {
            match k.to_ascii_lowercase().as_str() {
                "realm" => c.realm = v,
                "nonce" => c.nonce = v,
                "qop" => c.qop = v,
                "algorithm" => c.algorithm = v,
                "opaque" => c.opaque = v,
                _ => {}
            }
        }
        Some(c)
    }
}

/// Parse `a="x", b=y` style parameter lists (quoted strings may hold commas).
pub fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let key: String = chars.by_ref().take_while(|c| *c != '=').collect::<String>().trim().to_string();
        if key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    _ => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.peek() {
                if *c == ',' {
                    break;
                }
                value.push(*c);
                chars.next();
            }
            value = value.trim().to_string();
        }
        out.push((key, value));
    }
    out
}

fn h(algorithm: &str, data: &str) -> String {
    let alg = algorithm.trim_end_matches("-sess").trim_end_matches("-SESS").to_ascii_uppercase();
    match alg.as_str() {
        "SHA-256" => hex(&Sha256::digest(data.as_bytes())),
        "SHA-512-256" => hex(&Sha512_256::digest(data.as_bytes())),
        _ => hex(&Md5::digest(data.as_bytes())),
    }
}

/// Compute the `Authorization: Digest ...` header for a request.
pub fn authorization(
    auth: &Auth,
    ch: &Challenge,
    req: &PreparedRequest,
    ctx: &AuthContext,
) -> Result<String, AuthError> {
    let url = url::Url::parse(&req.url).map_err(|e| AuthError::Invalid(e.to_string()))?;
    let uri = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let algorithm = if ch.algorithm.is_empty() { "MD5".to_string() } else { ch.algorithm.clone() };
    let username = auth.get("username");
    let password = auth.get("password");
    // Prefer "auth" when the server offers several qop values.
    let qop = ch
        .qop
        .split(',')
        .map(str::trim)
        .find(|q| *q == "auth")
        .or_else(|| ch.qop.split(',').map(str::trim).find(|q| !q.is_empty()))
        .unwrap_or("")
        .to_string();
    let nc = match auth.get("nonceCount") {
        n if n.is_empty() => "00000001".to_string(),
        n => n,
    };
    let cnonce = match auth.get("clientNonce") {
        c if c.is_empty() => ctx.nonce(16),
        c => c,
    };

    let mut ha1 = h(&algorithm, &format!("{username}:{}:{password}", ch.realm));
    if algorithm.to_ascii_lowercase().ends_with("-sess") {
        ha1 = h(&algorithm, &format!("{ha1}:{}:{cnonce}", ch.nonce));
    }
    let ha2 = if qop == "auth-int" {
        let body_hash = h(&algorithm, &String::from_utf8_lossy(req.body.bytes().unwrap_or_default()));
        h(&algorithm, &format!("{}:{uri}:{body_hash}", req.method))
    } else {
        h(&algorithm, &format!("{}:{uri}", req.method))
    };
    let response = if qop.is_empty() {
        h(&algorithm, &format!("{ha1}:{}:{ha2}", ch.nonce))
    } else {
        h(&algorithm, &format!("{ha1}:{}:{nc}:{cnonce}:{qop}:{ha2}", ch.nonce))
    };

    let mut header = format!(
        "Digest username=\"{username}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", algorithm={algorithm}",
        ch.realm, ch.nonce
    );
    if !qop.is_empty() {
        header.push_str(&format!(", qop={qop}, nc={nc}, cnonce=\"{cnonce}\""));
    }
    header.push_str(&format!(", response=\"{response}\""));
    if !ch.opaque.is_empty() {
        header.push_str(&format!(", opaque=\"{}\"", ch.opaque));
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::req;
    use crate::model::AuthType;

    /// RFC 7616 §3.9.1 example.
    fn rfc_auth(alg: &str) -> (Auth, Challenge) {
        let auth = Auth::new(AuthType::Digest)
            .with("username", "Mufasa")
            .with("password", "Circle of Life")
            .with("clientNonce", "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ");
        let ch = Challenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
            qop: "auth, auth-int".into(),
            algorithm: alg.into(),
            opaque: "FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into(),
        };
        (auth, ch)
    }

    #[test]
    fn rfc7616_md5() {
        let (auth, ch) = rfc_auth("MD5");
        let h =
            authorization(&auth, &ch, &req("GET", "http://www.example.org/dir/index.html"), &AuthContext::default())
                .unwrap();
        assert!(h.contains("response=\"8ca523f5e9506fed4657c9700eebdbec\""), "{h}");
        assert!(h.contains("qop=auth, nc=00000001"));
        assert!(h.contains("opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\""));
    }

    #[test]
    fn rfc7616_sha256() {
        let (auth, ch) = rfc_auth("SHA-256");
        let h =
            authorization(&auth, &ch, &req("GET", "http://www.example.org/dir/index.html"), &AuthContext::default())
                .unwrap();
        assert!(h.contains("response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\""), "{h}");
    }

    #[test]
    fn parses_challenge() {
        let c =
            Challenge::parse(r#"Digest realm="a, b", qop="auth,auth-int", algorithm=SHA-256, nonce="n1", opaque="o""#)
                .unwrap();
        assert_eq!(c.realm, "a, b");
        assert_eq!(c.qop, "auth,auth-int");
        assert_eq!(c.algorithm, "SHA-256");
        assert_eq!(c.nonce, "n1");
        assert!(Challenge::parse("Basic realm=x").is_none());
    }
}
