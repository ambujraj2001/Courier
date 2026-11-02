//! Keeping the raw URL, the query-param table and path variables in sync.
//!
//! These functions work on the raw text the user typed (which may contain
//! `{{variables}}` and therefore is not always a valid URL), so they never
//! decode or re-encode values.

use super::KeyValue;

/// Split a raw URL into (base, query, fragment). Query and fragment exclude
/// their leading `?` / `#`.
pub fn split(raw: &str) -> (&str, Option<&str>, Option<&str>) {
    let (rest, fragment) = match raw.find('#') {
        Some(i) => (&raw[..i], Some(&raw[i + 1..])),
        None => (raw, None),
    };
    match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..]), fragment),
        None => (rest, None, fragment),
    }
}

/// Parse the query string of a raw URL into enabled rows.
pub fn parse_query(raw: &str) -> Vec<KeyValue> {
    let (_, query, _) = split(raw);
    let Some(query) = query else { return Vec::new() };
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            KeyValue::new(k, v)
        })
        .collect()
}

/// Combine freshly parsed enabled params with the previous table so that
/// descriptions survive and disabled rows keep their positions.
pub fn merge_query(previous: &[KeyValue], parsed: &[KeyValue]) -> Vec<KeyValue> {
    let mut enabled_prev = previous.iter().filter(|p| p.enabled);
    let mut out: Vec<KeyValue> = parsed
        .iter()
        .map(|p| {
            let mut row = p.clone();
            if let Some(old) = enabled_prev.next()
                && old.key == p.key
            {
                row.description = old.description.clone();
            }
            row
        })
        .collect();
    for (idx, row) in previous.iter().enumerate() {
        if !row.enabled {
            let at = idx.min(out.len());
            out.insert(at, row.clone());
        }
    }
    out
}

/// Rebuild a raw URL with the enabled rows of `query`.
pub fn with_query(raw: &str, query: &[KeyValue]) -> String {
    let (base, _, fragment) = split(raw);
    let parts: Vec<String> = query
        .iter()
        .filter(|q| q.enabled && !(q.key.is_empty() && q.value.is_empty()))
        .map(|q| if q.value.is_empty() { q.key.clone() } else { format!("{}={}", q.key, q.value) })
        .collect();
    let mut out = base.to_string();
    if !parts.is_empty() {
        out.push('?');
        out.push_str(&parts.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Names of `:name` path segments, in order.
pub fn path_variable_names(raw: &str) -> Vec<String> {
    let (base, _, _) = split(raw);
    // Skip the scheme and host so `http://host:8080` ports are not matched.
    let path_start = match base.find("://") {
        Some(i) => base[i + 3..].find('/').map(|p| p + i + 3).unwrap_or(base.len()),
        None => base.find('/').unwrap_or(base.len()),
    };
    base[path_start..]
        .split('/')
        .filter_map(|seg| seg.strip_prefix(':'))
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect::<String>())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Path variables for `raw`, keeping values from `previous` where names match.
pub fn merge_path_variables(previous: &[KeyValue], raw: &str) -> Vec<KeyValue> {
    path_variable_names(raw)
        .into_iter()
        .map(|name| previous.iter().find(|p| p.key == name).cloned().unwrap_or_else(|| KeyValue::new(name, "")))
        .collect()
}

/// Replace `:name` path segments with their values.
pub fn substitute_path_variables(raw: &str, vars: &[KeyValue]) -> String {
    if vars.is_empty() {
        return raw.to_string();
    }
    let (base, query, fragment) = split(raw);
    let (prefix, path) = match base.find("://") {
        Some(i) => match base[i + 3..].find('/') {
            Some(p) => base.split_at(p + i + 3),
            None => (base, ""),
        },
        None => ("", base),
    };
    let path: Vec<String> = path
        .split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(name) => match vars.iter().find(|v| v.key == name) {
                Some(v) if !v.value.is_empty() => v.value.clone(),
                _ => seg.to_string(),
            },
            None => seg.to_string(),
        })
        .collect();
    let mut out = format!("{prefix}{}", path.join("/"));
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query() {
        let q = parse_query("https://x.io/a?b=1&c&d=x=y#frag");
        assert_eq!(q, vec![KeyValue::new("b", "1"), KeyValue::new("c", ""), KeyValue::new("d", "x=y")]);
    }

    #[test]
    fn rebuilds_url_keeping_fragment_and_skipping_disabled() {
        let rows = vec![KeyValue::new("a", "1"), KeyValue::new("b", "2").disabled(), KeyValue::new("flag", "")];
        assert_eq!(with_query("{{base}}/x?old=1#top", &rows), "{{base}}/x?a=1&flag#top");
        assert_eq!(with_query("http://h/x?old=1", &[]), "http://h/x");
    }

    #[test]
    fn merge_keeps_disabled_rows_and_descriptions() {
        let mut first = KeyValue::new("a", "1");
        first.description = "the a".into();
        let prev = vec![first, KeyValue::new("off", "x").disabled(), KeyValue::new("b", "2")];
        let merged = merge_query(&prev, &parse_query("u?a=9&b=2&c=3"));
        assert_eq!(merged.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["a", "off", "b", "c"]);
        assert_eq!(merged[0].description, "the a");
        assert_eq!(merged[0].value, "9");
        assert!(!merged[1].enabled);
    }

    #[test]
    fn path_variables_ignore_port() {
        assert_eq!(path_variable_names("http://localhost:8080/users/:id/posts/:postId?x=:no"), ["id", "postId"]);
        assert_eq!(path_variable_names("{{base}}/:org"), ["org"]);
    }

    #[test]
    fn substitutes_path_variables() {
        let vars = vec![KeyValue::new("id", "42")];
        assert_eq!(substitute_path_variables("http://h:1/u/:id/:other?q=1", &vars), "http://h:1/u/42/:other?q=1");
    }
}
