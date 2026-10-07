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
    /// If an empty string is provided, prefix filtering is disabled.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        let p = prefix.into();
        self.prefix = if p.is_empty() { None } else { Some(p) };
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
                        if !self.separator.is_empty()
                            && let Some(stripped) = rest.strip_prefix(&self.separator)
                        {
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
            let segments: Vec<String> = if self.separator.is_empty() {
                let seg = if self.lowercase {
                    trimmed_key.to_lowercase()
                } else {
                    trimmed_key.to_string()
                };
                vec![seg]
            } else {
                trimmed_key
                    .split(&self.separator)
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        if self.lowercase {
                            s.to_lowercase()
                        } else {
                            s.to_string()
                        }
                    })
                    .collect()
            };

            if segments.is_empty() {
                continue;
            }

            let path = Path::from_segments(segments);
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

    // 3. Float check: only attempt float parse if it contains '.' or scientific notation ('e'/'E').
    // This prevents overflowed integers (e.g. 64-bit uints, snowflake IDs like 9223372036854775808)
    // from being lossily converted to f64.
    if (trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E'))
        && let Ok(f) = trimmed.parse::<f64>()
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

    #[test]
    fn test_env_extreme_numbers_and_overflows() {
        let mut store = MemStore::new();
        let test_vars = vec![
            ("CFG__BIG_INT", "9223372036854775808"),    // i64::MAX + 1
            ("CFG__MAX_UINT", "18446744073709551615"),  // u64::MAX
            ("CFG__UNDERFLOW", "-9223372036854775809"), // i64::MIN - 1
            ("CFG__SCIENTIFIC", "1.5e6"),               // valid float
            ("CFG__SCIENTIFIC_INT", "2e8"),             // valid float
            ("CFG__FLOAT", "123.456"),                  // valid float
            ("CFG__NAN", "NaN"),                        // non-finite, must be String
            ("CFG__INF", "inf"),                        // non-finite, must be String
            ("CFG__NEG_INF", "-infinity"),              // non-finite, must be String
            ("CFG__BOOL_TRUE", "tRuE"),                 // bool
            ("CFG__BOOL_FALSE", "False"),               // bool
            ("CFG__EMPTY_STR", ""),                     // empty string
            ("CFG__WHITESPACE_NUM", "  100  "),         // trimmed int
        ];

        let source = EnvSource::new().with_prefix("CFG");
        source.ingest_iter(&mut store, test_vars).unwrap();

        // Extreme integers must remain Strings to prevent lossy f64 precision truncation
        assert_eq!(
            store.get_str("/big_int").and_then(|n| n.value.as_str()),
            Some("9223372036854775808")
        );
        assert_eq!(
            store.get_str("/max_uint").and_then(|n| n.value.as_str()),
            Some("18446744073709551615")
        );
        assert_eq!(
            store.get_str("/underflow").and_then(|n| n.value.as_str()),
            Some("-9223372036854775809")
        );

        // Floats with explicit '.' or 'e'/'E' parse as Float
        assert_eq!(
            store
                .get_str("/scientific")
                .and_then(|n| n.value.as_float()),
            Some(1.5e6)
        );
        assert_eq!(
            store
                .get_str("/scientific_int")
                .and_then(|n| n.value.as_float()),
            Some(2e8)
        );
        assert_eq!(
            store.get_str("/float").and_then(|n| n.value.as_float()),
            Some(123.456)
        );

        // NaN and Infinity remain strings
        assert_eq!(
            store.get_str("/nan").and_then(|n| n.value.as_str()),
            Some("NaN")
        );
        assert_eq!(
            store.get_str("/inf").and_then(|n| n.value.as_str()),
            Some("inf")
        );
        assert_eq!(
            store.get_str("/neg_inf").and_then(|n| n.value.as_str()),
            Some("-infinity")
        );

        // Booleans
        assert_eq!(
            store.get_str("/bool_true").and_then(|n| n.value.as_bool()),
            Some(true)
        );
        assert_eq!(
            store.get_str("/bool_false").and_then(|n| n.value.as_bool()),
            Some(false)
        );

        // Empty and trimmed
        assert_eq!(
            store.get_str("/empty_str").and_then(|n| n.value.as_str()),
            Some("")
        );
        assert_eq!(
            store
                .get_str("/whitespace_num")
                .and_then(|n| n.value.as_int()),
            Some(100)
        );
    }

    #[test]
    fn test_env_empty_prefix_and_weird_separators() {
        let mut store = MemStore::new();
        let test_vars = vec![
            ("FOO__BAR", "1"),
            ("SERVER__PORT", "27015"),
            ("SINGLE_KEY", "hello"),
        ];

        // Empty prefix must behave like no prefix (ingests all keys)
        let source = EnvSource::new().with_prefix("");
        let count = source.ingest_iter(&mut store, test_vars).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            store.get_str("/foo/bar").and_then(|n| n.value.as_int()),
            Some(1)
        );

        // Empty separator must treat key as single segment, never character-split
        let mut store2 = MemStore::new();
        let vars2 = vec![("SERVER_HOST", "localhost")];
        let source2 = EnvSource::new().with_separator("");
        source2.ingest_iter(&mut store2, vars2).unwrap();
        assert_eq!(
            store2
                .get_str("/server_host")
                .and_then(|n| n.value.as_str()),
            Some("localhost")
        );

        // Consecutive separators (e.g. APP____DATA)
        let mut store3 = MemStore::new();
        let vars3 = vec![("APP____DATABASE____HOST", "127.0.0.1")];
        let source3 = EnvSource::new().with_prefix("APP");
        source3.ingest_iter(&mut store3, vars3).unwrap();
        assert_eq!(
            store3
                .get_str("/database/host")
                .and_then(|n| n.value.as_str()),
            Some("127.0.0.1")
        );

        // Non-ASCII keys (Cyrillic & Unicode)
        let mut store4 = MemStore::new();
        let vars4 = vec![("НАСТРОЙКИ__ПОРТ", "8080"), ("APP__CONFIG_キー", "val")];
        let source4 = EnvSource::new();
        source4.ingest_iter(&mut store4, vars4).unwrap();
        assert_eq!(
            store4
                .get_str("/настройки/порт")
                .and_then(|n| n.value.as_int()),
            Some(8080)
        );
        assert_eq!(
            store4
                .get_str("/app/config_キー")
                .and_then(|n| n.value.as_str()),
            Some("val")
        );
    }
}
