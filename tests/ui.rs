//! UI tests: drive the real egui app headlessly.
//!
//! Set `COURIER_SNAPSHOT_DIR` to also write rendered PNGs of key screens.

use courier::app::{CourierApp, RespState, TabKind};
use courier::model::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn harness(dir: PathBuf) -> Harness<'static, Option<CourierApp>> {
    // Rendering needs a GPU adapter; only set it up when snapshots are wanted
    // so the UI tests also run on headless CI machines.
    let mut builder = Harness::builder().with_size(egui::vec2(1360.0, 860.0));
    if std::env::var_os("COURIER_SNAPSHOT_DIR").is_some() {
        builder = builder.wgpu();
    }
    builder.build_state(
        move |ctx, app: &mut Option<CourierApp>| {
            let app = app.get_or_insert_with(|| {
                egui_extras::install_image_loaders(ctx);
                CourierApp::new(ctx, dir.clone())
            });
            app.ui(ctx);
        },
        None,
    )
}

fn app<'a>(h: &'a mut Harness<'static, Option<CourierApp>>) -> &'a mut CourierApp {
    h.state_mut().as_mut().expect("app created on first frame")
}

/// Step until `done` holds (background work finishes on other threads).
fn step_until(h: &mut Harness<'static, Option<CourierApp>>, mut done: impl FnMut(&CourierApp) -> bool) {
    let start = Instant::now();
    loop {
        h.step();
        if h.state().as_ref().is_some_and(&mut done) {
            h.step();
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            let a = h.state().as_ref().unwrap();
            let console: Vec<&str> = a.console.iter().map(|l| l.text.as_str()).collect();
            let toasts: Vec<&str> = a.toasts.iter().map(|t| t.text.as_str()).collect();
            panic!("timed out waiting for the UI; console={console:?} toasts={toasts:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn snapshot(h: &mut Harness<'static, Option<CourierApp>>, name: &str) {
    if let Some(dir) = std::env::var_os("COURIER_SNAPSHOT_DIR") {
        let img = h.render().expect("render");
        img.save(Path::new(&dir).join(format!("{name}.png"))).expect("save snapshot");
    }
}

fn seed(dir: &Path, server: &str) {
    let data = courier::storage::DataDir::open(dir).unwrap();
    let mut c = courier::formats::postman::import_collection(include_str!("fixtures/collection_v21.json")).unwrap();
    c.variables[0].initial = server.to_string();
    data.save_collection(&c).unwrap();
    let mut env = Environment::new("Local Dev");
    env.variables.push(Variable::new("token", "s3cret"));
    data.save_environment(&env).unwrap();
}

/// The mock server needs its own runtime: the app owns one too, and a
/// runtime cannot be dropped inside another runtime's async context.
fn mock_server() -> (tokio::runtime::Runtime, MockServer) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Grace"}]"#, "application/json"),
            )
            .mount(&server)
            .await;
        server
    });
    (rt, server)
}

#[test]
fn send_request_and_show_response() {
    let (_rt, server) = mock_server();
    let tmp = tempfile::tempdir().unwrap();
    seed(tmp.path(), &server.uri());
    let mut h = harness(tmp.path().to_path_buf());
    h.step();
    app(&mut h).focus_url = true;
    h.step();
    h.step();
    snapshot(&mut h, "01-empty");

    // Open the fixture's "List users" request from the collection.
    let (cid, rid) = {
        let a = app(&mut h);
        let c = &a.collections[0];
        (c.id.clone(), c.requests()[0].id.clone())
    };
    app(&mut h).open_request(&cid, &rid);
    app(&mut h).expanded.insert(cid.clone());
    if let Some(courier::model::Item::Folder(f)) = app(&mut h).collections[0].items.first() {
        let fid = f.id.clone();
        app(&mut h).expanded.insert(fid);
    }
    h.step();
    h.get_by_label("Send").click();
    step_until(
        &mut h,
        |a| matches!(&a.tabs[a.active_tab].kind, TabKind::Request(r) if matches!(r.resp, RespState::Done(_))),
    );
    h.step();
    snapshot(&mut h, "02-response");
    h.get_by_label_contains("200 OK");
    h.get_by_label("Test Results (1/1)");

    let a = app(&mut h);
    assert_eq!(a.history.entries.len(), 1);
    assert_eq!(a.history.entries[0].response.as_ref().unwrap().status, 200);
}

#[test]
fn edit_save_and_reload_persists() {
    let tmp = tempfile::tempdir().unwrap();
    seed(tmp.path(), "http://127.0.0.1:9");
    let (cid, rid) = {
        let mut h = harness(tmp.path().to_path_buf());
        h.step();
        let a = app(&mut h);
        let cid = a.collections[0].id.clone();
        let rid = a.collections[0].requests()[1].id.clone();
        a.open_request(&cid, &rid);
        if let TabKind::Request(r) = &mut a.tabs[a.active_tab].kind {
            r.item.request.headers.push(KeyValue::new("X-From-Test", "yes"));
        }
        h.step();
        let a = app(&mut h);
        assert!(a.tab_is_dirty(a.active_tab));
        h.get_by_label("Save").click();
        h.step();
        let a = app(&mut h);
        assert!(!a.tab_is_dirty(a.active_tab));
        a.shutdown();
        (cid, rid)
    };
    // A fresh app sees the saved header and the restored tab.
    let mut h = harness(tmp.path().to_path_buf());
    h.step();
    let a = app(&mut h);
    let req = a.collection(&cid).unwrap().find_request(&rid).unwrap();
    assert!(req.request.headers.iter().any(|h| h.key == "X-From-Test"));
    assert!(
        a.tabs.iter().any(|t| matches!(&t.kind, TabKind::Request(r) if r.source.as_ref().map(|s| &s.1) == Some(&rid)))
    );
}

#[test]
fn screens_render() {
    let tmp = tempfile::tempdir().unwrap();
    seed(tmp.path(), "https://api.example.com");
    let mut h = harness(tmp.path().to_path_buf());
    h.step();
    let (cid, env_id) = {
        let a = app(&mut h);
        (a.collections[0].id.clone(), a.environments[0].id.clone())
    };
    app(&mut h).active_env = Some(env_id.clone());
    app(&mut h).open_node(&cid, None);
    h.step();
    h.step();
    snapshot(&mut h, "03-collection");
    app(&mut h).open_environment(Some(&env_id));
    h.step();
    h.step();
    snapshot(&mut h, "04-environment");
    app(&mut h).open_runner(&cid, None);
    h.step();
    h.step();
    snapshot(&mut h, "05-runner");
    app(&mut h).dialog = Some(courier::app::Dialog::Import(Default::default()));
    h.step();
    h.step();
    snapshot(&mut h, "06-import");
    assert!(h.get_all_by_label("Import").count() >= 2, "dialog is open");
    app(&mut h).dialog = None;
    app(&mut h).settings.theme = courier::settings::Theme::Light;
    courier::app::theme::apply(&app(&mut h).egui_ctx.clone(), courier::settings::Theme::Light);
    app(&mut h).new_request_tab();
    if let TabKind::Request(r) = &mut app(&mut h).tabs.last_mut().unwrap().kind {
        r.item.request.set_url("{{baseUrl}}/orders?status=open&page={{page}}");
        r.item.request.method = "POST".into();
        r.item.request.body = Body::raw("{\n  \"item\": \"{{$guid}}\",\n  \"qty\": 2\n}", RawLanguage::Json);
        r.section = courier::app::ReqSection::Body;
    }
    let last = app(&mut h).tabs.len() - 1;
    app(&mut h).active_tab = last;
    h.step();
    h.step();
    snapshot(&mut h, "07-light-body");

    // More screens for visual review.
    let step2 = |h: &mut Harness<'static, Option<CourierApp>>| {
        h.step();
        h.step();
    };
    app(&mut h).dialog = Some(courier::app::Dialog::NewItem);
    step2(&mut h);
    snapshot(&mut h, "10-light-new");
    app(&mut h).dialog = Some(courier::app::Dialog::Shortcuts);
    step2(&mut h);
    snapshot(&mut h, "11-light-shortcuts");
    app(&mut h).dialog = None;
    courier::app::theme::apply(&app(&mut h).egui_ctx.clone(), courier::settings::Theme::Dark);
    app(&mut h).sidebar = courier::app::SidebarSection::Environments;
    app(&mut h).console_open = true;
    app(&mut h).log(&[
        courier::script::ConsoleEntry {
            level: "http".into(),
            message: "GET https://api.example.com/users → 200 OK (84 ms)".into(),
        },
        courier::script::ConsoleEntry { level: "log".into(), message: "token refreshed".into() },
        courier::script::ConsoleEntry { level: "warn".into(), message: "Unresolved variable {{page}} in URL".into() },
    ]);
    if let TabKind::Request(r) = &mut app(&mut h).tabs.last_mut().unwrap().kind {
        r.section = courier::app::ReqSection::Auth;
        r.item.request.auth = Auth::new(AuthType::OAuth2).with("grant_type", "client_credentials");
    }
    step2(&mut h);
    snapshot(&mut h, "12-dark-auth-console");
    app(&mut h).console_open = false;
    app(&mut h).sidebar = courier::app::SidebarSection::History;
    if let TabKind::Request(r) = &mut app(&mut h).tabs.last_mut().unwrap().kind {
        r.section = courier::app::ReqSection::Settings;
    }
    step2(&mut h);
    snapshot(&mut h, "13-dark-settings-history");
    let settings = courier::app::dialogs::SettingsDialog::new(&app(&mut h).settings);
    app(&mut h).dialog = Some(courier::app::Dialog::Settings(settings));
    step2(&mut h);
    snapshot(&mut h, "14-dark-settings-dialog");
}

#[test]
fn import_dialog_and_runner_work() {
    let (_rt, server) = mock_server();
    let tmp = tempfile::tempdir().unwrap();
    let mut h = harness(tmp.path().to_path_buf());
    h.step();
    assert!(app(&mut h).collections.is_empty());

    // Paste a collection into the import dialog and confirm.
    let text = include_str!("fixtures/collection_v21.json")
        .replace("https://api.example.com", &server.uri())
        .replace("https://example.com", &server.uri());
    let mut dlg = courier::app::dialogs::ImportDialog::default();
    dlg.set_text(text);
    app(&mut h).dialog = Some(courier::app::Dialog::Import(dlg));
    h.step();
    h.get_by_label_contains("Collection “Courier Fixture” with 7 requests");
    let import_buttons: Vec<_> = h.get_all_by_label("Import").collect();
    import_buttons.last().unwrap().click();
    h.step();
    h.step();
    assert_eq!(app(&mut h).collections.len(), 1);
    assert!(app(&mut h).dialog.is_none());

    // Run the whole collection from the runner tab.
    let cid = app(&mut h).collections[0].id.clone();
    app(&mut h).open_runner(&cid, None);
    h.step();
    h.get_by_label("Run Courier Fixture").click();
    step_until(
        &mut h,
        |a| matches!(&a.tabs[a.active_tab].kind, TabKind::Runner(r) if matches!(r.state, courier::app::runner_tab::RunState::Done)),
    );
    snapshot(&mut h, "08-runner-results");
    let a = app(&mut h);
    let TabKind::Runner(r) = &a.tabs[a.active_tab].kind else { panic!() };
    assert_eq!(r.result.totals.requests, 7);
    assert_eq!(r.result.totals.tests_passed, 1, "List users passes its status test");

    // Settings and snippet dialogs render.
    let settings = courier::app::dialogs::SettingsDialog::new(&a.settings);
    a.dialog = Some(courier::app::Dialog::Settings(settings));
    h.step();
    h.step();
    snapshot(&mut h, "09-settings");
    h.get_by_label("Cancel").click();
    h.step();
    assert!(app(&mut h).dialog.is_none());
}
