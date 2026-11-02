//! AWS Signature Version 4.

use super::{AuthContext, AuthError, hex};
use crate::http::PreparedRequest;
use crate::model::Auth;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::{Digest, Sha256};

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
const PATH: &AsciiSet = &UNRESERVED.remove(b'/');

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

fn canonical_query(pairs: &[(String, String)]) -> String {
    let mut enc_pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    enc_pairs.sort();
    enc_pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

pub fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k = hmac(&k, region);
    let k = hmac(&k, service);
    hmac(&k, "aws4_request")
}

pub fn sign(auth: &Auth, req: &mut PreparedRequest, ctx: &AuthContext) -> Result<(), AuthError> {
    let url = url::Url::parse(&req.url).map_err(|e| AuthError::Invalid(e.to_string()))?;
    let region = match auth.get("region") {
        r if r.is_empty() => "us-east-1".to_string(),
        r => r,
    };
    let service = auth.get("service");
    if service.is_empty() {
        return Err(AuthError::Invalid("AWS Signature needs a service name".into()));
    }
    let access = auth.get("accessKey");
    let secret = auth.get("secretKey");
    let token = auth.get("sessionToken");
    let amz_date = ctx.now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = ctx.now.format("%Y%m%d").to_string();
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let payload_hash = match req.body.bytes() {
        Some(b) => hex(&Sha256::digest(b)),
        None => "UNSIGNED-PAYLOAD".to_string(),
    };
    let mut host = url.host_str().unwrap_or_default().to_lowercase();
    if let Some(p) = url.port() {
        host.push_str(&format!(":{p}"));
    }
    let path = match url.path() {
        "" => "/".to_string(),
        p => utf8_percent_encode(&percent_encoding::percent_decode_str(p).decode_utf8_lossy(), PATH).to_string(),
    };
    let mut query: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let to_query = auth.get_bool("addAuthDataToQuery");

    // Headers that take part in the signature.
    let mut signed: Vec<(String, String)> = vec![("host".into(), host)];
    if !to_query {
        signed.push(("x-amz-date".into(), amz_date.clone()));
        if !token.is_empty() {
            signed.push(("x-amz-security-token".into(), token.clone()));
        }
        if service == "s3" {
            signed.push(("x-amz-content-sha256".into(), payload_hash.clone()));
        }
        if let Some(ct) = req.header("content-type") {
            signed.push(("content-type".into(), ct.trim().to_string()));
        }
    }
    signed.sort();
    let signed_names = signed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(";");
    if to_query {
        query.push(("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()));
        query.push(("X-Amz-Credential".into(), format!("{access}/{scope}")));
        query.push(("X-Amz-Date".into(), amz_date.clone()));
        query.push(("X-Amz-SignedHeaders".into(), signed_names.clone()));
        if !token.is_empty() {
            query.push(("X-Amz-Security-Token".into(), token.clone()));
        }
    }
    let canonical_headers: String = signed.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let canonical = format!(
        "{}\n{path}\n{}\n{canonical_headers}\n{signed_names}\n{}",
        req.method.to_uppercase(),
        canonical_query(&query),
        if to_query { "UNSIGNED-PAYLOAD" } else { &payload_hash }
    );
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&Sha256::digest(canonical.as_bytes())));
    let signature = hex(&hmac(&signing_key(&secret, &date, &region, &service), &string_to_sign));

    if to_query {
        query.push(("X-Amz-Signature".into(), signature));
        let mut u = url.clone();
        u.set_query(Some(&canonical_query(&query)));
        req.url = u.to_string();
    } else {
        req.set_header("X-Amz-Date", amz_date);
        if !token.is_empty() {
            req.set_header("X-Amz-Security-Token", token);
        }
        if service == "s3" {
            req.set_header("X-Amz-Content-Sha256", payload_hash);
        }
        req.set_header(
            "Authorization",
            format!(
                "AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed_names}, Signature={signature}"
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::req;
    use crate::model::AuthType;
    use chrono::TimeZone;

    fn ctx() -> AuthContext {
        AuthContext { now: chrono::Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap(), nonce: None }
    }

    fn creds() -> Auth {
        Auth::new(AuthType::AwsV4)
            .with("accessKey", "AKIDEXAMPLE")
            .with("secretKey", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")
            .with("region", "us-east-1")
            .with("service", "iam")
    }

    /// Example from the AWS Signature Version 4 documentation.
    #[test]
    fn documented_iam_example() {
        let mut r = req("GET", "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08");
        r.set_header("Content-Type", "application/x-www-form-urlencoded; charset=utf-8");
        sign(&creds(), &mut r, &ctx()).unwrap();
        assert_eq!(
            r.header("authorization").unwrap(),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
        assert_eq!(r.header("x-amz-date"), Some("20150830T123600Z"));
    }

    #[test]
    fn signing_key_matches_docs() {
        let k = signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20150830", "us-east-1", "iam");
        assert_eq!(hex(&k), "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9");
    }

    #[test]
    fn query_mode_adds_signature_params() {
        let mut r = req("GET", "https://example.amazonaws.com/path?x=1");
        sign(&creds().with("addAuthDataToQuery", true).with("sessionToken", "tok"), &mut r, &ctx()).unwrap();
        assert!(r.header("authorization").is_none());
        assert!(r.url.contains("X-Amz-Signature="));
        assert!(r.url.contains("X-Amz-Security-Token=tok"));
    }
}
