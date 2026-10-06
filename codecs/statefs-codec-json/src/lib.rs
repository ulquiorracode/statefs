//! # StateFS JSON Codec
//!
//! Encodes and decodes hierarchical state trees to and from JSON format.

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;
use statefs_core::{MemStore, Path, Store, StoreError, Value};

/// Errors occurring during JSON ingestion or parsing.
#[derive(Debug)]
pub enum JsonCodecError {
    Serde(serde_json::Error),
    Store(StoreError),
}

impl core::fmt::Display for JsonCodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Serde(e) => write!(f, "JSON serde error: {e}"),
            Self::Store(e) => write!(f, "StateFS store error: {e}"),
        }
    }
}

impl std::error::Error for JsonCodecError {}

impl From<serde_json::Error> for JsonCodecError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(e)
    }
}

impl From<StoreError> for JsonCodecError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Maximum recursion depth allowed during JSON traversal to prevent stack exhaustion.
const MAX_JSON_DEPTH: usize = 32;

/// Ingests a raw JSON string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_json_str(
    store: &mut MemStore,
    prefix: &str,
    raw_json: &str,
) -> Result<(), JsonCodecError> {
    let json_val: serde_json::Value = serde_json::from_str(raw_json)?;
    let p = Path::parse(prefix);
    flatten_json_value(store, &p, &json_val, 0)?;
    Ok(())
}

fn json_to_value(val: &serde_json::Value, depth: usize) -> Option<Value> {
    if depth > MAX_JSON_DEPTH {
        return None;
    }
    match val {
        serde_json::Value::Null => Some(Value::Null),
        serde_json::Value::Bool(b) => Some(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Value::Int(i))
            } else if let Some(u) = n.as_u64() {
                if let Ok(i) = i64::try_from(u) {
                    Some(Value::Int(i))
                } else {
                    n.as_f64().map(Value::Float)
                }
            } else {
                n.as_f64().map(Value::Float)
            }
        }
        serde_json::Value::String(s) => Some(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for item in arr {
                if let Some(v) = json_to_value(item, depth + 1) {
                    items.push(v);
                }
            }
            Some(Value::Array(items))
        }
        serde_json::Value::Object(map) => {
            let mut btree = alloc::collections::BTreeMap::new();
            for (k, v) in map {
                if let Some(val) = json_to_value(v, depth + 1) {
                    btree.insert(k.clone(), val);
                }
            }
            Some(Value::Map(btree))
        }
    }
}

fn flatten_json_value(
    store: &mut MemStore,
    current_path: &Path,
    val: &serde_json::Value,
    depth: usize,
) -> Result<(), StoreError> {
    if depth > MAX_JSON_DEPTH {
        return Ok(());
    }

    match val {
        serde_json::Value::Object(map) => {
            // If this object represents a branch node with a coexisting scalar value:
            if let Some(scalar_json) = map.get("_value")
                && let Some(scalar_val) = json_to_value(scalar_json, depth + 1)
            {
                store.insert(current_path, scalar_val)?;
            }
            for (k, v) in map {
                if k == "_value" {
                    continue;
                }
                let sub_path = current_path.join(k);
                flatten_json_value(store, &sub_path, v, depth + 1)?;
            }
        }
        serde_json::Value::String(s) => {
            store.insert(current_path, Value::from(s.as_str()))?;
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                store.insert(current_path, Value::from(i))?;
            } else if let Some(u) = n.as_u64() {
                if let Ok(i) = i64::try_from(u) {
                    store.insert(current_path, Value::from(i))?;
                } else if let Some(f) = n.as_f64() {
                    store.insert(current_path, Value::from(f))?;
                }
            } else if let Some(f) = n.as_f64() {
                store.insert(current_path, Value::from(f))?;
            }
        }
        serde_json::Value::Bool(b) => {
            store.insert(current_path, Value::from(*b))?;
        }
        serde_json::Value::Null => {
            store.insert(current_path, Value::Null)?;
        }
        serde_json::Value::Array(_) => {
            if let Some(v) = json_to_value(val, depth) {
                store.insert(current_path, v)?;
            }
        }
    }
    Ok(())
}

fn statefs_val_to_json(val: &Value, depth: usize) -> serde_json::Value {
    if depth > MAX_JSON_DEPTH {
        return serde_json::Value::Null;
    }
    match val {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Bytes(b) => serde_json::Value::Array(
            b.iter()
                .map(|&x| serde_json::Value::Number(x.into()))
                .collect(),
        ),
        Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|item| statefs_val_to_json(item, depth + 1))
                .collect(),
        ),
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                obj.insert(k.clone(), statefs_val_to_json(v, depth + 1));
            }
            serde_json::Value::Object(obj)
        }
    }
}

/// Exports a subtree under `prefix` into a hierarchical `serde_json::Value`.
pub fn export_json_value(store: &MemStore, prefix: &str) -> serde_json::Value {
    let p = Path::parse(prefix);
    let subpaths = store.list_subpaths(&p);

    let mut root_map = serde_json::Map::new();

    for path in subpaths {
        if let Some(node) = store.get(&path) {
            let relative = path.strip_prefix(&p).unwrap_or(path);
            let segments = relative.segments();
            if segments.is_empty() {
                continue;
            }

            let mut current = &mut root_map;
            let last_idx = segments.len() - 1;

            for (i, seg) in segments.iter().enumerate() {
                if i == last_idx {
                    let jval = statefs_val_to_json(&node.value, 0);

                    // If an object already exists at this key (due to branch children), preserve branch and attach scalar
                    if let Some(existing_obj) = current.get_mut(seg).and_then(|v| v.as_object_mut())
                    {
                        existing_obj.insert("_value".to_string(), jval);
                    } else {
                        current.insert(seg.to_string(), jval);
                    }
                } else {
                    let entry = current
                        .entry(seg.to_string())
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

                    // If existing entry was a scalar value, convert it to an object containing `_value`
                    if !entry.is_object() {
                        let old_val = core::mem::replace(
                            entry,
                            serde_json::Value::Object(serde_json::Map::new()),
                        );
                        if let Some(obj) = entry.as_object_mut() {
                            obj.insert("_value".to_string(), old_val);
                        }
                    }

                    if let Some(obj) = entry.as_object_mut() {
                        current = obj;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    serde_json::Value::Object(root_map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ingest_json_roundtrip() {
        let mut store = MemStore::new();
        let json_text = r#"
        {
            "server": {
                "tickrate": 128,
                "motd": "Welcome to GoldSrc!",
                "enabled": true
            }
        }
        "#;

        ingest_json_str(&mut store, "core", json_text).unwrap();

        let tickrate = store.get(&Path::parse("/core/server/tickrate")).unwrap();
        assert_eq!(tickrate.value, Value::from(128));

        let motd = store.get(&Path::parse("/core/server/motd")).unwrap();
        assert_eq!(motd.value, Value::from("Welcome to GoldSrc!"));

        let exported = export_json_value(&store, "core");
        assert_eq!(exported["server"]["tickrate"], 128);
        assert_eq!(exported["server"]["enabled"], true);
    }

    #[test]
    fn test_export_branch_and_scalar_coexistence() {
        let mut store = MemStore::new();
        // Insert both a scalar at /a and a branch child at /a/b
        store
            .insert(&Path::parse("/test/a"), Value::from(42))
            .unwrap();
        store
            .insert(&Path::parse("/test/a/b"), Value::from(100))
            .unwrap();

        // Must not panic on export!
        let exported = export_json_value(&store, "test");
        assert_eq!(exported["a"]["b"], 100);
        assert_eq!(exported["a"]["_value"], 42);
    }

    #[test]
    fn test_json_array_and_map_roundtrip_preserved() {
        let mut store = MemStore::new();
        let json_text = r#"
        {
            "tags": ["fps", "multiplayer", 128],
            "nested": {
                "arr": [1, 2, 3]
            }
        }
        "#;
        ingest_json_str(&mut store, "config", json_text).unwrap();

        let exported = export_json_value(&store, "config");
        assert_eq!(exported["tags"][0], "fps");
        assert_eq!(exported["tags"][1], "multiplayer");
        assert_eq!(exported["tags"][2], 128);
        assert_eq!(exported["nested"]["arr"][1], 2);
    }

    #[test]
    fn test_branch_scalar_coexistence_roundtrip() {
        let mut store = MemStore::new();
        let json_text = r#"
        {
            "server": {
                "_value": "legacy_name",
                "tickrate": 128
            }
        }
        "#;
        ingest_json_str(&mut store, "test", json_text).unwrap();
        assert_eq!(
            store.get(&Path::parse("/test/server")).unwrap().value,
            Value::from("legacy_name")
        );
        assert_eq!(
            store
                .get(&Path::parse("/test/server/tickrate"))
                .unwrap()
                .value,
            Value::from(128)
        );

        let exported = export_json_value(&store, "test");
        assert_eq!(exported["server"]["_value"], "legacy_name");
        assert_eq!(exported["server"]["tickrate"], 128);
    }
}
