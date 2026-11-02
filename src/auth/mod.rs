//! Request signing for every supported authorization scheme.

pub mod aws;
pub mod digest;
pub mod hawk;
pub mod oauth1;
pub mod oauth2;

use crate::http::PreparedRequest;
use crate::model::{Auth, AuthType};
use crate::vars::VarContext;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Utc};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AuthError {
    #[error("{0} is not supported in this version of Courier")]
    Unsupported(String),
    #[error("{0}")]
    Invalid(String),
}

/// Inputs that tests need to control: the clock and the nonce source.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub now: DateTime<Utc>,
    pub nonce: Option<String>,
}

impl Default for AuthContext {
    fn default() -> Self {
        Self { now: Utc::now(), nonce: None }
    }
}

impl AuthContext {
    pub fn nonce(&self, len: usize) -> String {
        self.nonce.clone().unwrap_or_else(|| random_token(len))
    }
}

pub fn random_token(len: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Return a copy of `auth` with every string parameter variable-resolved.
pub fn resolve(auth: &Auth, vars: &VarContext) -> Auth {
    let mut out = auth.clone();
    for v in out.params.values_mut() {
        if let Value::String(s) = v {
            *s = vars.resolve(s);
        }
    }
    out
}

/// Sign `req` with an already-resolved auth. Digest without a known nonce
/// is a no-op here; the executor retries after the server's challenge.
pub fn apply(auth: &Auth, req: &mut PreparedRequest, ctx: &AuthContext) -> Result<(), AuthError> {
    match &auth.kind {
        AuthType::Inherit | AuthType::NoAuth => Ok(()),
        AuthType::ApiKey => {
            let key = auth.get("key");
            if key.is_empty() {
                return Ok(());
            }
            if auth.get("in") == "query" {
                req.add_query(&key, &auth.get("value"));
            } else {
                req.set_header(&key, auth.get("value"));
            }
            Ok(())
        }
        AuthType::Bearer => {
            let token = auth.get("token");
            if !token.is_empty() {
                req.set_header("Authorization", format!("Bearer {token}"));
            }
            Ok(())
        }
        AuthType::Basic => {
            let raw = format!("{}:{}", auth.get("username"), auth.get("password"));
            req.set_header("Authorization", format!("Basic {}", B64.encode(raw)));
            Ok(())
        }
        AuthType::Digest => {
            if !auth.get("nonce").is_empty() && !auth.get("realm").is_empty() {
                let header = digest::authorization(auth, &digest::Challenge::from_auth(auth), req, ctx)?;
                req.set_header("Authorization", header);
            }
            Ok(())
        }
        AuthType::OAuth1 => oauth1::sign(auth, req, ctx),
        AuthType::OAuth2 => {
            let token = auth.get("accessToken");
            if token.is_empty() {
                return Ok(());
            }
            let prefix = match auth.params.get("headerPrefix") {
                Some(Value::String(p)) => p.clone(),
                _ => "Bearer".into(),
            };
            if auth.get("addTokenTo") == "queryParams" {
                req.add_query("access_token", &token);
            } else {
                let value = if prefix.is_empty() { token } else { format!("{prefix} {token}") };
                req.set_header("Authorization", value);
            }
            Ok(())
        }
        AuthType::Jwt => jwt(auth, req, ctx),
        AuthType::Hawk => hawk::sign(auth, req, ctx),
        AuthType::AwsV4 => aws::sign(auth, req, ctx),
        other => Err(AuthError::Unsupported(other.label().to_string())),
    }
}

fn jwt_algorithm(name: &str) -> Result<jsonwebtoken::Algorithm, AuthError> {
    use jsonwebtoken::Algorithm::*;
    Ok(match name {
        "HS256" | "" => HS256,
        "HS384" => HS384,
        "HS512" => HS512,
        "RS256" => RS256,
        "RS384" => RS384,
        "RS512" => RS512,
        "PS256" => PS256,
        "PS384" => PS384,
        "PS512" => PS512,
        "ES256" => ES256,
        "ES384" => ES384,
        other => return Err(AuthError::Invalid(format!("unknown JWT algorithm {other}"))),
    })
}

/// Build a signed JWT from the auth parameters.
pub fn build_jwt(auth: &Auth, ctx: &AuthContext) -> Result<String, AuthError> {
    use jsonwebtoken::{Algorithm::*, EncodingKey, Header};
    let alg = jwt_algorithm(&auth.get("algorithm"))?;
    let mut header: Header = match auth.get("header").trim() {
        "" => Header::default(),
        text => serde_json::from_str(text).map_err(|e| AuthError::Invalid(format!("JWT header: {e}")))?,
    };
    header.alg = alg;
    let payload_text = auth.get("payload");
    let mut payload: Value = if payload_text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&payload_text).map_err(|e| AuthError::Invalid(format!("JWT payload: {e}")))?
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("iat").or_insert_with(|| ctx.now.timestamp().into());
    }
    let key = match alg {
        HS256 | HS384 | HS512 => {
            let secret = auth.get("secret");
            if auth.get_bool("isSecretBase64Encoded") {
                EncodingKey::from_base64_secret(&secret).map_err(|e| AuthError::Invalid(format!("JWT secret: {e}")))?
            } else {
                EncodingKey::from_secret(secret.as_bytes())
            }
        }
        RS256 | RS384 | RS512 | PS256 | PS384 | PS512 => {
            EncodingKey::from_rsa_pem(auth.get("privateKey").as_bytes())
                .map_err(|e| AuthError::Invalid(format!("JWT private key: {e}")))?
        }
        _ => EncodingKey::from_ec_pem(auth.get("privateKey").as_bytes())
            .map_err(|e| AuthError::Invalid(format!("JWT private key: {e}")))?,
    };
    jsonwebtoken::encode(&header, &payload, &key).map_err(|e| AuthError::Invalid(format!("JWT: {e}")))
}

fn jwt(auth: &Auth, req: &mut PreparedRequest, ctx: &AuthContext) -> Result<(), AuthError> {
    let token = build_jwt(auth, ctx)?;
    if auth.get("addTokenTo") == "queryParam" {
        let key = auth.get("queryParamKey");
        req.add_query(if key.is_empty() { "token" } else { &key }, &token);
    } else {
        let prefix = match auth.params.get("headerPrefix") {
            Some(Value::String(p)) => p.clone(),
            _ => "Bearer".into(),
        };
        req.set_header("Authorization", if prefix.is_empty() { token } else { format!("{prefix} {token}") });
    }
    Ok(())
}

/// Field definitions used by the UI to render each auth type's form.
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldKind {
    Text,
    Secret,
    Multiline,
    Bool,
    Choice(&'static [&'static str]),
}

const fn f(key: &'static str, label: &'static str, kind: FieldKind) -> Field {
    Field { key, label, kind }
}

use FieldKind::*;

pub fn fields(kind: &AuthType) -> Vec<Field> {
    match kind {
        AuthType::ApiKey => {
            vec![f("key", "Key", Text), f("value", "Value", Secret), f("in", "Add to", Choice(&["header", "query"]))]
        }
        AuthType::Bearer => vec![f("token", "Token", Secret)],
        AuthType::Basic => vec![f("username", "Username", Text), f("password", "Password", Secret)],
        AuthType::Digest => vec![
            f("username", "Username", Text),
            f("password", "Password", Secret),
            f("realm", "Realm", Text),
            f("nonce", "Nonce", Text),
            f(
                "algorithm",
                "Algorithm",
                Choice(&["MD5", "MD5-sess", "SHA-256", "SHA-256-sess", "SHA-512-256", "SHA-512-256-sess"]),
            ),
            f("qop", "qop", Choice(&["", "auth", "auth-int"])),
            f("nonceCount", "Nonce count", Text),
            f("clientNonce", "Client nonce", Text),
            f("opaque", "Opaque", Text),
            f("disableRetryRequest", "Disable retrying the request", Bool),
        ],
        AuthType::OAuth1 => vec![
            f(
                "signatureMethod",
                "Signature method",
                Choice(&["HMAC-SHA1", "HMAC-SHA256", "HMAC-SHA512", "PLAINTEXT", "RSA-SHA256", "RSA-SHA512"]),
            ),
            f("consumerKey", "Consumer key", Text),
            f("consumerSecret", "Consumer secret", Secret),
            f("token", "Access token", Text),
            f("tokenSecret", "Token secret", Secret),
            f("privateKey", "Private key (RSA)", Multiline),
            f("addParamsToHeader", "Add auth data to header", Bool),
            f("callback", "Callback URL", Text),
            f("verifier", "Verifier", Text),
            f("timestamp", "Timestamp", Text),
            f("nonce", "Nonce", Text),
            f("version", "Version", Text),
            f("realm", "Realm", Text),
            f("includeBodyHash", "Include body hash", Bool),
            f("addEmptyParamsToSign", "Add empty parameters to signature", Bool),
        ],
        AuthType::OAuth2 => vec![
            f("accessToken", "Access token", Secret),
            f("headerPrefix", "Header prefix", Text),
            f("addTokenTo", "Add token to", Choice(&["header", "queryParams"])),
            f("tokenName", "Token name", Text),
            f(
                "grant_type",
                "Grant type",
                Choice(&[
                    "authorization_code",
                    "authorization_code_with_pkce",
                    "implicit",
                    "password_credentials",
                    "client_credentials",
                ]),
            ),
            f("authUrl", "Auth URL", Text),
            f("accessTokenUrl", "Access token URL", Text),
            f("redirect_uri", "Callback URL", Text),
            f("clientId", "Client ID", Text),
            f("clientSecret", "Client secret", Secret),
            f("scope", "Scope", Text),
            f("state", "State", Text),
            f("username", "Username", Text),
            f("password", "Password", Secret),
            f("challengeAlgorithm", "Code challenge method", Choice(&["S256", "plain"])),
            f("code_verifier", "Code verifier", Text),
            f("client_authentication", "Client authentication", Choice(&["header", "body"])),
            f("audience", "Audience", Text),
            f("resource", "Resource", Text),
            f("refreshToken", "Refresh token", Secret),
            f("autoRefreshToken", "Auto-refresh token", Bool),
        ],
        AuthType::Jwt => vec![
            f(
                "algorithm",
                "Algorithm",
                Choice(&[
                    "HS256", "HS384", "HS512", "RS256", "RS384", "RS512", "PS256", "PS384", "PS512", "ES256", "ES384",
                ]),
            ),
            f("secret", "Secret", Secret),
            f("isSecretBase64Encoded", "Secret is Base64 encoded", Bool),
            f("privateKey", "Private key (PEM)", Multiline),
            f("payload", "Payload (JSON)", Multiline),
            f("header", "Extra headers (JSON)", Multiline),
            f("addTokenTo", "Add token to", Choice(&["header", "queryParam"])),
            f("headerPrefix", "Header prefix", Text),
            f("queryParamKey", "Query param name", Text),
        ],
        AuthType::Hawk => vec![
            f("authId", "Hawk Auth ID", Text),
            f("authKey", "Hawk Auth Key", Secret),
            f("algorithm", "Algorithm", Choice(&["sha256", "sha1"])),
            f("user", "User", Text),
            f("nonce", "Nonce", Text),
            f("extraData", "ext", Text),
            f("app", "app", Text),
            f("delegation", "dlg", Text),
            f("timestamp", "Timestamp", Text),
            f("includePayloadHash", "Include payload hash", Bool),
        ],
        AuthType::AwsV4 => vec![
            f("accessKey", "Access key", Text),
            f("secretKey", "Secret key", Secret),
            f("sessionToken", "Session token", Secret),
            f("region", "AWS region", Text),
            f("service", "Service name", Text),
            f("addAuthDataToQuery", "Add auth data to query", Bool),
        ],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{PreparedBody, SendOptions};

    pub(crate) fn req(method: &str, url: &str) -> PreparedRequest {
        PreparedRequest {
            method: method.into(),
            url: url.into(),
            headers: vec![],
            body: PreparedBody::None,
            options: SendOptions::default(),
        }
    }

    #[test]
    fn basic_bearer_apikey() {
        let mut r = req("GET", "http://x/");
        apply(
            &Auth::new(AuthType::Basic).with("username", "Aladdin").with("password", "open sesame"),
            &mut r,
            &AuthContext::default(),
        )
        .unwrap();
        assert_eq!(r.header("authorization"), Some("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="));

        apply(&Auth::new(AuthType::Bearer).with("token", "t0k"), &mut r, &AuthContext::default()).unwrap();
        assert_eq!(r.header("authorization"), Some("Bearer t0k"));

        apply(&Auth::new(AuthType::ApiKey).with("key", "X-Key").with("value", "v"), &mut r, &AuthContext::default())
            .unwrap();
        assert_eq!(r.header("x-key"), Some("v"));
        apply(
            &Auth::new(AuthType::ApiKey).with("key", "k").with("value", "a b").with("in", "query"),
            &mut r,
            &AuthContext::default(),
        )
        .unwrap();
        assert_eq!(r.url, "http://x/?k=a%20b");
    }

    #[test]
    fn oauth2_token_placement() {
        let mut r = req("GET", "http://x/");
        let auth = Auth::new(AuthType::OAuth2).with("accessToken", "abc").with("headerPrefix", "Token");
        apply(&auth, &mut r, &AuthContext::default()).unwrap();
        assert_eq!(r.header("authorization"), Some("Token abc"));
        let auth = auth.with("addTokenTo", "queryParams");
        let mut r = req("GET", "http://x/");
        apply(&auth, &mut r, &AuthContext::default()).unwrap();
        assert_eq!(r.url, "http://x/?access_token=abc");
    }

    #[test]
    fn jwt_hs256_verifies() {
        let auth = Auth::new(AuthType::Jwt)
            .with("algorithm", "HS256")
            .with("secret", "s3cret")
            .with("payload", r#"{"sub":"42","exp":4102444800}"#);
        let mut r = req("GET", "http://x/");
        apply(&auth, &mut r, &AuthContext::default()).unwrap();
        let token = r.header("authorization").unwrap().strip_prefix("Bearer ").unwrap().to_string();
        let data = jsonwebtoken::decode::<Value>(
            &token,
            &jsonwebtoken::DecodingKey::from_secret(b"s3cret"),
            &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256),
        )
        .unwrap();
        assert_eq!(data.claims["sub"], "42");
        assert!(data.claims["iat"].is_number());
    }

    #[test]
    fn variables_are_resolved_in_params() {
        let mut vars = VarContext::default();
        vars.local.insert("tok".into(), "zzz".into());
        let a = resolve(&Auth::new(AuthType::Bearer).with("token", "{{tok}}").with("flag", true), &vars);
        assert_eq!(a.get("token"), "zzz");
        assert!(a.get_bool("flag"));
    }

    #[test]
    fn unsupported_types_error() {
        let mut r = req("GET", "http://x/");
        assert!(matches!(
            apply(&Auth::new(AuthType::Ntlm), &mut r, &AuthContext::default()),
            Err(AuthError::Unsupported(_))
        ));
    }
}
