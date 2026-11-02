//! Request history, stored as one JSON file per day.

use crate::exec::Execution;
use crate::http::HttpResponse;
use crate::model::Request;
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Response bodies kept in history are capped at this size.
pub const BODY_CAP: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub name: String,
    /// The request as written (variables unresolved), for reopening.
    pub request: Request,
    pub method: String,
    /// The resolved URL that was sent.
    pub url: String,
    #[serde(default)]
    pub response: Option<HttpResponse>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub collection_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}

impl HistoryEntry {
    pub fn from_execution(
        name: &str,
        exec: &Execution,
        collection_id: Option<String>,
        request_id: Option<String>,
    ) -> Self {
        let mut response = exec.response.clone();
        if let Some(r) = response.as_mut()
            && r.body.len() > BODY_CAP
        {
            r.body.truncate(BODY_CAP);
            r.truncated = true;
        }
        let mut request = exec.request.clone();
        if request.body.raw.len() > BODY_CAP {
            request.body.raw.truncate(BODY_CAP);
        }
        Self {
            id: crate::model::new_id(),
            timestamp: Utc::now(),
            name: name.to_string(),
            method: exec.sent.as_ref().map(|s| s.method.clone()).unwrap_or_else(|| request.method.clone()),
            url: exec.sent.as_ref().map(|s| s.url.clone()).unwrap_or_else(|| request.url.clone()),
            request,
            response,
            error: exec.error.clone(),
            collection_id,
            request_id,
        }
    }

    pub fn day(&self) -> NaiveDate {
        self.timestamp.with_timezone(&Local).date_naive()
    }
}

/// History entries, newest first.
#[derive(Debug, Clone, Default)]
pub struct History {
    pub dir: PathBuf,
    pub entries: Vec<HistoryEntry>,
    pub limit: usize,
}

impl History {
    pub fn day_path(dir: &Path, day: NaiveDate) -> PathBuf {
        dir.join(format!("{}.json", day.format("%Y-%m-%d")))
    }

    /// Load every daily file in `dir`. Unreadable files are skipped.
    pub fn load(dir: &Path, limit: usize) -> Self {
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "json")
                    && let Ok(Some(list)) = crate::storage::read_json::<Vec<HistoryEntry>>(&path)
                {
                    entries.extend(list);
                }
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
        let mut h = Self { dir: dir.to_path_buf(), entries, limit };
        h.trim();
        h
    }

    /// Drop the oldest entries beyond the limit; returns days that changed.
    pub fn trim(&mut self) -> BTreeSet<NaiveDate> {
        let mut days = BTreeSet::new();
        while self.limit > 0 && self.entries.len() > self.limit {
            if let Some(e) = self.entries.pop() {
                days.insert(e.day());
            }
        }
        days
    }

    /// Add an entry; returns the days whose files need rewriting.
    pub fn add(&mut self, entry: HistoryEntry) -> BTreeSet<NaiveDate> {
        let mut days = BTreeSet::from([entry.day()]);
        self.entries.insert(0, entry);
        days.extend(self.trim());
        days
    }

    pub fn delete(&mut self, id: &str) -> BTreeSet<NaiveDate> {
        let mut days = BTreeSet::new();
        self.entries.retain(|e| {
            if e.id == id {
                days.insert(e.day());
                false
            } else {
                true
            }
        });
        days
    }

    pub fn clear(&mut self) -> BTreeSet<NaiveDate> {
        let days = self.entries.iter().map(HistoryEntry::day).collect();
        self.entries.clear();
        days
    }

    /// File contents for a day, or `None` when the day is now empty.
    pub fn day_file(&self, day: NaiveDate) -> (PathBuf, Option<Vec<u8>>) {
        let list: Vec<&HistoryEntry> = self.entries.iter().filter(|e| e.day() == day).collect();
        let path = Self::day_path(&self.dir, day);
        if list.is_empty() { (path, None) } else { (path, Some(crate::storage::to_json_bytes(&list))) }
    }

    /// Case-insensitive search over method, URL and name.
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.url.to_lowercase().contains(&q)
                    || e.method.to_lowercase().contains(&q)
                    || e.name.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Entries grouped by local day, newest day first.
    pub fn grouped<'a>(&'a self, query: &str) -> Vec<(NaiveDate, Vec<&'a HistoryEntry>)> {
        let mut groups: Vec<(NaiveDate, Vec<&HistoryEntry>)> = Vec::new();
        for e in self.search(query) {
            let day = e.day();
            match groups.last_mut() {
                Some((d, list)) if *d == day => list.push(e),
                _ => groups.push((day, vec![e])),
            }
        }
        groups
    }
}

/// "Today", "Yesterday" or a formatted date.
pub fn day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if day == today {
        "Today".into()
    } else if Some(day) == today.pred_opt() {
        "Yesterday".into()
    } else {
        day.format("%B %-d, %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, ts: DateTime<Utc>) -> HistoryEntry {
        HistoryEntry {
            id: crate::model::new_id(),
            timestamp: ts,
            name: url.into(),
            request: Request::new("GET", url),
            method: "GET".into(),
            url: url.into(),
            response: None,
            error: None,
            collection_id: None,
            request_id: None,
        }
    }

    #[test]
    fn add_trim_persist_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let mut h = History { dir: tmp.path().into(), entries: vec![], limit: 3 };
        let now = Utc::now();
        let old = now - chrono::Duration::days(3);
        let mut dirty = BTreeSet::new();
        dirty.extend(h.add(entry("http://old", old)));
        for i in 0..3 {
            dirty.extend(h.add(entry(&format!("http://new/{i}"), now + chrono::Duration::seconds(i))));
        }
        assert_eq!(h.entries.len(), 3);
        assert_eq!(h.entries[0].url, "http://new/2");
        for day in dirty {
            let (path, bytes) = h.day_file(day);
            match bytes {
                Some(b) => crate::storage::write_atomic(&path, &b).unwrap(),
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let reloaded = History::load(tmp.path(), 10);
        assert_eq!(reloaded.entries, h.entries);
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1, "old day file removed");
    }

    #[test]
    fn search_group_delete_clear() {
        let mut h = History { dir: "/tmp".into(), entries: vec![], limit: 0 };
        let now = Utc::now();
        h.add(entry("http://a/users", now - chrono::Duration::days(1)));
        h.add(entry("http://b/orders", now));
        assert_eq!(h.search("USERS").len(), 1);
        let groups = h.grouped("");
        assert_eq!(groups.len(), 2);
        assert_eq!(day_label(groups[0].0), "Today");
        assert_eq!(day_label(groups[1].0), "Yesterday");
        let id = h.entries[0].id.clone();
        h.delete(&id);
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.clear().len(), 1);
        assert!(h.entries.is_empty());
    }

    #[test]
    fn large_bodies_are_capped() {
        let exec = Execution {
            response: Some(HttpResponse { body: vec![b'x'; BODY_CAP + 10], ..Default::default() }),
            ..Default::default()
        };
        let e = HistoryEntry::from_execution("r", &exec, None, None);
        let r = e.response.unwrap();
        assert_eq!(r.body.len(), BODY_CAP);
        assert!(r.truncated);
    }
}
