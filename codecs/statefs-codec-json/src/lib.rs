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

/// Ingests a raw JSON string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_json_str(
    store: &mut MemStore,
    prefix: &str,
    raw_json: &str,
) -> Result<(), JsonCodecError> {
    let json_val: serde_json::Value = serde_json::from_str(raw_json)?;
    let p = Path::parse(prefix);
    flatten_json_value(store, &p, &json_val)?;
    Ok(())
}

fn flatten_json_value(
    store: &mut MemStore,
    current_path: &Path,
    val: &serde_json::Value,
) -> Result<(), StoreError> {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let sub_path = current_path.join(k);
                flatten_json_value(store, &sub_path, v)?;
            }
        }
        serde_json::Value::String(s) => {
            store.insert(current_path, Value::from(s.as_str()))?;
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                store.insert(current_path, Value::from(i))?;
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
        serde_json::Value::Array(arr) => {
            let vals: Vec<Value> = arr
                .iter()
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(Value::from(s.as_str())),
                    serde_json::Value::Number(n) => n
                        .as_i64()
                        .map(Value::from)
                        .or_else(|| n.as_f64().map(Value::from)),
                    serde_json::Value::Bool(b) => Some(Value::from(*b)),
                    _ => None,
                })
                .collect();
            store.insert(current_path, Value::Array(vals))?;
        }
    }
    Ok(())
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
                    let jval = match &node.value {
                        Value::Null => serde_json::Value::Null,
                        Value::Bool(b) => serde_json::Value::Bool(*b),
                        Value::Int(i) => serde_json::Value::Number((*i).into()),
                        Value::Float(f) => serde_json::Number::from_f64(*f)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null),
                        Value::String(s) => serde_json::Value::String(s.clone()),
                        Value::Bytes(b) => serde_json::Value::String(alloc::format!("{:?}", b)),
                        Value::Array(_) => serde_json::Value::Array(Vec::new()),
                        Value::Map(_) => serde_json::Value::Object(serde_json::Map::new()),
                    };

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
}
