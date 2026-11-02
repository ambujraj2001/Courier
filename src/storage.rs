//! Local JSON storage: data directory layout, atomic writes, corrupt-file
//! handling and a debounced background writer.

use crate::formats::postman::{self, Scope};
use crate::model::{Collection, Environment};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Version stamped into every file Courier writes.
pub const FORMAT_VERSION: u64 = 1;

/// A file that could not be loaded. The file is moved aside, never deleted.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadIssue {
    pub path: PathBuf,
    pub moved_to: Option<PathBuf>,
    pub error: String,
}

/// Default data directory: `$COURIER_DATA_DIR`, else the platform data dir.
pub fn default_location() -> PathBuf {
    if let Some(dir) = std::env::var_os("COURIER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    directories::BaseDirs::new().map(|d| d.data_dir().join("Courier")).unwrap_or_else(|| PathBuf::from(".courier"))
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

/// Write `bytes` to `path` atomically: temp file, fsync, rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        restrict(&tmp, 0o600);
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Read a JSON file. A missing file yields `Ok(None)`. A file that fails to
/// parse is renamed to `*.corrupt-<timestamp>` and reported as an issue.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, LoadIssue> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LoadIssue { path: path.into(), moved_to: None, error: e.to_string() }),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| quarantine(path, e.to_string()))
}

fn quarantine(path: &Path, error: String) -> LoadIssue {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let name = format!("{}.corrupt-{stamp}", path.file_name().unwrap_or_default().to_string_lossy());
    let target = path.with_file_name(name);
    let moved_to = fs::rename(path, &target).ok().map(|_| target);
    LoadIssue { path: path.into(), moved_to, error }
}

pub fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("model types always serialize")
}

/// Make an id safe to use as a file name.
pub fn file_stem(id: &str) -> String {
    let s: String =
        id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "_".into() } else { s }
}

/// The Courier data directory and the files inside it.
#[derive(Debug, Clone)]
pub struct DataDir {
    pub root: PathBuf,
}

impl DataDir {
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        for sub in ["", "collections", "environments", "history", "runs"] {
            let dir = root.join(sub);
            fs::create_dir_all(&dir)?;
            restrict(&dir, 0o700);
        }
        Ok(Self { root })
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    pub fn workspace_path(&self) -> PathBuf {
        self.root.join("workspace.json")
    }
    pub fn globals_path(&self) -> PathBuf {
        self.root.join("globals.json")
    }
    pub fn cookies_path(&self) -> PathBuf {
        self.root.join("cookies.json")
    }
    pub fn tokens_path(&self) -> PathBuf {
        self.root.join("oauth_tokens.json")
    }
    pub fn history_dir(&self) -> PathBuf {
        self.root.join("history")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }
    pub fn collection_path(&self, id: &str) -> PathBuf {
        self.root.join("collections").join(format!("{}.json", file_stem(id)))
    }
    pub fn environment_path(&self, id: &str) -> PathBuf {
        self.root.join("environments").join(format!("{}.json", file_stem(id)))
    }

    fn json_files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> =
            fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|e| e == "json"));
        files.sort();
        files
    }

    // ---- collections

    pub fn collection_bytes(c: &Collection) -> Vec<u8> {
        let mut v = postman::collection_to_value(c, true);
        v["_courier"] = serde_json::json!({ "format": FORMAT_VERSION });
        to_json_bytes(&v)
    }

    pub fn save_collection(&self, c: &Collection) -> io::Result<()> {
        write_atomic(&self.collection_path(&c.id), &Self::collection_bytes(c))
    }

    pub fn delete_collection(&self, id: &str) -> io::Result<()> {
        remove_if_exists(&self.collection_path(id))
    }

    pub fn load_collections(&self) -> (Vec<Collection>, Vec<LoadIssue>) {
        let mut out = Vec::new();
        let mut issues = Vec::new();
        for path in Self::json_files(&self.root.join("collections")) {
            match read_json::<Value>(&path) {
                Ok(Some(mut v)) => {
                    if let Some(o) = v.as_object_mut() {
                        o.remove("_courier");
                    }
                    match postman::collection_from_value(v) {
                        Ok(c) => out.push(c),
                        Err(e) => issues.push(quarantine(&path, e.to_string())),
                    }
                }
                Ok(None) => {}
                Err(issue) => issues.push(issue),
            }
        }
        out.sort_by_key(|c| c.name.to_lowercase());
        (out, issues)
    }

    // ---- environments & globals

    pub fn environment_bytes(env: &Environment, scope: Scope) -> Vec<u8> {
        let mut v = postman::environment_to_value(env, scope, false, true);
        v["_courier"] = serde_json::json!({ "format": FORMAT_VERSION });
        to_json_bytes(&v)
    }

    pub fn save_environment(&self, env: &Environment) -> io::Result<()> {
        write_atomic(&self.environment_path(&env.id), &Self::environment_bytes(env, Scope::Environment))
    }

    pub fn delete_environment(&self, id: &str) -> io::Result<()> {
        remove_if_exists(&self.environment_path(id))
    }

    fn load_env_file(path: &Path) -> Result<Option<Environment>, LoadIssue> {
        match read_json::<Value>(path)? {
            Some(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.remove("_courier");
                }
                postman::environment_from_value(v).map(|(e, _)| Some(e)).map_err(|e| quarantine(path, e.to_string()))
            }
            None => Ok(None),
        }
    }

    pub fn load_environments(&self) -> (Vec<Environment>, Vec<LoadIssue>) {
        let mut out = Vec::new();
        let mut issues = Vec::new();
        for path in Self::json_files(&self.root.join("environments")) {
            match Self::load_env_file(&path) {
                Ok(Some(e)) => out.push(e),
                Ok(None) => {}
                Err(i) => issues.push(i),
            }
        }
        out.sort_by_key(|e| e.name.to_lowercase());
        (out, issues)
    }

    pub fn save_globals(&self, globals: &Environment) -> io::Result<()> {
        write_atomic(&self.globals_path(), &Self::environment_bytes(globals, Scope::Globals))
    }

    pub fn load_globals(&self) -> (Environment, Option<LoadIssue>) {
        match Self::load_env_file(&self.globals_path()) {
            Ok(Some(g)) => (g, None),
            Ok(None) => (Environment::new("Globals"), None),
            Err(i) => (Environment::new("Globals"), Some(i)),
        }
    }

    // ---- generic documents (settings, workspace, cookies, tokens)

    pub fn load_doc<T: DeserializeOwned + Default>(&self, path: &Path) -> (T, Option<LoadIssue>) {
        match read_json(path) {
            Ok(Some(v)) => (v, None),
            Ok(None) => (T::default(), None),
            Err(i) => (T::default(), Some(i)),
        }
    }

    pub fn save_doc<T: Serialize>(&self, path: &Path, value: &T) -> io::Result<()> {
        write_atomic(path, &to_json_bytes(value))
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

enum Job {
    Write(PathBuf, Vec<u8>),
    Delete(PathBuf),
    Flush(Sender<()>),
}

/// Background writer that coalesces writes to the same path and performs
/// them off the UI thread, in order, after a short debounce.
#[derive(Clone)]
pub struct Persister {
    tx: Sender<Job>,
}

impl Persister {
    pub fn new(debounce: Duration) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("courier-persist".into())
            .spawn(move || {
                // Pending operations by path; `None` means delete.
                let mut pending: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
                let mut order: Vec<PathBuf> = Vec::new();
                let mut deadline: Option<Instant> = None;
                let run = |pending: &mut HashMap<PathBuf, Option<Vec<u8>>>, order: &mut Vec<PathBuf>| {
                    for path in order.drain(..) {
                        match pending.remove(&path) {
                            Some(Some(bytes)) => {
                                if let Err(e) = write_atomic(&path, &bytes) {
                                    eprintln!("courier: failed to write {}: {e}", path.display());
                                }
                            }
                            Some(None) => {
                                let _ = remove_if_exists(&path);
                            }
                            None => {}
                        }
                    }
                };
                loop {
                    let msg = match deadline {
                        Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    };
                    match msg {
                        Ok(Job::Write(path, bytes)) => {
                            if pending.insert(path.clone(), Some(bytes)).is_none() {
                                order.push(path);
                            }
                            deadline.get_or_insert_with(|| Instant::now() + debounce);
                        }
                        Ok(Job::Delete(path)) => {
                            if pending.insert(path.clone(), None).is_none() {
                                order.push(path);
                            }
                            deadline.get_or_insert_with(|| Instant::now() + debounce);
                        }
                        Ok(Job::Flush(done)) => {
                            run(&mut pending, &mut order);
                            deadline = None;
                            let _ = done.send(());
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            run(&mut pending, &mut order);
                            deadline = None;
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            run(&mut pending, &mut order);
                            break;
                        }
                    }
                }
            })
            .expect("spawn persister thread");
        Self { tx }
    }

    pub fn write(&self, path: PathBuf, bytes: Vec<u8>) {
        let _ = self.tx.send(Job::Write(path, bytes));
    }

    pub fn delete(&self, path: PathBuf) {
        let _ = self.tx.send(Job::Delete(path));
    }

    /// Block until every queued operation has hit the disk.
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn data_dir() -> (tempfile::TempDir, DataDir) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::open(tmp.path().join("Courier")).unwrap();
        (tmp, dir)
    }

    #[test]
    fn collections_round_trip_with_current_values() {
        let (_t, dir) = data_dir();
        let mut c = Collection::new("API");
        c.variables.push(Variable { current: Some("cur".into()), ..Variable::new("v", "init") });
        c.items.push(Item::Request(RequestItem::new("r", Request::new("GET", "http://x/:id?a=1"))));
        dir.save_collection(&c).unwrap();
        let (loaded, issues) = dir.load_collections();
        assert!(issues.is_empty());
        assert_eq!(loaded, vec![c.clone()]);
        dir.delete_collection(&c.id).unwrap();
        assert!(dir.load_collections().0.is_empty());
    }

    #[test]
    fn environments_and_globals() {
        let (_t, dir) = data_dir();
        let mut env = Environment::new("Dev");
        env.variables.push(Variable { secret: true, current: Some("s".into()), ..Variable::new("k", "v") });
        dir.save_environment(&env).unwrap();
        assert_eq!(dir.load_environments().0, vec![env]);
        let (g, issue) = dir.load_globals();
        assert!(issue.is_none());
        assert_eq!(g.name, "Globals");
        let mut g = g;
        g.variables.push(Variable::new("a", "1"));
        dir.save_globals(&g).unwrap();
        assert_eq!(dir.load_globals().0, g);
    }

    #[test]
    fn corrupt_files_are_quarantined_not_deleted() {
        let (_t, dir) = data_dir();
        let bad = dir.root.join("collections").join("bad.json");
        fs::write(&bad, "{ not json").unwrap();
        let (loaded, issues) = dir.load_collections();
        assert!(loaded.is_empty());
        assert_eq!(issues.len(), 1);
        let moved = issues[0].moved_to.clone().unwrap();
        assert!(moved.exists());
        assert_eq!(fs::read_to_string(moved).unwrap(), "{ not json");
        assert!(!bad.exists());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, dir) = data_dir();
        dir.save_doc(&dir.settings_path(), &serde_json::json!({"a": 1})).unwrap();
        let mode = fs::metadata(dir.settings_path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn persister_coalesces_and_flushes() {
        let (_t, dir) = data_dir();
        let p = Persister::new(Duration::from_secs(60));
        let path = dir.root.join("x.json");
        p.write(path.clone(), b"1".to_vec());
        p.write(path.clone(), b"2".to_vec());
        assert!(!path.exists(), "debounced");
        p.flush();
        assert_eq!(fs::read(&path).unwrap(), b"2");
        p.delete(path.clone());
        p.flush();
        assert!(!path.exists());
    }

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("a/b..c"), "a_b__c");
        assert_eq!(file_stem("ok-id_1"), "ok-id_1");
    }
}
