//! # StateFS config-rs Compatibility Bridge
//!
//! Exposes a high-performance, drop-in replacement API for code expecting [`config::Config`].

use statefs_core::MemStore;
use statefs_runtime::{QueryIntent, QueryScenarioResolver};

/// High-performance StateFS wrapper matching the API ergonomics of `config::Config`.
pub struct ConfigBridge<const CAP: usize = 64> {
    resolver: QueryScenarioResolver<CAP>,
}

impl<const CAP: usize> ConfigBridge<CAP> {
    pub fn new(store: MemStore) -> Self {
        Self {
            resolver: QueryScenarioResolver::new(store),
        }
    }

    #[inline(always)]
    pub fn get_int(&mut self, key: &str) -> Result<i64, &'static str> {
        let node = self
            .resolver
            .resolve_query(QueryIntent::hot_loop(key))
            .ok_or("Key not found")?;

        node.value.as_int().ok_or("Value is not an integer")
    }

    #[inline(always)]
    pub fn get_string(&mut self, key: &str) -> Result<String, &'static str> {
        let node = self
            .resolver
            .resolve_query(QueryIntent::hot_loop(key))
            .ok_or("Key not found")?;

        node.value
            .as_str()
            .map(|s| s.to_string())
            .ok_or("Value is not a string")
    }

    #[inline(always)]
    pub fn get_bool(&mut self, key: &str) -> Result<bool, &'static str> {
        let node = self
            .resolver
            .resolve_query(QueryIntent::hot_loop(key))
            .ok_or("Key not found")?;

        node.value.as_bool().ok_or("Value is not a boolean")
    }
}
