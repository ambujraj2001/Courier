//! OAuth 2.0 token acquisition: client credentials, password, authorization
//! code (with optional PKCE), implicit and refresh. Browser-based flows use
//! the system browser and a loopback redirect listener; no cloud callback.

use super::{AuthError, random_token};
use crate::model::Auth;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A token obtained from an authorization server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2Token {
    pub id: String,
    pub name: String,
    pub access_token: String,
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    pub obtained_at: DateTime<Utc>,
    /// The full token response, for display.
    #[serde(default)]
    pub raw: Value,
}

impl OAuth2Token {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
    }
}

/// Saved tokens (`oauth_tokens.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenStore {
    pub tokens: Vec<OAuth2Token>,
}

impl TokenStore {
    pub fn add(&mut self, token: OAuth2Token) {
        self.tokens.retain(|t| t.id != token.id);
        self.tokens.push(token);
    }

    pub fn remove(&mut self, id: &str) {
        self.tokens.retain(|t| t.id != id);
    }

    pub fn find_by_access_token(&self, access: &str) -> Option<&OAuth2Token> {
        self.tokens.iter().find(|t| t.access_token == access)
    }
}

/// Write a token into the auth params so subsequent requests use it.
pub fn use_token(auth: &mut Auth, token: &OAuth2Token) {
    auth.set("accessToken", token.access_token.clone());
    if let Some(r) = &token.refresh_token {
        auth.set("refreshToken", r.clone());
    }
    match token.expires_at {
        Some(e) => auth.set("tokenExpiresAt", e.to_rfc3339()),
        None => {
            auth.params.remove("tokenExpiresAt");
        }
    }
    if !token.token_type.is_empty() && !auth.params.contains_key("headerPrefix") {
        auth.set("headerPrefix", "Bearer");
    }
}

/// Whether the token in the params should be refreshed before sending.
pub fn needs_refresh(auth: &Auth, now: DateTime<Utc>) -> bool {
    if !auth.get_bool("autoRefreshToken") || auth.get("refreshToken").is_empty() {
        return false;
    }
    DateTime::parse_from_rfc3339(&auth.get("tokenExpiresAt"))
        .is_ok_and(|e| e.with_timezone(&Utc) <= now + Duration::seconds(30))
}

pub fn pkce_challenge(verifier: &str, method: &str) -> String {
    if method.eq_ignore_ascii_case("plain") {
        verifier.to_string()
    } else {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}

fn grant(auth: &Auth) -> String {
    match auth.get("grant_type") {
        g if g.is_empty() => "authorization_code".into(),
        g => g,
    }
}

/// The authorization URL to open in the browser for code/implicit flows.
pub fn authorization_url(auth: &Auth, redirect_uri: &str, state: &str, verifier: &str) -> Result<String, AuthError> {
    let mut url = url::Url::parse(&auth.get("authUrl")).map_err(|e| AuthError::Invalid(format!("Auth URL: {e}")))?;
    let g = grant(auth);
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", if g == "implicit" { "token" } else { "code" });
        q.append_pair("client_id", &auth.get("clientId"));
        q.append_pair("redirect_uri", redirect_uri);
        for (key, param) in [("scope", "scope"), ("audience", "audience"), ("resource", "resource")] {
            let v = auth.get(param);
            if !v.is_empty() {
                q.append_pair(key, &v);
            }
        }
        q.append_pair("state", state);
        if g == "authorization_code_with_pkce" {
            let method = match auth.get("challengeAlgorithm") {
                m if m.is_empty() => "S256".to_string(),
                m => m,
            };
            q.append_pair("code_challenge", &pkce_challenge(verifier, &method));
            q.append_pair("code_challenge_method", &method);
        }
    }
    Ok(url.to_string())
}

fn parse_token_response(status: u16, body: &str, name: &str, now: DateTime<Utc>) -> Result<OAuth2Token, AuthError> {
    let raw: Value = serde_json::from_str(body).unwrap_or_else(|_| {
        let map: serde_json::Map<String, Value> = url::form_urlencoded::parse(body.as_bytes())
            .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
            .collect();
        Value::Object(map)
    });
    if let Some(err) = raw.get("error").and_then(Value::as_str) {
        let desc = raw.get("error_description").and_then(Value::as_str).unwrap_or("");
        return Err(AuthError::Invalid(format!("token endpoint returned {err}: {desc}")));
    }
    let access = raw.get("access_token").and_then(Value::as_str).unwrap_or_default().to_string();
    if access.is_empty() {
        return Err(AuthError::Invalid(format!("no access_token in response (HTTP {status}): {body}")));
    }
    let expires_in = raw.get("expires_in").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
    Ok(OAuth2Token {
        id: crate::model::new_id(),
        name: if name.is_empty() { "Token".into() } else { name.into() },
        access_token: access,
        token_type: raw.get("token_type").and_then(Value::as_str).unwrap_or("Bearer").to_string(),
        refresh_token: raw.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        scope: raw.get("scope").and_then(Value::as_str).map(str::to_string),
        expires_at: expires_in.map(|s| now + Duration::seconds(s)),
        obtained_at: now,
        raw,
    })
}

async fn token_request(
    client: &reqwest::Client,
    auth: &Auth,
    mut form: Vec<(&str, String)>,
) -> Result<OAuth2Token, AuthError> {
    let url = auth.get("accessTokenUrl");
    if url.is_empty() {
        return Err(AuthError::Invalid("Access Token URL is required".into()));
    }
    for (key, param) in [("scope", "scope"), ("audience", "audience"), ("resource", "resource")] {
        let v = auth.get(param);
        if !v.is_empty() && !form.iter().any(|(k, _)| *k == key) {
            form.push((key, v));
        }
    }
    let mut rb = client.post(&url).header("Accept", "application/json");
    let (id, secret) = (auth.get("clientId"), auth.get("clientSecret"));
    if auth.get("client_authentication") == "body" {
        form.push(("client_id", id));
        if !secret.is_empty() {
            form.push(("client_secret", secret));
        }
    } else if !secret.is_empty() {
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        rb = rb.header("Authorization", format!("Basic {}", B64.encode(format!("{}:{}", enc(&id), enc(&secret)))));
    } else {
        form.push(("client_id", id));
    }
    let resp = rb.form(&form).send().await.map_err(|e| AuthError::Invalid(format!("token request failed: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| AuthError::Invalid(e.to_string()))?;
    parse_token_response(status, &body, &auth.get("tokenName"), Utc::now())
}

/// Exchange a refresh token for a new access token.
pub async fn refresh(client: &reqwest::Client, auth: &Auth) -> Result<OAuth2Token, AuthError> {
    let refresh = auth.get("refreshToken");
    let mut token =
        token_request(client, auth, vec![("grant_type", "refresh_token".into()), ("refresh_token", refresh.clone())])
            .await?;
    token.refresh_token.get_or_insert(refresh);
    Ok(token)
}

/// Obtain a new token using the auth's configured grant type. `open_browser`
/// is called with the authorization URL for browser-based flows.
pub async fn fetch_token(
    client: &reqwest::Client,
    auth: &Auth,
    open_browser: impl Fn(&str),
    timeout: std::time::Duration,
) -> Result<OAuth2Token, AuthError> {
    match grant(auth).as_str() {
        "client_credentials" => token_request(client, auth, vec![("grant_type", "client_credentials".into())]).await,
        "password_credentials" | "password" => {
            let form = vec![
                ("grant_type", "password".to_string()),
                ("username", auth.get("username")),
                ("password", auth.get("password")),
            ];
            token_request(client, auth, form).await
        }
        g @ ("authorization_code" | "authorization_code_with_pkce" | "implicit") => {
            let (listener, redirect_uri) = bind_redirect(&auth.get("redirect_uri")).await?;
            let state = match auth.get("state") {
                s if s.is_empty() => random_token(16),
                s => s,
            };
            let verifier = match auth.get("code_verifier") {
                v if v.is_empty() => random_token(64),
                v => v,
            };
            open_browser(&authorization_url(auth, &redirect_uri, &state, &verifier)?);
            let params = tokio::time::timeout(timeout, wait_for_callback(&listener))
                .await
                .map_err(|_| AuthError::Invalid("timed out waiting for the browser callback".into()))??;
            if let Some(err) = params.get("error") {
                return Err(AuthError::Invalid(format!(
                    "authorization failed: {err} {}",
                    params.get("error_description").map(String::as_str).unwrap_or("")
                )));
            }
            if params.get("state").is_some_and(|s| *s != state) {
                return Err(AuthError::Invalid("state mismatch in callback".into()));
            }
            if g == "implicit" {
                let body = serde_json::to_string(&params).unwrap_or_default();
                return parse_token_response(200, &body, &auth.get("tokenName"), Utc::now());
            }
            let code = params.get("code").cloned().ok_or_else(|| AuthError::Invalid("callback had no code".into()))?;
            let mut form =
                vec![("grant_type", "authorization_code".to_string()), ("code", code), ("redirect_uri", redirect_uri)];
            if g == "authorization_code_with_pkce" {
                form.push(("code_verifier", verifier));
            }
            token_request(client, auth, form).await
        }
        other => Err(AuthError::Unsupported(format!("OAuth 2.0 grant type {other}"))),
    }
}

/// Bind the loopback listener for `redirect_uri` (or a random port).
async fn bind_redirect(redirect_uri: &str) -> Result<(TcpListener, String), AuthError> {
    if redirect_uri.is_empty() {
        let l = TcpListener::bind("127.0.0.1:0").await.map_err(|e| AuthError::Invalid(e.to_string()))?;
        let port = l.local_addr().map_err(|e| AuthError::Invalid(e.to_string()))?.port();
        return Ok((l, format!("http://127.0.0.1:{port}/callback")));
    }
    let u = url::Url::parse(redirect_uri).map_err(|e| AuthError::Invalid(format!("Callback URL: {e}")))?;
    let host = u.host_str().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err(AuthError::Invalid(
            "Callback URL must be a loopback address such as http://127.0.0.1:PORT/callback".into(),
        ));
    }
    let port = u.port_or_known_default().unwrap_or(80);
    let l = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| AuthError::Invalid(format!("cannot listen on port {port}: {e}")))?;
    Ok((l, redirect_uri.to_string()))
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Courier</title>\
<body style=\"font-family:-apple-system,sans-serif;padding:3em\"><h2>Authorization complete</h2>\
<p>You can close this tab and return to Courier.</p></body>";

/// Fragment tokens never reach the server, so bounce them back as a query.
const FRAGMENT_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Courier</title><script>\
if (location.hash.length > 1) { location.replace(location.pathname + '?' + location.hash.substring(1) + '&_fragment=1'); }\
</script><body style=\"font-family:-apple-system,sans-serif;padding:3em\">Finishing sign-in…</body>";

async fn wait_for_callback(listener: &TcpListener) -> Result<HashMap<String, String>, AuthError> {
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| AuthError::Invalid(e.to_string()))?;
        let mut buf = vec![0u8; 16 * 1024];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        let done = params.contains_key("code") || params.contains_key("access_token") || params.contains_key("error");
        let page = if done { DONE_PAGE } else { FRAGMENT_PAGE };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
        if done {
            return Ok(params);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AuthType;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn token_json() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(
            r#"{"access_token":"AT","token_type":"Bearer","expires_in":3600,"refresh_token":"RT","scope":"read"}"#,
            "application/json",
        )
    }

    #[test]
    fn pkce_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", "S256"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(pkce_challenge("abc", "plain"), "abc");
    }

    #[tokio::test]
    async fn client_credentials_with_basic_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("authorization", "Basic aWQ6c2VjcmV0"))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("scope=read"))
            .respond_with(token_json())
            .mount(&server)
            .await;
        let auth = Auth::new(AuthType::OAuth2)
            .with("grant_type", "client_credentials")
            .with("accessTokenUrl", format!("{}/token", server.uri()))
            .with("clientId", "id")
            .with("clientSecret", "secret")
            .with("scope", "read");
        let t = fetch_token(&reqwest::Client::new(), &auth, |_| {}, std::time::Duration::from_secs(5)).await.unwrap();
        assert_eq!(t.access_token, "AT");
        assert_eq!(t.refresh_token.as_deref(), Some("RT"));
        assert!(t.expires_at.unwrap() > Utc::now());
    }

    #[tokio::test]
    async fn password_grant_with_body_credentials_and_errors() {
        let server = MockServer::start().await;
        Mock::given(path("/token"))
            .and(body_string_contains("grant_type=password"))
            .and(body_string_contains("username=u"))
            .and(body_string_contains("client_id=id"))
            .respond_with(token_json())
            .mount(&server)
            .await;
        Mock::given(path("/bad"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_raw(r#"{"error":"invalid_grant","error_description":"nope"}"#, "application/json"),
            )
            .mount(&server)
            .await;
        let auth = Auth::new(AuthType::OAuth2)
            .with("grant_type", "password_credentials")
            .with("accessTokenUrl", format!("{}/token", server.uri()))
            .with("clientId", "id")
            .with("clientSecret", "s")
            .with("client_authentication", "body")
            .with("username", "u")
            .with("password", "p");
        let client = reqwest::Client::new();
        assert_eq!(
            fetch_token(&client, &auth, |_| {}, std::time::Duration::from_secs(5)).await.unwrap().access_token,
            "AT"
        );
        let bad = auth.clone().with("accessTokenUrl", format!("{}/bad", server.uri()));
        let err = fetch_token(&client, &bad, |_| {}, std::time::Duration::from_secs(5)).await.unwrap_err();
        assert!(err.to_string().contains("invalid_grant"));
    }

    #[tokio::test]
    async fn authorization_code_with_pkce_via_loopback() {
        let server = MockServer::start().await;
        Mock::given(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains("code_verifier=myverifier"))
            .respond_with(token_json())
            .mount(&server)
            .await;
        let auth = Auth::new(AuthType::OAuth2)
            .with("grant_type", "authorization_code_with_pkce")
            .with("authUrl", "https://auth.example/authorize")
            .with("accessTokenUrl", format!("{}/token", server.uri()))
            .with("clientId", "id")
            .with("code_verifier", "myverifier");
        // Simulate the browser: read redirect_uri and state from the URL and call back.
        let browser = |url: &str| {
            let u = url::Url::parse(url).unwrap();
            let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
            assert_eq!(q["code_challenge"], pkce_challenge("myverifier", "S256"));
            let cb = format!("{}?code=the-code&state={}", q["redirect_uri"], q["state"]);
            tokio::spawn(async move {
                reqwest::get(cb).await.unwrap();
            });
        };
        let t = fetch_token(&reqwest::Client::new(), &auth, browser, std::time::Duration::from_secs(5)).await.unwrap();
        assert_eq!(t.access_token, "AT");
    }

    #[tokio::test]
    async fn refresh_keeps_old_refresh_token_when_absent() {
        let server = MockServer::start().await;
        Mock::given(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"access_token":"NEW"}"#, "application/json"))
            .mount(&server)
            .await;
        let auth = Auth::new(AuthType::OAuth2)
            .with("accessTokenUrl", format!("{}/token", server.uri()))
            .with("refreshToken", "R1");
        let t = refresh(&reqwest::Client::new(), &auth).await.unwrap();
        assert_eq!(t.access_token, "NEW");
        assert_eq!(t.refresh_token.as_deref(), Some("R1"));
    }

    #[test]
    fn token_params_and_refresh_check() {
        let mut auth = Auth::new(AuthType::OAuth2).with("autoRefreshToken", true);
        let token = OAuth2Token {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: Some(Utc::now() - Duration::seconds(5)),
            ..Default::default()
        };
        use_token(&mut auth, &token);
        assert_eq!(auth.get("accessToken"), "a");
        assert!(needs_refresh(&auth, Utc::now()));
        auth.set("autoRefreshToken", false);
        assert!(!needs_refresh(&auth, Utc::now()));
    }

    #[test]
    fn form_encoded_token_responses() {
        let t = parse_token_response(200, "access_token=x&token_type=bearer&expires_in=60", "", Utc::now()).unwrap();
        assert_eq!(t.access_token, "x");
        assert!(t.expires_at.is_some());
    }
}
