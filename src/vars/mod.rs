//! Variable scopes and `{{variable}}` resolution.

pub mod dynamic;

use crate::model::{Variable, VariableList};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum nesting depth for variables that reference other variables.
pub const MAX_DEPTH: usize = 20;

/// Where a resolved variable came from, highest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeKind {
    Local,
    Data,
    Environment,
    Collection,
    Global,
    Dynamic,
}

impl ScopeKind {
    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::Local => "Local",
            ScopeKind::Data => "Data",
            ScopeKind::Environment => "Environment",
            ScopeKind::Collection => "Collection",
            ScopeKind::Global => "Global",
            ScopeKind::Dynamic => "Dynamic",
        }
    }
}

/// Every variable scope visible to one request execution.
///
/// The context owns copies of the stores; after scripts have run, callers
/// write `environment`, `collection` and `globals` back to their sources.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarContext {
    pub local: BTreeMap<String, String>,
    pub data: BTreeMap<String, String>,
    pub environment: Option<Vec<Variable>>,
    pub collection: Vec<Variable>,
    pub globals: Vec<Variable>,
}

impl VarContext {
    /// Look a variable up through the scopes in precedence order.
    pub fn lookup(&self, name: &str) -> Option<(String, ScopeKind)> {
        if let Some(v) = self.local.get(name) {
            return Some((v.clone(), ScopeKind::Local));
        }
        if let Some(v) = self.data.get(name) {
            return Some((v.clone(), ScopeKind::Data));
        }
        if let Some(v) = self.environment.as_ref().and_then(|e| e.get_var(name)) {
            return Some((v.to_string(), ScopeKind::Environment));
        }
        if let Some(v) = self.collection.get_var(name) {
            return Some((v.to_string(), ScopeKind::Collection));
        }
        if let Some(v) = self.globals.get_var(name) {
            return Some((v.to_string(), ScopeKind::Global));
        }
        if name.starts_with('$') && dynamic::is_known(name) {
            return Some((String::new(), ScopeKind::Dynamic));
        }
        None
    }

    fn value_of(&self, name: &str) -> Option<String> {
        if name.starts_with('$') {
            // A user-defined variable can shadow a dynamic one.
            if let Some((v, scope)) = self.lookup(name)
                && scope != ScopeKind::Dynamic
            {
                return Some(v);
            }
            return dynamic::generate(name);
        }
        self.lookup(name).map(|(v, _)| v)
    }

    /// Replace every resolvable `{{name}}` in `text`. Unresolved references
    /// are left as-is.
    pub fn resolve(&self, text: &str) -> String {
        self.resolve_depth(text, 0, &mut Vec::new())
    }

    /// Like [`resolve`](Self::resolve) but also reports unresolved names.
    pub fn resolve_report(&self, text: &str) -> (String, Vec<String>) {
        let mut missing = Vec::new();
        let out = self.resolve_depth(text, 0, &mut missing);
        missing.dedup();
        (out, missing)
    }

    fn resolve_depth(&self, text: &str, depth: usize, missing: &mut Vec<String>) -> String {
        if !text.contains("{{") {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = &after[..end];
            // `{{a{{b}}` - treat the inner-most opening as the reference.
            if let Some(inner) = name.rfind("{{") {
                out.push_str(&rest[start..start + 2 + inner]);
                rest = &rest[start + 2 + inner..];
                continue;
            }
            match (depth < MAX_DEPTH).then(|| self.value_of(name.trim())).flatten() {
                Some(value) => out.push_str(&self.resolve_depth(&value, depth + 1, missing)),
                None => {
                    missing.push(name.trim().to_string());
                    out.push_str(&rest[start..start + 2 + end + 2]);
                }
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    /// Variable names referenced in `text` (for highlighting and autocomplete).
    pub fn references(text: &str) -> Vec<(usize, usize, String)> {
        let mut refs = Vec::new();
        let mut offset = 0;
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else { break };
            let name = &after[..end];
            if !name.contains("{{") && !name.is_empty() {
                let s = offset + start;
                refs.push((s, s + end + 4, name.trim().to_string()));
            }
            let consumed = start + 2 + end + 2;
            offset += consumed;
            rest = &rest[consumed..];
        }
        refs
    }

    /// Every variable name visible in this context, for autocomplete.
    pub fn all_names(&self) -> Vec<(String, ScopeKind)> {
        let mut out: Vec<(String, ScopeKind)> = Vec::new();
        let mut push = |name: &str, scope| {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.to_string(), scope));
            }
        };
        self.local.keys().for_each(|k| push(k, ScopeKind::Local));
        self.data.keys().for_each(|k| push(k, ScopeKind::Data));
        if let Some(env) = &self.environment {
            env.iter().filter(|v| v.enabled).for_each(|v| push(&v.key, ScopeKind::Environment));
        }
        self.collection.iter().filter(|v| v.enabled).for_each(|v| push(&v.key, ScopeKind::Collection));
        self.globals.iter().filter(|v| v.enabled).for_each(|v| push(&v.key, ScopeKind::Global));
        dynamic::NAMES.iter().for_each(|n| push(n, ScopeKind::Dynamic));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> VarContext {
        let mut c = VarContext::default();
        c.globals.push(Variable::new("host", "global.io"));
        c.globals.push(Variable::new("only_global", "g"));
        c.collection.push(Variable::new("host", "collection.io"));
        c.collection.push(Variable::new("base", "https://{{host}}/{{version}}"));
        c.environment = Some(vec![Variable::new("host", "env.io"), Variable::new("version", "v1")]);
        c
    }

    #[test]
    fn precedence_local_data_env_collection_global() {
        let mut c = ctx();
        assert_eq!(c.resolve("{{host}}"), "env.io");
        c.data.insert("host".into(), "data.io".into());
        assert_eq!(c.resolve("{{host}}"), "data.io");
        c.local.insert("host".into(), "local.io".into());
        assert_eq!(c.resolve("{{host}}"), "local.io");
        assert_eq!(c.lookup("only_global"), Some(("g".into(), ScopeKind::Global)));
        c.environment = None;
        c.data.clear();
        c.local.clear();
        assert_eq!(c.resolve("{{host}}"), "collection.io");
    }

    #[test]
    fn nested_resolution() {
        assert_eq!(ctx().resolve("{{base}}/users"), "https://env.io/v1/users");
    }

    #[test]
    fn unresolved_and_cycles_are_left_alone() {
        let mut c = ctx();
        c.local.insert("a".into(), "{{b}}".into());
        c.local.insert("b".into(), "{{a}}".into());
        let (out, missing) = c.resolve_report("x {{nope}} {{a}} {{ host }}");
        assert!(out.starts_with("x {{nope}} "));
        assert!(out.ends_with(" env.io"));
        assert!(out.contains("{{"));
        assert_eq!(missing[0], "nope");
    }

    #[test]
    fn disabled_variables_are_ignored() {
        let mut c = ctx();
        c.environment.as_mut().unwrap()[0].enabled = false;
        assert_eq!(c.resolve("{{host}}"), "collection.io");
    }

    #[test]
    fn dynamic_variables() {
        let c = VarContext::default();
        let guid = c.resolve("{{$guid}}");
        assert_eq!(guid.len(), 36);
        assert_ne!(guid, c.resolve("{{$guid}}"));
        assert!(c.resolve("{{$timestamp}}").parse::<i64>().unwrap() > 1_600_000_000);
        let n: u32 = c.resolve("{{$randomInt}}").parse().unwrap();
        assert!(n <= 1000);
        assert_eq!(c.resolve("{{$randomNotAThing}}"), "{{$randomNotAThing}}");
    }

    #[test]
    fn user_variable_shadows_dynamic() {
        let mut c = VarContext::default();
        c.local.insert("$guid".into(), "fixed".into());
        assert_eq!(c.resolve("{{$guid}}"), "fixed");
    }

    #[test]
    fn finds_references() {
        let refs = VarContext::references("a{{x}}b{{ y }}{{");
        assert_eq!(refs, vec![(1, 6, "x".into()), (7, 14, "y".into())]);
    }

    #[test]
    fn unterminated_braces_are_kept() {
        assert_eq!(ctx().resolve("{{host"), "{{host");
        assert_eq!(ctx().resolve("{{{{host}}"), "{{env.io");
    }
}
