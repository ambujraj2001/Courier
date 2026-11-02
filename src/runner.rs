//! Collection runner: iterations, data files, flow control, stop rules,
//! results and JUnit export.

use crate::exec::{ExecOptions, Executor};
use crate::model::Collection;
use crate::script::{NextRequest, TestResult};
use crate::vars::VarContext;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub type DataRow = BTreeMap<String, String>;

/// Parse a CSV (header row required) or JSON (array of objects) data file.
pub fn parse_data_file(text: &str) -> Result<Vec<DataRow>, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('[') {
        let v: Vec<serde_json::Value> =
            serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON data file: {e}"))?;
        return v
            .into_iter()
            .map(|row| match row {
                serde_json::Value::Object(o) => Ok(o
                    .into_iter()
                    .map(|(k, v)| {
                        let s = match v {
                            serde_json::Value::String(s) => s,
                            serde_json::Value::Null => String::new(),
                            other => other.to_string(),
                        };
                        (k, s)
                    })
                    .collect()),
                _ => Err("JSON data files must be an array of objects".to_string()),
            })
            .collect();
    }
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(trimmed.as_bytes());
    let headers = reader.headers().map_err(|e| format!("invalid CSV data file: {e}"))?.clone();
    reader
        .records()
        .map(|rec| {
            let rec = rec.map_err(|e| format!("invalid CSV data file: {e}"))?;
            Ok(headers.iter().zip(rec.iter()).map(|(h, v)| (h.to_string(), v.to_string())).collect())
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfig {
    pub collection_id: String,
    /// Run only this folder's requests.
    pub folder_id: Option<String>,
    /// Requests to run, in order. Empty means "all, in collection order".
    pub request_ids: Vec<String>,
    pub environment_id: Option<String>,
    /// 0 means "one per data row" (or 1 without data).
    pub iterations: usize,
    pub delay_ms: u64,
    #[serde(default)]
    pub data: Vec<DataRow>,
    #[serde(default)]
    pub data_file_name: Option<String>,
    pub keep_variable_values: bool,
    pub persist_responses: bool,
    pub stop_on_error: bool,
    pub stop_on_failure: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            collection_id: String::new(),
            folder_id: None,
            request_ids: Vec::new(),
            environment_id: None,
            iterations: 1,
            delay_ms: 0,
            data: Vec::new(),
            data_file_name: None,
            keep_variable_values: true,
            persist_responses: false,
            stop_on_error: false,
            stop_on_failure: false,
        }
    }
}

impl RunConfig {
    pub fn iteration_count(&self) -> usize {
        match (self.iterations, self.data.len()) {
            (0, 0) => 1,
            (0, n) => n,
            (i, _) => i,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RequestRun {
    pub request_id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub status_text: String,
    pub time_ms: f64,
    pub size: u64,
    pub tests: Vec<TestResult>,
    pub error: Option<String>,
    pub script_errors: Vec<String>,
    pub skipped: bool,
    #[serde(default)]
    pub request_headers: Vec<(String, String)>,
    #[serde(default)]
    pub response_headers: Vec<(String, String)>,
    #[serde(default)]
    pub response_body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IterationResult {
    pub index: usize,
    pub requests: Vec<RequestRun>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    #[default]
    Running,
    Completed,
    Stopped,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunTotals {
    pub requests: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub tests_skipped: usize,
    pub errors: usize,
    pub avg_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub id: String,
    pub collection_name: String,
    pub environment_name: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub config: Option<RunConfig>,
    pub iterations: Vec<IterationResult>,
    pub status: RunStatus,
    pub totals: RunTotals,
    /// Why the run stopped early, if it did.
    pub stop_reason: Option<String>,
}

impl RunResult {
    pub fn compute_totals(&mut self) {
        let mut t = RunTotals::default();
        let mut time = 0.0;
        for r in self.iterations.iter().flat_map(|i| &i.requests) {
            t.requests += 1;
            time += r.time_ms;
            if r.error.is_some() {
                t.errors += 1;
            }
            for test in &r.tests {
                if test.skipped {
                    t.tests_skipped += 1;
                } else if test.passed {
                    t.tests_passed += 1;
                } else {
                    t.tests_failed += 1;
                }
            }
        }
        t.avg_time_ms = if t.requests > 0 { time / t.requests as f64 } else { 0.0 };
        self.totals = t;
    }

    /// JUnit XML report: one test suite per request per iteration.
    pub fn to_junit(&self) -> String {
        fn esc(s: &str) -> String {
            s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
        }
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str(&format!(
            "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" time=\"{:.3}\">\n",
            esc(&self.collection_name),
            self.totals.tests_passed + self.totals.tests_failed + self.totals.tests_skipped,
            self.totals.tests_failed,
            self.iterations.iter().flat_map(|i| &i.requests).map(|r| r.time_ms).sum::<f64>() / 1000.0
        ));
        for it in &self.iterations {
            for r in &it.requests {
                let failures = r.tests.iter().filter(|t| !t.passed && !t.skipped).count();
                let skipped = r.tests.iter().filter(|t| t.skipped).count();
                out.push_str(&format!(
                    "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"{}\" skipped=\"{skipped}\" time=\"{:.3}\">\n",
                    esc(&format!("{} (iteration {})", r.name, it.index + 1)),
                    r.tests.len(),
                    usize::from(r.error.is_some()),
                    r.time_ms / 1000.0
                ));
                for t in &r.tests {
                    out.push_str(&format!("    <testcase name=\"{}\" classname=\"{}\"", esc(&t.name), esc(&r.name)));
                    if t.skipped {
                        out.push_str("><skipped/></testcase>\n");
                    } else if !t.passed {
                        let msg = t.error.clone().unwrap_or_default();
                        out.push_str(&format!(
                            "><failure message=\"{}\">{}</failure></testcase>\n",
                            esc(&msg),
                            esc(&msg)
                        ));
                    } else {
                        out.push_str("/>\n");
                    }
                }
                if let Some(e) = &r.error {
                    out.push_str(&format!(
                        "    <testcase name=\"request\" classname=\"{}\"><error message=\"{}\"/></testcase>\n",
                        esc(&r.name),
                        esc(e)
                    ));
                }
                out.push_str("  </testsuite>\n");
            }
        }
        out.push_str("</testsuites>\n");
        out
    }
}

/// Progress notifications while a run is in flight.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    IterationStarted(usize),
    RequestStarted { iteration: usize, request_id: String },
    RequestFinished { iteration: usize, run: Box<RequestRun> },
    Console(Vec<crate::script::ConsoleEntry>),
}

/// Pause/stop controls shared with the UI.
#[derive(Clone, Default)]
pub struct RunControl {
    pub cancel: CancellationToken,
    pub paused: Arc<AtomicBool>,
}

impl RunControl {
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
    pub fn stop(&self) {
        self.cancel.cancel();
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// Upper bound on requests per iteration, to stop runaway setNextRequest loops.
pub const MAX_STEPS_PER_ITERATION: usize = 10_000;

/// Run a collection (or folder). Returns the result and the final variables.
pub async fn run_collection(
    executor: &Executor,
    collection: &Collection,
    config: &RunConfig,
    vars: VarContext,
    environment_name: &str,
    control: &RunControl,
    mut on_event: impl FnMut(RunEvent),
) -> (RunResult, VarContext) {
    let order: Vec<String> = if config.request_ids.is_empty() {
        collection.requests_under(config.folder_id.as_deref()).iter().map(|r| r.id.clone()).collect()
    } else {
        config.request_ids.clone()
    };
    let iterations = config.iteration_count();
    let mut result = RunResult {
        id: crate::model::new_id(),
        collection_name: collection.name.clone(),
        environment_name: environment_name.to_string(),
        started_at: Utc::now(),
        config: Some(RunConfig { data: Vec::new(), ..config.clone() }),
        ..Default::default()
    };
    let mut vars = vars;
    // The collection being run is the source of truth for its variables.
    vars.collection = collection.variables.clone();
    let mut first_request = true;

    'outer: for iteration in 0..iterations {
        on_event(RunEvent::IterationStarted(iteration));
        vars.data = if config.data.is_empty() {
            DataRow::new()
        } else {
            config.data[iteration.min(config.data.len() - 1)].clone()
        };
        let mut it = IterationResult { index: iteration, requests: Vec::new() };
        let mut idx = 0;
        let mut steps = 0;
        while idx < order.len() {
            steps += 1;
            if steps > MAX_STEPS_PER_ITERATION {
                result.stop_reason = Some("too many requests in one iteration (setNextRequest loop?)".into());
                result.iterations.push(it);
                result.status = RunStatus::Aborted;
                break 'outer;
            }
            while control.is_paused() && !control.cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if control.cancel.is_cancelled() {
                result.iterations.push(it);
                result.status = RunStatus::Stopped;
                result.stop_reason = Some("stopped by user".into());
                break 'outer;
            }
            if !first_request && config.delay_ms > 0 {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(config.delay_ms)) => {}
                    _ = control.cancel.cancelled() => continue,
                }
            }
            first_request = false;

            let Some(item) = collection.find_request(&order[idx]) else {
                idx += 1;
                continue;
            };
            on_event(RunEvent::RequestStarted { iteration, request_id: item.id.clone() });
            let opts = ExecOptions {
                environment_name: environment_name.to_string(),
                iteration,
                iteration_count: iterations,
                download_to: None,
            };
            let exec = executor.execute(item, Some(collection), vars.clone(), &opts, &control.cancel).await;
            vars = exec.vars.clone();
            if !exec.console.is_empty() {
                on_event(RunEvent::Console(exec.console.clone()));
            }
            let run = RequestRun {
                request_id: item.id.clone(),
                name: item.name.clone(),
                method: exec.sent.as_ref().map(|s| s.method.clone()).unwrap_or_else(|| exec.request.method.clone()),
                url: exec.sent.as_ref().map(|s| s.url.clone()).unwrap_or_else(|| vars.resolve(&exec.request.url)),
                status: exec.response.as_ref().map(|r| r.status),
                status_text: exec.response.as_ref().map(|r| r.status_text.clone()).unwrap_or_default(),
                time_ms: exec.response.as_ref().map(|r| r.timings.total_ms).unwrap_or(0.0),
                size: exec.response.as_ref().map(|r| r.total_size()).unwrap_or(0),
                tests: exec.tests.clone(),
                error: exec.error.clone(),
                script_errors: exec.script_errors.clone(),
                skipped: exec.skipped,
                request_headers: if config.persist_responses {
                    exec.sent.as_ref().map(|s| s.headers.clone()).unwrap_or_default()
                } else {
                    Vec::new()
                },
                response_headers: if config.persist_responses {
                    exec.response.as_ref().map(|r| r.headers.clone()).unwrap_or_default()
                } else {
                    Vec::new()
                },
                response_body: if config.persist_responses { exec.response.as_ref().map(|r| r.text()) } else { None },
            };
            let had_error = run.error.is_some();
            let had_failure = run.tests.iter().any(|t| !t.passed && !t.skipped);
            on_event(RunEvent::RequestFinished { iteration, run: Box::new(run.clone()) });
            it.requests.push(run);

            if had_error && config.stop_on_error && !control.cancel.is_cancelled() {
                result.stop_reason = Some(format!("{} failed: {}", item.name, exec.error.clone().unwrap_or_default()));
                result.iterations.push(it);
                result.status = RunStatus::Aborted;
                break 'outer;
            }
            if had_failure && config.stop_on_failure {
                result.stop_reason = Some(format!("a test failed in {}", item.name));
                result.iterations.push(it);
                result.status = RunStatus::Aborted;
                break 'outer;
            }
            idx = match &exec.next_request {
                NextRequest::Unset => idx + 1,
                NextRequest::Stop => order.len(),
                NextRequest::Named(target) => {
                    match order
                        .iter()
                        .position(|id| id == target || collection.find_request(id).is_some_and(|r| &r.name == target))
                    {
                        Some(p) => p,
                        None => {
                            on_event(RunEvent::Console(vec![crate::script::ConsoleEntry {
                                level: "warn".into(),
                                message: format!(
                                    "setNextRequest: no request named “{target}” in this run; ending the iteration"
                                ),
                            }]));
                            order.len()
                        }
                    }
                }
            };
        }
        result.iterations.push(it);
    }
    if result.status == RunStatus::Running {
        result.status = RunStatus::Completed;
    }
    result.finished_at = Some(Utc::now());
    result.compute_totals();
    (result, vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_and_json_data() {
        let rows = parse_data_file("id,name\n1,\"Ada, L\"\n2,Bob\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Ada, L");
        let rows = parse_data_file(r#"[{"id": 1, "ok": true, "n": null}, {"id": "2"}]"#).unwrap();
        assert_eq!(rows[0]["id"], "1");
        assert_eq!(rows[0]["ok"], "true");
        assert_eq!(rows[0]["n"], "");
        assert!(parse_data_file("[1,2]").is_err());
    }

    #[test]
    fn iteration_count_rules() {
        let mut c = RunConfig { iterations: 0, ..Default::default() };
        assert_eq!(c.iteration_count(), 1);
        c.data = vec![DataRow::new(), DataRow::new()];
        assert_eq!(c.iteration_count(), 2);
        c.iterations = 5;
        assert_eq!(c.iteration_count(), 5);
    }

    #[test]
    fn junit_escapes_and_counts() {
        let mut r = RunResult { collection_name: "A&B".into(), ..Default::default() };
        r.iterations.push(IterationResult {
            index: 0,
            requests: vec![RequestRun {
                name: "Get <x>".into(),
                tests: vec![
                    TestResult { name: "ok".into(), passed: true, skipped: false, error: None, event: "test".into() },
                    TestResult {
                        name: "bad".into(),
                        passed: false,
                        skipped: false,
                        error: Some("expected \"a\"".into()),
                        event: "test".into(),
                    },
                ],
                ..Default::default()
            }],
        });
        r.compute_totals();
        let xml = r.to_junit();
        assert!(xml.contains("<testsuites name=\"A&amp;B\" tests=\"2\" failures=\"1\""));
        assert!(xml.contains("Get &lt;x&gt; (iteration 1)"));
        assert!(xml.contains("<failure message=\"expected &quot;a&quot;\">"));
    }
}
