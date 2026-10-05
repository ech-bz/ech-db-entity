use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use board_model::Schemas;
use ech_db_entity::Schema;
use serde_json::{json, Value};

use crate::cases::Cases;

pub struct FrozenCase {
    pub name: String,
    pub schema: String,
    pub bytes: Vec<u8>,
}

pub struct Corpus;

impl Corpus {
    pub fn verify(dir: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        for (kind, name, text) in Self::entries() {
            let relative = format!("{kind}/{name}.json");
            match std::fs::read_to_string(dir.join(&relative)) {
                Err(_) => problems.push(format!("missing {relative}")),
                Ok(frozen) => {
                    if frozen != text {
                        problems.push(format!("changed {relative}\nfrozen:\n{frozen}\ncurrent:\n{text}"));
                    }
                }
            }
        }
        problems.extend(Self::unexpected(dir));
        problems
    }

    pub fn append(dir: &Path) -> Result<Vec<String>, Vec<String>> {
        let mut problems = Vec::new();
        let mut pending = Vec::new();
        for (kind, name, text) in Self::entries() {
            let relative = format!("{kind}/{name}.json");
            match std::fs::read_to_string(dir.join(&relative)) {
                Err(_) => pending.push((relative, text)),
                Ok(frozen) => {
                    if frozen != text {
                        problems.push(format!("changed {relative}\nfrozen:\n{frozen}\ncurrent:\n{text}"));
                    }
                }
            }
        }
        problems.extend(Self::unexpected(dir));
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut written = Vec::new();
        for (relative, text) in pending {
            let path = dir.join(&relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            written.push(relative);
        }
        Ok(written)
    }

    fn unexpected(dir: &Path) -> Vec<String> {
        let mut known = BTreeMap::new();
        for (kind, name, _) in Self::entries() {
            known.insert((kind, name), ());
        }
        Self::stored(dir)
            .into_iter()
            .filter(|(kind, name)| !known.contains_key(&(kind.clone(), name.clone())))
            .map(|(kind, name)| format!("unexpected {kind}/{name}.json (no current counterpart)"))
            .collect()
    }

    pub fn frozen_cases(dir: &Path) -> Vec<FrozenCase> {
        let mut names: Vec<String> = std::fs::read_dir(dir.join("bytes"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
            .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|name| Self::load_case(dir, &name))
            .collect()
    }

    pub fn frozen_case(dir: &Path, name: &str) -> FrozenCase {
        Self::load_case(dir, name)
    }

    fn load_case(dir: &Path, name: &str) -> FrozenCase {
        let path = dir.join("bytes").join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let value: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("invalid {}: {error}", path.display()));
        let schema = value
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{} has no schema", path.display()))
            .to_string();
        let hex = value
            .get("hex")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{} has no hex", path.display()));
        FrozenCase {
            name: name.to_string(),
            schema,
            bytes: hex::decode(hex).unwrap_or_else(|error| panic!("bad hex in {}: {error}", path.display())),
        }
    }

    fn entries() -> Vec<(String, String, String)> {
        let mut entries = Vec::new();
        for (name, text) in Self::schema_nodes() {
            entries.push(("schemas".to_string(), name, text));
        }
        for (name, text) in Self::byte_cases() {
            entries.push(("bytes".to_string(), name, text));
        }
        entries
    }

    fn stored(dir: &Path) -> Vec<(String, String)> {
        let mut stored = Vec::new();
        for kind in ["schemas", "bytes"] {
            let Ok(read) = std::fs::read_dir(dir.join(kind)) else {
                continue;
            };
            for entry in read {
                let path: PathBuf = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "json") {
                    stored.push((
                        kind.to_string(),
                        path.file_stem().unwrap().to_string_lossy().into_owned(),
                    ));
                }
            }
        }
        stored.sort();
        stored
    }

    fn schema_nodes() -> Vec<(String, String)> {
        let mut nodes = Vec::new();
        let mut seen = BTreeMap::new();
        for entity in Schemas::entities() {
            for version in &entity.versions {
                let key = format!("{}@{}", entity.name, version.version);
                let value = json!({
                    "entity": entity.name,
                    "key": entity.key,
                    "version": version.version,
                    "migration": version.migration,
                    "fields": serde_json::to_value(&version.fields).unwrap(),
                });
                Self::push(&mut nodes, &mut seen, key, value);
            }
        }
        for schema in Schemas::events() {
            let key = match &schema {
                Schema::Struct { name, .. }
                | Schema::Enum { name, .. }
                | Schema::Newtype { name, .. } => name.clone(),
                _ => panic!("event schema without a name"),
            };
            Self::push(&mut nodes, &mut seen, key, serde_json::to_value(&schema).unwrap());
        }
        nodes
    }

    fn byte_cases() -> Vec<(String, String)> {
        Cases::all()
            .into_iter()
            .map(|case| {
                let value = json!({
                    "name": case.name,
                    "schema": case.schema,
                    "hex": case.hex,
                });
                (case.name.to_string(), Self::text(&value))
            })
            .collect()
    }

    fn push(
        nodes: &mut Vec<(String, String)>,
        seen: &mut BTreeMap<String, ()>,
        key: String,
        value: Value,
    ) {
        if seen.insert(key.clone(), ()).is_some() {
            panic!("duplicate schema node {key}");
        }
        nodes.push((key, Self::text(&value)));
    }

    fn text(value: &Value) -> String {
        serde_json::to_string_pretty(value).unwrap() + "\n"
    }
}
