use super::*;
use crate::http::HttpResponse;
use crate::model::{Body, KeyValue, RawLanguage, Variable};

fn response(code: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status: code,
        status_text: if code == 200 { "OK".into() } else { "Not Found".into() },
        headers: vec![("Content-Type".into(), "application/json".into()), ("X-Trace".into(), "abc".into())],
        body: body.as_bytes().to_vec(),
        body_size: body.len() as u64,
        ..Default::default()
    }
}

fn input() -> ScriptInput {
    let mut vars = VarContext::default();
    vars.environment = Some(vec![Variable::new("host", "env.io")]);
    vars.collection = vec![Variable::new("c", "1")];
    vars.globals = vec![Variable::new("g", "x")];
    vars.data.insert("row".into(), "7".into());
    ScriptInput {
        request: Request::new("GET", "http://{{host}}/a?x=1"),
        response: Some(response(200, r#"{"user":{"id":42,"tags":["a","b"]},"ok":true}"#)),
        vars,
        environment_name: "Dev".into(),
        info: ScriptInfo { request_name: "Get A".into(), iteration: 0, iteration_count: 1, ..Default::default() },
        ..Default::default()
    }
}

fn run1(code: &str, event: Event) -> ScriptResult {
    run(&[code.to_string()], event, &input(), &ScriptHost::default())
}

fn passed(r: &ScriptResult) -> Vec<(String, bool)> {
    r.tests.iter().map(|t| (t.name.clone(), t.passed)).collect()
}

#[test]
fn response_assertions_and_expect() {
    let r = run1(
        r#"
        pm.test("status", () => pm.response.to.have.status(200));
        pm.test("reason", () => pm.response.to.have.status("OK"));
        pm.test("header", () => pm.response.to.have.header("x-trace", "abc"));
        pm.test("json", () => {
            const j = pm.response.json();
            pm.expect(j.user.id).to.equal(42);
            pm.expect(j.user.tags).to.have.lengthOf(2).and.include("a");
            pm.expect(j).to.have.nested.property("user.tags[1]", "b");
            pm.expect(j.user).to.have.property("id").that.is.a("number").above(40);
            pm.expect(j).to.deep.include({ ok: true });
            pm.expect(j.user).to.include.keys("id");
            pm.expect(j.user.tags).to.have.length.above(1);
            pm.expect("hello").to.match(/ell/).and.not.be.empty;
            pm.expect([1, 2]).to.have.members([2, 1]);
            pm.expect(3).to.be.oneOf([1, 3]).and.within(2, 4);
            pm.expect(null).to.be.null;
            pm.expect(() => { throw new TypeError("boom") }).to.throw(TypeError);
        });
        pm.test("jsonBody", () => pm.response.to.have.jsonBody("user.id", 42));
        pm.test("success", () => pm.response.to.be.success.and.to.be.json);
        pm.test("fails", () => pm.expect(1).to.eql(2));
        pm.test("not", () => pm.response.to.not.have.status(404));
        pm.test.skip("skipped", () => {});
        "#,
        Event::Test,
    );
    assert_eq!(r.error, None);
    let p = passed(&r);
    assert_eq!(p.len(), 9);
    assert!(p.iter().filter(|(n, _)| n != "fails" && n != "skipped").all(|(_, ok)| *ok), "{:?}", r.tests);
    let fail = r.tests.iter().find(|t| t.name == "fails").unwrap();
    assert!(!fail.passed);
    assert_eq!(fail.error.as_deref(), Some("expected 1 to deeply equal 2"));
    assert!(r.tests.iter().find(|t| t.name == "skipped").unwrap().skipped);
}

#[test]
fn variables_flow_out_of_scripts() {
    let r = run1(
        r#"
        pm.environment.set("token", "t1");
        pm.environment.set("host", { a: 1 });
        pm.collectionVariables.set("c", 2);
        pm.globals.unset("g");
        pm.variables.set("loc", "L");
        pm.test("reads", () => {
            pm.expect(pm.variables.get("row")).to.equal("7");
            pm.expect(pm.iterationData.get("row")).to.equal("7");
            pm.expect(pm.variables.get("loc")).to.equal("L");
            pm.expect(pm.environment.name).to.equal("Dev");
            pm.expect(pm.variables.replaceIn("{{loc}}-{{c}}")).to.equal("L-2");
            pm.expect(pm.variables.replaceIn("{{$guid}}")).to.have.lengthOf(36);
        });
        "#,
        Event::Test,
    );
    assert_eq!(r.error, None);
    assert!(r.tests[0].passed, "{:?}", r.tests);
    let env = r.vars.environment.as_ref().unwrap();
    assert_eq!(env.iter().find(|v| v.key == "token").unwrap().current.as_deref(), Some("t1"));
    assert_eq!(env.iter().find(|v| v.key == "host").unwrap().current.as_deref(), Some(r#"{"a":1}"#));
    assert_eq!(r.vars.collection[0].current.as_deref(), Some("2"));
    assert!(r.vars.globals.is_empty());
    assert_eq!(r.vars.local["loc"], "L");
}

#[test]
fn pre_request_can_modify_request_and_control_flow() {
    let mut inp = input();
    inp.response = None;
    inp.request.body = Body::raw("{}", RawLanguage::Json);
    let code = r#"
        pm.request.headers.add({ key: "X-Added", value: "1" });
        pm.request.headers.upsert({ key: "x-added", value: "2" });
        pm.request.url.query.add({ key: "y", value: "2" });
        pm.request.body.raw = JSON.stringify({ n: 1 });
        pm.request.method = "post";
        console.log("host is", pm.request.url.getHost(), { a: 1 });
        pm.execution.setNextRequest("Other");
        pm.execution.skipRequest();
    "#;
    let r = run(&[code.into()], Event::PreRequest, &inp, &ScriptHost::default());
    assert_eq!(r.error, None);
    assert_eq!(r.request.method, "POST");
    assert_eq!(r.request.url, "http://{{host}}/a?x=1&y=2");
    assert_eq!(r.request.headers, vec![KeyValue::new("X-Added", "2")]);
    assert_eq!(r.request.body.raw, r#"{"n":1}"#);
    assert_eq!(r.next_request, NextRequest::Named("Other".into()));
    assert!(r.skip_request);
    assert_eq!(r.console[0].message, r#"host is {{host}} {"a":1}"#);

    let r = run1("postman.setNextRequest(null);", Event::Test);
    assert_eq!(r.next_request, NextRequest::Stop);
}

#[test]
fn errors_are_reported_with_context() {
    let r = run1("pm.test('a', () => {});\nundefinedFunction();", Event::Test);
    assert_eq!(r.error.as_deref(), Some("ReferenceError: undefinedFunction is not defined (line 2)"));
    assert_eq!(r.tests.len(), 1, "tests before the error are kept");

    let r = run1("this is not js", Event::Test);
    assert!(r.error.unwrap().contains("SyntaxError"));

    let r = run(
        &["while (true) {}".into()],
        Event::Test,
        &input(),
        &ScriptHost { timeout: Duration::from_millis(200), ..Default::default() },
    );
    assert_eq!(r.error.as_deref(), Some("Script timed out after 200 ms"));
}

#[test]
fn chain_shares_state_and_isolates_locals() {
    let scripts = vec!["var x = 1; pm.variables.set('a', 'from-collection');".to_string(), "pm.test('sees', () => pm.expect(typeof x).to.equal('undefined') && pm.expect(pm.variables.get('a')).to.equal('from-collection'));".to_string()];
    let r = run(&scripts, Event::Test, &input(), &ScriptHost::default());
    assert_eq!(passed(&r), vec![("sees".into(), true)]);
}

#[test]
fn legacy_api_and_async_tests() {
    let r = run1(
        r#"
        tests["legacy ok"] = responseCode.code === 200;
        tests["legacy body"] = JSON.parse(responseBody).ok;
        postman.setEnvironmentVariable("legacy", "1");
        pm.test("async done", (done) => setTimeout(() => { pm.expect(1).to.equal(1); done(); }, 10));
        pm.test("never done", (done) => {});
        pm.test("promise", () => Promise.resolve().then(() => pm.expect(1).to.equal(2)));
        "#,
        Event::Test,
    );
    assert_eq!(r.error, None);
    let p = passed(&r);
    assert!(p.contains(&("async done".into(), true)));
    assert!(p.contains(&("never done".into(), false)));
    assert!(p.contains(&("promise".into(), false)));
    assert!(p.contains(&("legacy ok".into(), true)));
    assert!(p.contains(&("legacy body".into(), true)));
    assert!(r.vars.environment.unwrap().iter().any(|v| v.key == "legacy"));
}

#[test]
fn send_request_uses_host() {
    let host = ScriptHost {
        send: Arc::new(|spec: SendSpec| {
            assert_eq!(spec.method, "POST");
            assert_eq!(spec.header[0].key, "A");
            Ok(response(200, r#"{"token":"abc"}"#))
        }),
        ..Default::default()
    };
    let code = r#"
        pm.sendRequest({ url: "http://x/login", method: "post", header: { A: "1" }, body: { mode: "raw", raw: "{}" } }, (err, res) => {
            pm.environment.set("token", res.json().token);
            pm.test("cb", () => pm.expect(err).to.be.null);
        });
    "#;
    let r = run(&[code.into()], Event::PreRequest, &input(), &host);
    assert_eq!(r.error, None);
    assert!(r.tests[0].passed);
    assert!(r.vars.environment.unwrap().iter().any(|v| v.key == "token" && v.value() == "abc"));
}

#[test]
fn crypto_encoding_xml_and_schema() {
    let r = run1(
        r#"
        pm.test("crypto", () => {
            pm.expect(CryptoJS.MD5("abc").toString()).to.equal("900150983cd24fb0d6963f7d28e17f72");
            pm.expect(CryptoJS.SHA256("abc").toString()).to.equal("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
            pm.expect(CryptoJS.HmacSHA256("msg", "key").toString(CryptoJS.enc.Base64)).to.be.a("string");
            pm.expect(CryptoJS.enc.Base64.stringify(CryptoJS.enc.Utf8.parse("hi"))).to.equal("aGk=");
            pm.expect(btoa("hello")).to.equal("aGVsbG8=");
            pm.expect(atob("aGVsbG8=")).to.equal("hello");
            pm.expect(require("uuid").v4()).to.have.lengthOf(36);
        });
        pm.test("xml", () => {
            const j = xml2Json('<?xml version="1.0"?><root a="1"><item>x</item><item>y</item><e/></root>');
            pm.expect(j.root.$.a).to.equal("1");
            pm.expect(j.root.item).to.eql(["x", "y"]);
        });
        pm.test("schema", () => {
            pm.response.to.have.jsonSchema({ type: "object", required: ["user"], properties: { user: { type: "object", properties: { id: { type: "integer" } } } } });
            pm.expect({ a: "x" }).to.not.have.jsonSchema({ properties: { a: { type: "number" } } });
        });
        pm.test("require unknown", () => pm.expect(() => require("lodash")).to.throw("not available"));
        "#,
        Event::Test,
    );
    assert_eq!(r.error, None);
    assert!(r.tests.iter().all(|t| t.passed), "{:?}", r.tests);
}

#[test]
fn empty_scripts_are_noops() {
    let r = run(&["  ".into()], Event::Test, &input(), &ScriptHost::default());
    assert!(r.tests.is_empty());
    assert_eq!(r.vars, input().vars);
}
