//! End-to-end tests for the executor and collection runner.

use courier::cookies::CookieJar;
use courier::exec::{ExecOptions, Executor};
use courier::model::*;
use courier::runner::{RunConfig, RunControl, RunStatus, parse_data_file, run_collection};
use courier::settings::Settings;
use courier::vars::VarContext;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request as WireRequest, ResponseTemplate};

fn executor() -> Executor {
    Executor::new(Settings::default(), Arc::new(CookieJar::default()))
}

fn item(name: &str, method: &str, url: &str, tests: &str) -> RequestItem {
    let mut i = RequestItem::new(name, Request::new(method, url));
    i.scripts.post_response = tests.to_string();
    i
}

async fn api() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"token":"T123"}"#, "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer T123"))
        .respond_with(|req: &WireRequest| {
            let user =
                req.url.query_pairs().find(|(k, _)| k == "user").map(|(_, v)| v.into_owned()).unwrap_or_default();
            ResponseTemplate::new(200).set_body_raw(format!(r#"{{"user":"{user}"}}"#), "application/json")
        })
        .mount(&server)
        .await;
    Mock::given(path("/me")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
    Mock::given(path("/never")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    server
}

fn collection(server: &MockServer) -> Collection {
    let mut c = Collection::new("API");
    c.variables.push(Variable::new("base", server.uri()));
    c.auth = Auth::new(AuthType::Bearer).with("token", "{{token}}");
    let mut login = item(
        "Login",
        "POST",
        "{{base}}/login",
        "pm.test('login ok', () => pm.response.to.have.status(200)); pm.environment.set('token', pm.response.json().token);",
    );
    login.request.auth = Auth::none();
    let mut folder = Folder::new("Users");
    folder.items.push(Item::Request(item(
        "Me",
        "GET",
        "{{base}}/me?user={{name}}",
        "pm.test('is user', () => pm.expect(pm.response.json().user).to.equal(pm.iterationData.get('name')));",
    )));
    folder.items.push(Item::Request(item("Never", "GET", "{{base}}/never", "")));
    c.items.push(Item::Request(login));
    c.items.push(Item::Folder(folder));
    c
}

fn env_vars() -> VarContext {
    VarContext { environment: Some(vec![]), ..Default::default() }
}

#[tokio::test]
async fn runs_iterations_with_data_and_variables() {
    let server = api().await;
    let mut c = collection(&server);
    // Skip "Never" using flow control from the Me request.
    if let Some(Item::Folder(f)) = c.items.get_mut(1)
        && let Item::Request(me) = &mut f.items[0]
    {
        me.scripts.post_response.push_str("\npm.execution.setNextRequest(null);");
    }
    let config = RunConfig {
        collection_id: c.id.clone(),
        iterations: 0,
        data: parse_data_file("name\nada\nbob\n").unwrap(),
        ..Default::default()
    };
    let mut events = 0;
    let (result, vars) =
        run_collection(&executor(), &c, &config, env_vars(), "Test", &RunControl::default(), |_| events += 1).await;
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.iterations.len(), 2);
    assert_eq!(result.iterations[1].requests.len(), 2, "Never was skipped by setNextRequest(null)");
    assert_eq!(result.totals.tests_passed, 4, "{:#?}", result.iterations);
    assert_eq!(result.totals.tests_failed, 0);
    assert!(events > 0);
    let token = vars.environment.unwrap().into_iter().find(|v| v.key == "token").unwrap();
    assert_eq!(token.value(), "T123");
    let junit = result.to_junit();
    assert!(junit.contains("Me (iteration 2)"));
}

#[tokio::test]
async fn stop_on_failure_and_selected_order() {
    let server = api().await;
    let c = collection(&server);
    let ids: Vec<String> = c.requests().iter().map(|r| r.id.clone()).collect();
    // Run "Me" before "Login": the token is missing, so the test fails.
    let config = RunConfig {
        collection_id: c.id.clone(),
        request_ids: vec![ids[1].clone(), ids[0].clone()],
        data: vec![[("name".to_string(), "ada".to_string())].into()],
        stop_on_failure: true,
        ..Default::default()
    };
    let (result, _) = run_collection(&executor(), &c, &config, env_vars(), "", &RunControl::default(), |_| {}).await;
    assert_eq!(result.status, RunStatus::Aborted, "{:#?}", result.iterations);
    assert_eq!(result.iterations[0].requests.len(), 1);
    assert_eq!(result.iterations[0].requests[0].status, Some(401));
    assert!(result.stop_reason.unwrap().contains("Me"));
}

#[tokio::test]
async fn stop_control_ends_run() {
    let server = api().await;
    let c = collection(&server);
    let control = RunControl::default();
    control.stop();
    let config = RunConfig { collection_id: c.id.clone(), iterations: 3, ..Default::default() };
    let (result, _) = run_collection(&executor(), &c, &config, env_vars(), "", &control, |_| {}).await;
    assert_eq!(result.status, RunStatus::Stopped);
    assert_eq!(result.totals.requests, 0);
}

#[tokio::test]
async fn executor_handles_skip_errors_and_digest() {
    let server = MockServer::start().await;
    Mock::given(path("/digest"))
        .and(header("authorization", "Digest username=\"u\""))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(path("/digest"))
        .and(wiremock::matchers::header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/digest"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("www-authenticate", "Digest realm=\"r\", qop=\"auth\", nonce=\"n\", algorithm=MD5"),
        )
        .mount(&server)
        .await;
    let ex = executor();
    let vars = VarContext::default();
    let opts = ExecOptions::default();
    let cancel = CancellationToken::new();

    let mut digest = item("Digest", "GET", &format!("{}/digest", server.uri()), "");
    digest.request.auth = Auth::new(AuthType::Digest).with("username", "u").with("password", "p");
    let e = ex.execute(&digest, None, vars.clone(), &opts, &cancel).await;
    assert_eq!(e.response.as_ref().map(|r| r.status), Some(200), "{:?}", e.error);
    assert!(e.sent.unwrap().header("authorization").unwrap().contains("response=\""));

    let mut skip = item("Skip", "GET", "http://127.0.0.1:1/", "");
    skip.scripts.pre_request = "pm.execution.skipRequest()".into();
    let e = ex.execute(&skip, None, vars.clone(), &opts, &cancel).await;
    assert!(e.skipped);
    assert!(e.response.is_none() && e.error.is_none());

    let mut broken = item("Broken", "GET", "http://127.0.0.1:1/", "");
    broken.scripts.pre_request = "throw new Error('nope')".into();
    let e = ex.execute(&broken, None, vars, &opts, &cancel).await;
    assert!(e.error.unwrap().contains("Error: nope"));
}

#[tokio::test]
async fn send_request_from_script_reaches_server() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"t":"S"}"#, "application/json"))
        .mount(&server)
        .await;
    Mock::given(path("/data"))
        .and(header("x-token", "S"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let mut it = item("Data", "GET", &format!("{}/data", server.uri()), "");
    it.request.headers.push(KeyValue::new("X-Token", "{{t}}"));
    it.scripts.pre_request = format!(
        "pm.sendRequest({{ url: '{}/token', method: 'POST', body: {{ mode: 'raw', raw: 'grant' }} }}, (e, r) => pm.variables.set('t', r.json().t));",
        server.uri()
    );
    let e =
        executor().execute(&it, None, VarContext::default(), &ExecOptions::default(), &CancellationToken::new()).await;
    assert_eq!(e.response.map(|r| r.status), Some(204), "{:?} {:?}", e.error, e.console);
}
