//! Hawk HTTP authentication (header scheme).

use super::{AuthContext, AuthError};
use crate::http::PreparedRequest;
use crate::model::Auth;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};

fn mac(algorithm: &str, key: &[u8], data: &[u8]) -> String {
    if algorithm.eq_ignore_ascii_case("sha1") {
        let mut m = Hmac::<Sha1>::new_from_slice(key).expect("any key length");
        m.update(data);
        B64.encode(m.finalize().into_bytes())
    } else {
        let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
        m.update(data);
        B64.encode(m.finalize().into_bytes())
    }
}

/// `hash` attribute: digest of the content type and payload.
pub fn payload_hash(algorithm: &str, content_type: &str, payload: &[u8]) -> String {
    let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let mut data = format!("hawk.1.payload\n{ct}\n").into_bytes();
    data.extend_from_slice(payload);
    data.push(b'\n');
    if algorithm.eq_ignore_ascii_case("sha1") {
        B64.encode(Sha1::digest(&data))
    } else {
        B64.encode(Sha256::digest(&data))
    }
}

pub fn sign(auth: &Auth, req: &mut PreparedRequest, ctx: &AuthContext) -> Result<(), AuthError> {
    let url = url::Url::parse(&req.url).map_err(|e| AuthError::Invalid(e.to_string()))?;
    let algorithm = match auth.get("algorithm") {
        a if a.is_empty() => "sha256".to_string(),
        a => a,
    };
    let ts = match auth.get("timestamp") {
        t if t.is_empty() => ctx.now.timestamp().to_string(),
        t => t,
    };
    let nonce = match auth.get("nonce") {
        n if n.is_empty() => ctx.nonce(6),
        n => n,
    };
    let resource = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let host = url.host_str().unwrap_or_default().to_lowercase();
    let port = url.port_or_known_default().unwrap_or(80);
    let hash = if auth.get_bool("includePayloadHash") {
        payload_hash(&algorithm, req.header("content-type").unwrap_or(""), req.body.bytes().unwrap_or_default())
    } else {
        String::new()
    };
    let ext = auth.get("extraData");
    let app = auth.get("app");
    let dlg = auth.get("delegation");

    let mut normalized = format!(
        "hawk.1.header\n{ts}\n{nonce}\n{}\n{resource}\n{host}\n{port}\n{hash}\n{}\n",
        req.method.to_uppercase(),
        ext.replace('\\', "\\\\").replace('\n', "\\n")
    );
    if !app.is_empty() {
        normalized.push_str(&format!("{app}\n{dlg}\n"));
    }
    let signature = mac(&algorithm, auth.get("authKey").as_bytes(), normalized.as_bytes());

    let mut header = format!("Hawk id=\"{}\", ts=\"{ts}\", nonce=\"{nonce}\"", auth.get("authId"));
    if !hash.is_empty() {
        header.push_str(&format!(", hash=\"{hash}\""));
    }
    if !ext.is_empty() {
        header.push_str(&format!(", ext=\"{}\"", ext.replace('\\', "\\\\").replace('"', "\\\"")));
    }
    header.push_str(&format!(", mac=\"{signature}\""));
    if !app.is_empty() {
        header.push_str(&format!(", app=\"{app}\""));
        if !dlg.is_empty() {
            header.push_str(&format!(", dlg=\"{dlg}\""));
        }
    }
    req.set_header("Authorization", header);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::req;
    use crate::http::PreparedBody;
    use crate::model::AuthType;

    fn creds() -> Auth {
        Auth::new(AuthType::Hawk)
            .with("authId", "dh37fgj492je")
            .with("authKey", "werxhqb98rpaxn39848xrunpaw3489ruxnpa98w4rxn")
            .with("algorithm", "sha256")
            .with("timestamp", "1353832234")
            .with("nonce", "j4h3g2")
            .with("extraData", "some-app-ext-data")
    }

    /// Example from the Hawk protocol documentation.
    #[test]
    fn documented_get_example() {
        let mut r = req("GET", "http://example.com:8000/resource/1?b=1&a=2");
        sign(&creds(), &mut r, &AuthContext::default()).unwrap();
        assert_eq!(
            r.header("authorization").unwrap(),
            "Hawk id=\"dh37fgj492je\", ts=\"1353832234\", nonce=\"j4h3g2\", ext=\"some-app-ext-data\", mac=\"6R4rV5iE+NPoym+WwjeHzjAGXUtLNIxmo1vpMofpLAE=\""
        );
    }

    #[test]
    fn documented_payload_example() {
        let mut r = req("POST", "http://example.com:8000/resource/1?b=1&a=2");
        r.set_header("Content-Type", "text/plain");
        r.body = PreparedBody::Bytes(b"Thank you for flying Hawk".to_vec());
        sign(&creds().with("includePayloadHash", true), &mut r, &AuthContext::default()).unwrap();
        let h = r.header("authorization").unwrap();
        assert!(h.contains("hash=\"Yi9LfIIFRtBEPt74PVmbTF/xVAwPn7ub15ePICfgnuY=\""), "{h}");
        assert!(h.contains("mac=\"aSe1DERmZuRl3pI36/9BdZmnErTw3sNzOOAUlfeKjVw=\""), "{h}");
    }
}
