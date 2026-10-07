//! # StateFS Environment Variable Ingestion Bridge
//!
//! Bridges system environment variables directly into a StateFS hierarchical store.
//!
//! Supports customizable key prefixes, segment separators, case normalization,
//! and automatic scalar type inference (`bool`, `i64`, `f64`, `String`).

use statefs_core::{Path, StateSource, Store, StoreError, Value};
use std::env;

/// Configuration builder and executor for environment variable ingestion.
#[derive(Debug, Clone)]
pub struct EnvSource {
    prefix: Option<String>,
    separator: String,
    lowercase: bool,
    trim_prefix_separator: bool,
}

impl Default for EnvSource {
    fn default() -> Self {
        Self {
            prefix: None,
            separator: "__".to_string(),
            lowercase: true,
            trim_prefix_separator: true,
        }
    }
}

impl EnvSource {
    /// Creates a new `EnvSource` with default settings (`separator = "__"`, `lowercase = true`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an optional environment prefix filter (e.g. `"APP"` or `"SERVER"`).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Sets the segment separator string (default: `"__"`).
    pub fn with_separator(mut self, sep: impl Into<String>) -> Self {
        self.separator = sep.into();
        self
    }

    /// Sets whether path segments are lowercased (default: `true`).
    pub fn with_lowercase(mut self, lowercase: bool) -> Self {
        self.lowercase = lowercase;
        self
    }

    /// Ingests from an arbitrary iterator of `(key, value)` pairs into the store.
    pub fn ingest_iter<I, K, V, S>(&self, store: &mut S, vars: I) -> Result<usize, StoreError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
        S: Store,
    {
        let mut count = 0;
        for (raw_key, raw_val) in vars {
            let key = raw_key.as_ref();
            let val = raw_val.as_ref();

            let trimmed_key = match &self.prefix {
                Some(pfx) => {
                    if !key.starts_with(pfx) {
                        continue;
                    }
                    let rest = &key[pfx.len()..];
                    if self.trim_prefix_separator {
                        if let Some(stripped) = rest.strip_prefix(&self.separator) {
                            stripped
                        } else if let Some(stripped) = rest.strip_prefix('_') {
                            stripped
                        } else {
                            rest
                        }
                    } else {
                        rest
                    }
                }
                None => key,
            };

            if trimmed_key.is_empty() {
                continue;
            }

            // Split into path segments
            let segments: Vec<String> = trimmed_key
                .split(&self.separator)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    if self.lowercase {
                        s.to_lowercase()
                    } else {
                        s.to_string()
                    }
                })
                .collect();

            if segments.is_empty() {
                continue;
            }

            let path_str = format!("/{}", segments.join("/"));
            let path = Path::parse(&path_str);
            let parsed_value = parse_scalar_value(val);

            store.insert(&path, parsed_value)?;
            count += 1;
        }

        Ok(count)
    }

    /// Ingests system environment variables (`std::env::vars()`) into the store.
    pub fn ingest<S: Store>(&self, store: &mut S) -> Result<usize, StoreError> {
        self.ingest_iter(store, env::vars())
    }
}

impl StateSource for EnvSource {
    fn apply_to_store<S: Store>(&self, store: &mut S) -> Result<(), StoreError> {
        self.ingest(store)?;
        Ok(())
    }
}

/// Convenience function to ingest environment variables with standard conventions in one call.
pub fn ingest_env<S: Store>(
    store: &mut S,
    prefix: Option<&str>,
    separator: &str,
) -> Result<usize, StoreError> {
    let mut source = EnvSource::new().with_separator(separator);
    if let Some(p) = prefix {
        source = source.with_prefix(p);
    }
    source.ingest(store)
}

/// Parses a string representation into a strongly-typed [`Value`].
fn parse_scalar_value(s: &str) -> Value {
    let trimmed = s.trim();

    // 1. Boolean check
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }

    // 2. Integer check
    if let Ok(i) = trimmed.parse::<i64>() {
        return Value::Int(i);
    }

    // 3. Float check
    if let Ok(f) = trimmed.parse::<f64>()
        && f.is_finite()
    {
        return Value::Float(f);
    }

    // 4. Fallback to String
    Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::MemStore;

    #[test]
    fn test_env_source_ingestion() {
        let mut store = MemStore::new();
        let test_vars = vec![
            ("APP__SERVER__PORT", "8080"),
            ("APP__SERVER__HOST", "127.0.0.1"),
            ("APP__METRICS__ENABLED", "true"),
            ("APP__CACHE__RATIO", "0.75"),
            ("OTHER__IGNORED", "true"),
        ];

        let source = EnvSource::new().with_prefix("APP");
        let count = source.ingest_iter(&mut store, test_vars).unwrap();

        assert_eq!(count, 4);

        assert_eq!(
            store.get_str("/server/port").and_then(|n| n.value.as_int()),
            Some(8080)
        );
        assert_eq!(
            store.get_str("/server/host").and_then(|n| n.value.as_str()),
            Some("127.0.0.1")
        );
        assert_eq!(
            store
                .get_str("/metrics/enabled")
                .and_then(|n| n.value.as_bool()),
            Some(true)
        );
        assert_eq!(
            store
                .get_str("/cache/ratio")
                .and_then(|n| n.value.as_float()),
            Some(0.75)
        );
        assert!(store.get_str("/ignored").is_none());
    }

    #[test]
    fn test_single_underscore_fallback() {
        let mut store = MemStore::new();
        let test_vars = vec![("CFG_DATABASE__POOL_SIZE", "16")];

        let source = EnvSource::new().with_prefix("CFG");
        source.ingest_iter(&mut store, test_vars).unwrap();

        assert_eq!(
            store
                .get_str("/database/pool_size")
                .and_then(|n| n.value.as_int()),
            Some(16)
        );
    }
}
