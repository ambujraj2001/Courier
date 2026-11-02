//! Import and export formats, plus detection of what a pasted or dropped
//! piece of text contains.

pub mod curl;
pub mod postman;
pub mod snippet;

use crate::model::{Collection, Environment, RequestItem};
use crate::settings::Settings;
use postman::Scope;
use serde_json::{Value, json};

/// Everything Courier stores except history, in one portable file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Backup {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    pub globals: Option<Environment>,
    pub settings: Option<Settings>,
}

/// What an import produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Imported {
    Collection(Collection),
    Environment(Environment),
    Globals(Environment),
    Requests(Vec<RequestItem>),
    Backup(Backup),
}

impl Imported {
    pub fn describe(&self) -> String {
        match self {
            Imported::Collection(c) => format!("Collection “{}” with {} requests", c.name, c.request_count()),
            Imported::Environment(e) => format!("Environment “{}” with {} variables", e.name, e.variables.len()),
            Imported::Globals(g) => format!("Globals with {} variables", g.variables.len()),
            Imported::Requests(r) if r.len() == 1 => format!("Request {}", r[0].name),
            Imported::Requests(r) => format!("{} requests from cURL", r.len()),
            Imported::Backup(b) => {
                format!("Courier backup: {} collections, {} environments", b.collections.len(), b.environments.len())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportResult {
    pub items: Vec<Imported>,
    pub warnings: Vec<String>,
}

pub fn export_backup(b: &Backup) -> String {
    let v = json!({
        "courierBackup": 1,
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "collections": b.collections.iter().map(|c| postman::collection_to_value(c, false)).collect::<Vec<_>>(),
        "environments": b.environments.iter().map(|e| postman::environment_to_value(e, Scope::Environment, false, false)).collect::<Vec<_>>(),
        "globals": b.globals.as_ref().map(|g| postman::environment_to_value(g, Scope::Globals, false, false)),
        "settings": b.settings,
    });
    serde_json::to_string_pretty(&v).expect("serializable")
}

fn import_json(v: Value, result: &mut ImportResult) -> Result<(), String> {
    if v.get("courierBackup").is_some() {
        let mut b = Backup::default();
        for c in v["collections"].as_array().cloned().unwrap_or_default() {
            match postman::collection_from_value(c) {
                Ok(c) => b.collections.push(c),
                Err(e) => result.warnings.push(e.to_string()),
            }
        }
        for e in v["environments"].as_array().cloned().unwrap_or_default() {
            match postman::environment_from_value(e) {
                Ok((e, _)) => b.environments.push(e),
                Err(err) => result.warnings.push(err.to_string()),
            }
        }
        if v["globals"].is_object() {
            b.globals = postman::environment_from_value(v["globals"].clone()).ok().map(|(g, _)| g);
        }
        b.settings = serde_json::from_value(v["settings"].clone()).ok();
        result.items.push(Imported::Backup(b));
        return Ok(());
    }
    if postman::is_collection(&v) {
        result.items.push(Imported::Collection(postman::collection_from_value(v).map_err(|e| e.to_string())?));
        return Ok(());
    }
    if let Some(scope) = postman::is_environment(&v) {
        let (env, _) = postman::environment_from_value(v).map_err(|e| e.to_string())?;
        result.items.push(match scope {
            Scope::Environment => Imported::Environment(env),
            Scope::Globals => Imported::Globals(env),
        });
        return Ok(());
    }
    // Multi-entity data dumps: { collections: [], environments: [], globals: [] }
    if v.get("collections").is_some_and(Value::is_array) || v.get("environments").is_some_and(Value::is_array) {
        for c in v["collections"].as_array().cloned().unwrap_or_default() {
            match postman::collection_from_value(c) {
                Ok(c) => result.items.push(Imported::Collection(c)),
                Err(_) => result.warnings.push("skipped a collection in an older format".into()),
            }
        }
        for e in v["environments"].as_array().cloned().unwrap_or_default() {
            let mut e = e;
            if e.get("_postman_variable_scope").is_none() {
                e["_postman_variable_scope"] = "environment".into();
            }
            if let Ok((env, _)) = postman::environment_from_value(e) {
                result.items.push(Imported::Environment(env));
            }
        }
        if let Some(values) = v.get("globals").and_then(Value::as_array) {
            let g = json!({ "name": "Globals", "values": values, "_postman_variable_scope": "globals" });
            if let Ok((env, _)) = postman::environment_from_value(g) {
                result.items.push(Imported::Globals(env));
            }
        }
        if result.items.is_empty() {
            return Err("the data dump contained nothing Courier can import".into());
        }
        return Ok(());
    }
    if v.get("info").is_some() {
        return Err("unsupported collection format (only v2.0 and v2.1 can be imported)".into());
    }
    Err("unrecognised JSON: expected a collection, environment, globals or Courier backup".into())
}

/// Detect and import text from a file, a paste or a drop.
pub fn import_text(text: &str) -> Result<ImportResult, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err("nothing to import".into());
    }
    let mut result = ImportResult::default();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let v: Value = serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON: {e}"))?;
        match v {
            Value::Array(list) => {
                for item in list {
                    import_json(item, &mut result)?;
                }
            }
            v => import_json(v, &mut result)?,
        }
        return Ok(result);
    }
    let parsed = curl::parse(trimmed)?;
    result.warnings = parsed.warnings;
    result.items.push(Imported::Requests(parsed.requests));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Variable;

    #[test]
    fn detects_each_kind() {
        let c = import_text(include_str!("../../tests/fixtures/collection_v21.json")).unwrap();
        assert!(matches!(&c.items[0], Imported::Collection(c) if c.name == "Courier Fixture"));
        let e = import_text(r#"{"name":"Dev","values":[],"_postman_variable_scope":"environment"}"#).unwrap();
        assert!(matches!(e.items[0], Imported::Environment(_)));
        let g = import_text(r#"{"values":[],"_postman_variable_scope":"globals"}"#).unwrap();
        assert!(matches!(g.items[0], Imported::Globals(_)));
        let r = import_text("curl https://x.io --bogus").unwrap();
        assert!(matches!(&r.items[0], Imported::Requests(r) if r.len() == 1));
        assert_eq!(r.warnings.len(), 1);
        assert!(import_text("hello").is_err());
        assert!(import_text("{\"a\":1}").is_err());
    }

    #[test]
    fn backup_round_trip() {
        let mut env = Environment::new("Dev");
        env.variables.push(Variable::new("a", "1"));
        let b = Backup {
            collections: vec![Collection::new("One")],
            environments: vec![env],
            globals: Some(Environment::new("Globals")),
            settings: Some(Settings::default()),
        };
        let text = export_backup(&b);
        let r = import_text(&text).unwrap();
        let Imported::Backup(back) = &r.items[0] else { panic!() };
        assert_eq!(back.collections[0].name, "One");
        assert_eq!(back.environments[0].variables[0].initial, "1");
        assert!(back.globals.is_some());
        assert_eq!(back.settings, Some(Settings::default()));
    }

    #[test]
    fn data_dump_with_globals() {
        let dump = r#"{"version":1,"collections":[{"id":"x","name":"old v1","requests":[]}],
            "environments":[{"id":"e","name":"Env","values":[{"key":"k","value":"v","enabled":true}]}],
            "globals":[{"key":"g","value":"1","enabled":true}]}"#;
        let r = import_text(dump).unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.warnings.len(), 1);
    }
}
