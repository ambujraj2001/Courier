//! Integration tests for the HTTP engine against a local mock server.

use courier::cookies::CookieJar;
use courier::http::{HttpEngine, HttpError, PreparedBody, prepare};
use courier::model::*;
use courier::settings::Settings;
use courier::vars::VarContext;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string, body_string_contains, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx(server: &MockServer) -> VarContext {
    let mut v = VarContext::default();
    v.environment = Some(vec![Variable::new("base", server.uri()), Variable::new("id", "7")]);
    v
}

async fn send(req: &Request, server: &MockServer) -> Result<courier::http::HttpResponse, HttpError> {
    let engine = HttpEngine::new(Settings::default());
    let prepared = prepare(req, &ctx(server), &Settings::default());
    engine.send(&prepared, None, &CancellationToken::new()).await
}

#[tokio::test]
async fn get_with_query_headers_and_path_variables() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/7"))
        .and(query_param("q", "a b"))
        .and(header("x-token", "7"))
        .and(header_exists("user-agent"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("{\"ok\":true}", "application/json"))
        .mount(&server)
        .await;
    let mut req = Request::new("GET", "{{base}}/users/:id?q=a b");
    req.path_variables[0].value = "{{id}}".into();
    req.headers.push(KeyValue::new("X-Token", "{{id}}"));
    let res = send(&req, &server).await.unwrap();
    assert_eq!(res.status, 200);
    assert_eq!(res.status_text, "OK");
    assert_eq!(res.text(), "{\"ok\":true}");
    assert_eq!(res.content_type(), "application/json");
    assert!(res.timings.total_ms > 0.0);
    assert_eq!(res.body_size, 11);
}

#[tokio::test]
async fn post_raw_json_and_urlencoded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/json"))
        .and(header("content-type", "application/json"))
        .and(body_string("{\"id\": 7}"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/form"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string("a=1&b=7"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let mut req = Request::new("POST", "{{base}}/json");
    req.body = Body::raw("{\"id\": {{id}}}", RawLanguage::Json);
    assert_eq!(send(&req, &server).await.unwrap().status, 201);

    let mut req = Request::new("POST", "{{base}}/form");
    req.body.mode = BodyMode::Urlencoded;
    req.body.urlencoded = vec![KeyValue::new("a", "1"), KeyValue::new("b", "{{id}}")];
    assert_eq!(send(&req, &server).await.unwrap().status, 204);
}

#[tokio::test]
async fn multipart_and_binary_file_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "file-contents").unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .and(body_string_contains("name=\"caption\""))
        .and(body_string_contains("filename=\"note.txt\""))
        .and(body_string_contains("file-contents"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/blob"))
        .and(body_string("file-contents"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mut req = Request::new("POST", "{{base}}/upload");
    req.body.mode = BodyMode::Formdata;
    req.body.formdata = vec![
        FormField { key: "caption".into(), value: "hi".into(), enabled: true, ..Default::default() },
        FormField {
            key: "f".into(),
            kind: FormFieldKind::File,
            files: vec![file.display().to_string()],
            enabled: true,
            ..Default::default()
        },
    ];
    assert_eq!(send(&req, &server).await.unwrap().status, 200);

    let mut req = Request::new("PUT", "{{base}}/blob");
    req.body.mode = BodyMode::File;
    req.body.file = Some(file.display().to_string());
    assert_eq!(send(&req, &server).await.unwrap().status, 200);

    req.body.file = Some("/definitely/missing".into());
    assert!(matches!(send(&req, &server).await, Err(HttpError::File(..))));
}

#[tokio::test]
async fn redirects_follow_rules() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
        .mount(&server)
        .await;
    Mock::given(path("/keep"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "/b"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .respond_with(ResponseTemplate::new(200).set_body_string("got"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/b"))
        .respond_with(ResponseTemplate::new(200).set_body_string("posted"))
        .mount(&server)
        .await;
    Mock::given(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
        .mount(&server)
        .await;

    let mut req = Request::new("POST", "{{base}}/a");
    req.body = Body::raw("x", RawLanguage::Text);
    let res = send(&req, &server).await.unwrap();
    assert_eq!(res.text(), "got", "302 switches POST to GET");
    assert_eq!(res.redirects.len(), 1);

    req.set_url("{{base}}/keep");
    assert_eq!(send(&req, &server).await.unwrap().text(), "posted", "307 keeps the method");

    req.set_url("{{base}}/a");
    req.settings.insert("followOriginalHttpMethod".into(), true.into());
    assert_eq!(send(&req, &server).await.unwrap().text(), "posted");

    req.settings.insert("followRedirects".into(), false.into());
    assert_eq!(send(&req, &server).await.unwrap().status, 302);

    let mut req = Request::new("GET", "{{base}}/loop");
    req.settings.insert("maxRedirects".into(), 3.into());
    assert_eq!(send(&req, &server).await, Err(HttpError::TooManyRedirects(3)));
}

#[tokio::test]
async fn cookies_are_stored_and_sent() {
    let server = MockServer::start().await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "sid=abc; Path=/"))
        .mount(&server)
        .await;
    Mock::given(path("/me"))
        .and(header("cookie", "sid=abc"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/me")).respond_with(ResponseTemplate::new(401)).mount(&server).await;

    let jar = CookieJar::default();
    let engine = HttpEngine::new(Settings::default());
    let vars = ctx(&server);
    let s = Settings::default();
    let cancel = CancellationToken::new();
    engine.send(&prepare(&Request::new("GET", "{{base}}/login"), &vars, &s), Some(&jar), &cancel).await.unwrap();
    let me = engine.send(&prepare(&Request::new("GET", "{{base}}/me"), &vars, &s), Some(&jar), &cancel).await.unwrap();
    assert_eq!(me.status, 200);

    let mut no_cookies = Request::new("GET", "{{base}}/me");
    no_cookies.settings.insert("disableCookies".into(), true.into());
    let me = engine.send(&prepare(&no_cookies, &vars, &s), Some(&jar), &cancel).await.unwrap();
    assert_eq!(me.status, 401);
}

#[tokio::test]
async fn timeout_and_cancel() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let mut req = Request::new("GET", "{{base}}/slow");
    req.settings.insert("timeout".into(), 100.into());
    assert_eq!(send(&req, &server).await, Err(HttpError::Timeout(100)));

    let engine = HttpEngine::new(Settings::default());
    let cancel = CancellationToken::new();
    let prepared = prepare(&Request::new("GET", "{{base}}/slow"), &ctx(&server), &Settings::default());
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        c2.cancel();
    });
    assert_eq!(engine.send(&prepared, None, &cancel).await, Err(HttpError::Cancelled));
}

#[tokio::test]
async fn large_bodies_are_truncated_and_downloads_stream_to_disk() {
    let server = MockServer::start().await;
    Mock::given(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 3 * 1024 * 1024]))
        .mount(&server)
        .await;
    let settings = Settings { max_response_size_mb: 1, ..Settings::default() };
    let engine = HttpEngine::new(settings.clone());
    let mut prepared = prepare(&Request::new("GET", "{{base}}/big"), &ctx(&server), &settings);
    let res = engine.send(&prepared, None, &CancellationToken::new()).await.unwrap();
    assert!(res.truncated);
    assert_eq!(res.body.len(), 1024 * 1024);
    assert_eq!(res.body_size, 3 * 1024 * 1024);

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("big.bin");
    prepared.options.download_to = Some(out.clone());
    let res = engine.send(&prepared, None, &CancellationToken::new()).await.unwrap();
    assert!(res.body.is_empty());
    assert_eq!(std::fs::metadata(out).unwrap().len(), 3 * 1024 * 1024);
}

#[tokio::test]
async fn bad_url_and_connection_errors() {
    let engine = HttpEngine::new(Settings::default());
    let mut p = prepare(&Request::new("GET", "http://exa mple"), &VarContext::default(), &Settings::default());
    assert!(matches!(engine.send(&p, None, &CancellationToken::new()).await, Err(HttpError::InvalidUrl(_))));
    p.url = "http://127.0.0.1:1/".into();
    p.body = PreparedBody::None;
    assert!(matches!(engine.send(&p, None, &CancellationToken::new()).await, Err(HttpError::Network(_))));
}
