//! # StateFS JSON Codec
//!
//! Encodes and decodes hierarchical state trees to and from JSON format.

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;
use statefs_core::{MemStore, Path, Store, Value};

/// Ingests a raw JSON string into a [`MemStore`] under the specified base path prefix.
pub fn ingest_json_str(
    store: &mut MemStore,
    prefix: &str,
    raw_json: &str,
) -> Result<(), serde_json::Error> {
    let json_val: serde_json::Value = serde_json::from_str(raw_json)?;
    let p = Path::parse(prefix);
    flatten_json_value(store, &p, &json_val);
    Ok(())
}

fn flatten_json_value(store: &mut MemStore, current_path: &Path, val: &serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let sub_path = current_path.join(k);
                flatten_json_value(store, &sub_path, v);
            }
        }
        serde_json::Value::String(s) => {
            let _ = store.insert(current_path, Value::from(s.as_str()));
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let _ = store.insert(current_path, Value::from(i));
            } else if let Some(f) = n.as_f64() {
                let _ = store.insert(current_path, Value::from(f));
            }
        }
        serde_json::Value::Bool(b) => {
            let _ = store.insert(current_path, Value::from(*b));
        }
        serde_json::Value::Null => {
            let _ = store.insert(current_path, Value::Null);
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
            let _ = store.insert(current_path, Value::Array(vals));
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
            for (i, seg) in segments.iter().enumerate() {
                if i == segments.len() - 1 {
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
                    current.insert(seg.to_string(), jval);
                } else {
                    current = current
                        .entry(seg.to_string())
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
                        .as_object_mut()
                        .unwrap();
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
}
