//! OAuth 1.0a request signing (RFC 5849).

use super::{AuthContext, AuthError};
use crate::http::{PreparedBody, PreparedRequest};
use crate::model::Auth;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

/// RFC 3986 unreserved characters are left alone; everything else encoded.
const RFC3986: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

pub fn encode(s: &str) -> String {
    utf8_percent_encode(s, RFC3986).to_string()
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned()
}

fn hmac_b64<M: Mac + hmac::digest::KeyInit>(key: &[u8], data: &[u8]) -> String {
    let mut mac = <M as hmac::digest::KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    B64.encode(mac.finalize().into_bytes())
}

/// Compute the signature for `base` with the given method.
fn signature(method: &str, base: &str, key: &str, private_key: &str) -> Result<String, AuthError> {
    Ok(match method {
        "HMAC-SHA1" | "" => hmac_b64::<Hmac<Sha1>>(key.as_bytes(), base.as_bytes()),
        "HMAC-SHA256" => hmac_b64::<Hmac<Sha256>>(key.as_bytes(), base.as_bytes()),
        "HMAC-SHA512" => hmac_b64::<Hmac<Sha512>>(key.as_bytes(), base.as_bytes()),
        "PLAINTEXT" => key.to_string(),
        "RSA-SHA256" | "RSA-SHA512" => {
            let alg =
                if method == "RSA-SHA256" { jsonwebtoken::Algorithm::RS256 } else { jsonwebtoken::Algorithm::RS512 };
            let k = jsonwebtoken::EncodingKey::from_rsa_pem(private_key.as_bytes())
                .map_err(|e| AuthError::Invalid(format!("RSA private key: {e}")))?;
            let sig =
                jsonwebtoken::crypto::sign(base.as_bytes(), &k, alg).map_err(|e| AuthError::Invalid(e.to_string()))?;
            let raw = URL_SAFE_NO_PAD.decode(sig).map_err(|e| AuthError::Invalid(e.to_string()))?;
            B64.encode(raw)
        }
        other => return Err(AuthError::Unsupported(format!("OAuth 1.0 signature method {other}"))),
    })
}

/// Build the signature base string from the request and oauth parameters.
pub fn base_string(method: &str, url: &url::Url, params: &[(String, String)]) -> String {
    let mut base_url = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default().to_lowercase());
    if let Some(port) = url.port() {
        base_url.push_str(&format!(":{port}"));
    }
    base_url.push_str(url.path());
    let mut enc: Vec<(String, String)> = params.iter().map(|(k, v)| (encode(k), encode(v))).collect();
    enc.sort();
    let normalized = enc.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    format!("{}&{}&{}", method.to_uppercase(), encode(&base_url), encode(&normalized))
}

pub fn sign(auth: &Auth, req: &mut PreparedRequest, ctx: &AuthContext) -> Result<(), AuthError> {
    let url = url::Url::parse(&req.url).map_err(|e| AuthError::Invalid(e.to_string()))?;
    let method_name = auth.get("signatureMethod");
    let method_name = if method_name.is_empty() { "HMAC-SHA1".to_string() } else { method_name };
    let timestamp = match auth.get("timestamp") {
        t if t.is_empty() => ctx.now.timestamp().to_string(),
        t => t,
    };
    let nonce = match auth.get("nonce") {
        n if n.is_empty() => ctx.nonce(11),
        n => n,
    };
    let version = match auth.get("version") {
        v if v.is_empty() => "1.0".to_string(),
        v => v,
    };
    let include_empty = auth.get_bool("addEmptyParamsToSign");

    let mut oauth: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), auth.get("consumerKey")),
        ("oauth_signature_method".into(), method_name.clone()),
        ("oauth_timestamp".into(), timestamp),
        ("oauth_nonce".into(), nonce),
        ("oauth_version".into(), version),
    ];
    for (key, param) in [("oauth_token", "token"), ("oauth_callback", "callback"), ("oauth_verifier", "verifier")] {
        let v = auth.get(param);
        if !v.is_empty() || include_empty {
            oauth.push((key.into(), v));
        }
    }

    let is_form = req
        .header("content-type")
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/x-www-form-urlencoded"));
    if auth.get_bool("includeBodyHash")
        && !is_form
        && let Some(b) = req.body.bytes()
    {
        let digest = if method_name.contains("256") {
            B64.encode(Sha256::digest(b))
        } else if method_name.contains("512") {
            B64.encode(Sha512::digest(b))
        } else {
            B64.encode(Sha1::digest(b))
        };
        oauth.push(("oauth_body_hash".into(), digest));
    }

    let mut all = oauth.clone();
    all.extend(url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())));
    if is_form && let Some(body) = req.body.bytes() {
        let text = String::from_utf8_lossy(body);
        for pair in text.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            all.push((decode(k), decode(v)));
        }
    }
    if !include_empty {
        all.retain(|(k, v)| !v.is_empty() || !k.starts_with("oauth_"));
    }

    let base = base_string(&req.method, &url, &all);
    let key = format!("{}&{}", encode(&auth.get("consumerSecret")), encode(&auth.get("tokenSecret")));
    let sig = signature(&method_name, &base, &key, &auth.get("privateKey"))?;
    oauth.push(("oauth_signature".into(), sig));

    let add_to_header =
        auth.params.get("addParamsToHeader").map(|_| auth.get_bool("addParamsToHeader")).unwrap_or(true);
    if add_to_header {
        let mut parts = Vec::new();
        let realm = auth.get("realm");
        if !realm.is_empty() {
            parts.push(format!("realm=\"{}\"", encode(&realm)));
        }
        parts.extend(oauth.iter().map(|(k, v)| format!("{}=\"{}\"", encode(k), encode(v))));
        req.set_header("Authorization", format!("OAuth {}", parts.join(", ")));
    } else if is_form {
        if let PreparedBody::Bytes(b) = &mut req.body {
            for (k, v) in &oauth {
                if !b.is_empty() {
                    b.push(b'&');
                }
                b.extend_from_slice(format!("{}={}", encode(k), encode(v)).as_bytes());
            }
        }
    } else {
        for (k, v) in &oauth {
            req.add_query(k, v);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::req;
    use crate::model::AuthType;

    /// The widely published "Creating a signature" example.
    #[test]
    fn known_hmac_sha1_signature() {
        let auth = Auth::new(AuthType::OAuth1)
            .with("consumerKey", "xvz1evFS4wEEPTGEFPHBog")
            .with("consumerSecret", "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw")
            .with("token", "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb")
            .with("tokenSecret", "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE")
            .with("nonce", "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg")
            .with("timestamp", "1318622958");
        let mut r = req("POST", "https://api.twitter.com/1.1/statuses/update.json?include_entities=true");
        r.set_header("Content-Type", "application/x-www-form-urlencoded");
        r.body = PreparedBody::Bytes(
            b"status=Hello%20Ladies%20%2B%20Gentlemen%2C%20a%20signed%20OAuth%20request%21".to_vec(),
        );
        sign(&auth, &mut r, &AuthContext::default()).unwrap();
        let h = r.header("authorization").unwrap();
        assert!(h.starts_with("OAuth "));
        assert!(h.contains("oauth_signature=\"hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D\""), "{h}");
    }

    #[test]
    fn plaintext_and_query_placement() {
        let auth = Auth::new(AuthType::OAuth1)
            .with("signatureMethod", "PLAINTEXT")
            .with("consumerKey", "ck")
            .with("consumerSecret", "cs&x")
            .with("tokenSecret", "ts")
            .with("addParamsToHeader", false);
        let mut r = req("GET", "http://x/a");
        sign(&auth, &mut r, &AuthContext::default()).unwrap();
        assert!(r.header("authorization").is_none());
        assert!(r.url.contains("oauth_signature=cs%2526x%26ts"), "{}", r.url);
        assert!(r.url.contains("oauth_consumer_key=ck"));
    }

    #[test]
    fn encoding_is_rfc3986() {
        assert_eq!(encode("a b+c~*"), "a%20b%2Bc~%2A");
    }
}
