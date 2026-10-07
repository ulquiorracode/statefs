//! # StateFS Runtime & Scenario Orchestrator
//!
//! Orchestrates access to `statefs-core` through `stitch-rs` U-Cycle pipelines
//! and scenario resolvers without dynamic allocation.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod builder;

pub use builder::{StateFs, StateFsBuilder, StateFsError};
pub use statefs_adapter_opt_cache::{DEFAULT_CACHE_CAP, L1PathCache};
pub use statefs_adapter_opt_simd::SimdPathScanner;
pub use statefs_core::{MemStore, Node};
pub use stitch_rs::middleware::TerminalHandler;

/// Execution workload scenario under which a query operates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkloadScenario {
    /// Cold startup or mass batch write (bypasses caches to avoid pollution).
    BootLoading,

    /// High-frequency steady-state frame loop (99.9% read-heavy, hot path).
    #[default]
    SteadyStateLoop,

    /// Ad-hoc analytical or administrative query (unpredictable, wide paths).
    AdHocQuery,
}

/// Intent entering the query scenario resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryIntent<'a> {
    pub path: &'a str,
    pub scenario: WorkloadScenario,
}

impl<'a> QueryIntent<'a> {
    pub const fn new(path: &'a str, scenario: WorkloadScenario) -> Self {
        Self { path, scenario }
    }

    pub const fn hot_loop(path: &'a str) -> Self {
        Self {
            path,
            scenario: WorkloadScenario::SteadyStateLoop,
        }
    }

    pub const fn boot_load(path: &'a str) -> Self {
        Self {
            path,
            scenario: WorkloadScenario::BootLoading,
        }
    }
}

/// First-level Scenario Resolver coordinating MemStore, L1 Cache, and SIMD scanner.
pub struct QueryScenarioResolver<const CAP: usize = DEFAULT_CACHE_CAP> {
    pub store: MemStore,
    pub cache: L1PathCache<CAP>,
    pub scanner: SimdPathScanner,
    pub metrics: ResolverMetrics,
}

/// Telemetry metrics for resolver inspections.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolverMetrics {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_bypasses: u64,
    pub total_queries: u64,
}

impl<const CAP: usize> QueryScenarioResolver<CAP> {
    pub fn new(store: MemStore) -> Self {
        Self {
            store,
            cache: L1PathCache::new(),
            scanner: SimdPathScanner::new(),
            metrics: ResolverMetrics::default(),
        }
    }

    #[inline(always)]
    pub fn resolve_query(&mut self, intent: QueryIntent) -> Option<&Node> {
        self.metrics.total_queries = self.metrics.total_queries.saturating_add(1);

        if intent.scenario == WorkloadScenario::SteadyStateLoop {
            let current_epoch = self.store.subtree_revision_str(intent.path).unwrap_or(0);
            if let Some(cached_node_id) = self.cache.get_with_epoch(intent.path, current_epoch)
                && let Some(node) = self.store.get_by_id(cached_node_id)
            {
                self.metrics.cache_hits = self.metrics.cache_hits.saturating_add(1);
                return Some(node);
            }

            self.metrics.cache_misses = self.metrics.cache_misses.saturating_add(1);

            if let Some(node_id) = self.store.find_node_id(intent.path) {
                self.cache
                    .put_with_epoch(intent.path, node_id, current_epoch);
                return self.store.get_by_id(node_id);
            }
            None
        } else {
            self.metrics.cache_bypasses = self.metrics.cache_bypasses.saturating_add(1);
            self.store.get_str(intent.path)
        }
    }
}

/// Execution context for stitch-rs U-cycle dispatches.
#[derive(Debug, Default)]
pub struct StateFsContext {
    pub dispatches: u64,
}

/// Terminal handler executing the query at the bottom of the stitch-rs U-cycle.
pub struct StateFsTerminal<const CAP: usize = DEFAULT_CACHE_CAP> {
    pub resolver: QueryScenarioResolver<CAP>,
}

impl<const CAP: usize> StateFsTerminal<CAP> {
    pub fn new(resolver: QueryScenarioResolver<CAP>) -> Self {
        Self { resolver }
    }
}

impl<'a, const CAP: usize>
    TerminalHandler<StateFsContext, QueryIntent<'a>, Option<Node>, &'static str>
    for StateFsTerminal<CAP>
{
    #[inline(always)]
    fn execute(
        &mut self,
        ctx: &mut StateFsContext,
        intent: QueryIntent<'a>,
    ) -> Result<Option<Node>, &'static str> {
        ctx.dispatches = ctx.dispatches.saturating_add(1);
        let node = self.resolver.resolve_query(intent).cloned();
        Ok(node)
    }
}

/// Extension trait extending [`MemStore`] with fluent runtime builders.
pub trait RuntimeExt: Sized {
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP>;
    fn with_default_l1_cache(self) -> QueryScenarioResolver<DEFAULT_CACHE_CAP> {
        self.with_l1_cache::<DEFAULT_CACHE_CAP>()
    }
}

impl RuntimeExt for MemStore {
    #[inline(always)]
    fn with_l1_cache<const CAP: usize>(self) -> QueryScenarioResolver<CAP> {
        QueryScenarioResolver::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::{Path, Store, Value};

    #[test]
    fn test_resolver_cache_epoch_invalidation_on_mutation() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::Int(128))
            .expect("insert tickrate");

        let mut resolver = store.with_l1_cache::<64>();

        // 1. Initial lookup populates cache
        let node1 = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("initial lookup");
        assert_eq!(node1.value.as_int(), Some(128));
        assert_eq!(resolver.metrics.cache_misses, 1);
        assert_eq!(resolver.metrics.cache_hits, 0);

        // 2. Second lookup hits L1 cache with matching epoch
        let node2 = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("cached lookup");
        assert_eq!(node2.value.as_int(), Some(128));
        assert_eq!(resolver.metrics.cache_hits, 1);

        // 3. Mutate store: update tickrate advances global_revision
        resolver
            .store
            .insert(&Path::parse("/server/tickrate"), Value::Int(64))
            .expect("update tickrate");

        // 4. Third lookup detects stale epoch, invalidates slot and fetches fresh node
        let node3 = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("updated lookup");
        assert_eq!(node3.value.as_int(), Some(64));
        assert_eq!(resolver.metrics.cache_misses, 2);

        // 5. Remove node from store
        resolver
            .store
            .remove(&Path::parse("/server/tickrate"))
            .expect("remove tickrate");

        // 6. Fourth lookup must safely return None rather than stale-reading or panicking
        let node4 = resolver.resolve_query(QueryIntent::hot_loop("/server/tickrate"));
        assert!(node4.is_none());
    }

    #[test]
    fn test_resolver_thundering_miss_prevention() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::Int(128))
            .expect("insert tickrate");

        let mut resolver = store.with_l1_cache::<64>();

        // Pre-warm cache for /server/tickrate
        let _ = resolver.resolve_query(QueryIntent::hot_loop("/server/tickrate"));
        assert_eq!(resolver.metrics.cache_misses, 1);
        assert_eq!(resolver.metrics.cache_hits, 0);

        // Mutate an unrelated subtree: /plugins/moderation/ban_time
        resolver
            .store
            .insert(
                &Path::parse("/plugins/moderation/ban_time"),
                Value::Int(300),
            )
            .expect("insert moderation");

        // Querying /server/tickrate MUST NOT suffer a thundering miss!
        let node = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("tickrate lookup");
        assert_eq!(node.value.as_int(), Some(128));
        assert_eq!(resolver.metrics.cache_misses, 1); // Miss count did NOT increase!
        assert_eq!(resolver.metrics.cache_hits, 1); // Hit count increased!
    }

    #[test]
    #[cfg(all(feature = "toml", feature = "json"))]
    fn test_statefs_builder_composition() {
        let store = StateFs::builder()
            .with_toml_str(
                r#"
                [server]
                tickrate = 128
                name = "Competitive HLDS"
            "#,
            )
            .expect("toml parse")
            .with_json_str(
                r#"
                {
                    "network": {
                        "port": 27015
                    }
                }
            "#,
            )
            .expect("json parse")
            .with_value("/custom/override", 42)
            .expect("custom value")
            .build()
            .expect("build store");

        assert_eq!(
            store
                .get_str("/server/tickrate")
                .and_then(|n| n.value.as_int()),
            Some(128)
        );
        assert_eq!(
            store.get_str("/server/name").and_then(|n| n.value.as_str()),
            Some("Competitive HLDS")
        );
        assert_eq!(
            store
                .get_str("/network/port")
                .and_then(|n| n.value.as_int()),
            Some(27015)
        );
        assert_eq!(
            store
                .get_str("/custom/override")
                .and_then(|n| n.value.as_int()),
            Some(42)
        );
    }

    #[test]
    #[cfg(all(feature = "snapshot", feature = "toml"))]
    fn test_builder_layering_and_snapshot_merge() {
        // Prepare a binary snapshot
        let mut snap_store = StateFs::new_store();
        snap_store
            .insert(
                &statefs_core::Path::parse("/server/name"),
                statefs_core::Value::String("Snapshot Server".into()),
            )
            .unwrap();
        snap_store
            .insert(
                &statefs_core::Path::parse("/server/max_players"),
                statefs_core::Value::Int(32),
            )
            .unwrap();
        let snap_bytes = statefs_codec_bin::export_snapshot_bytes(&snap_store);

        // Build layered state:
        // 1. Pre-existing value in builder
        // 2. Snapshot merge
        // 3. TOML override of snapshot key
        // 4. with_value final override
        let store = StateFs::builder()
            .with_value("/pre_existing/item", "preserved")
            .unwrap()
            .with_snapshot_bytes(&snap_bytes)
            .unwrap()
            .with_toml_str(
                r#"
                [server]
                name = "TOML Override Server"
                "#,
            )
            .unwrap()
            .with_value("/server/max_players", 64)
            .unwrap()
            .build()
            .unwrap();

        // 1. Pre-existing value was preserved across snapshot merge
        assert_eq!(
            store
                .get_str("/pre_existing/item")
                .and_then(|n| n.value.as_str()),
            Some("preserved")
        );
        // 2. TOML overrode snapshot name
        assert_eq!(
            store.get_str("/server/name").and_then(|n| n.value.as_str()),
            Some("TOML Override Server")
        );
        // 3. Manual with_value overrode max_players
        assert_eq!(
            store
                .get_str("/server/max_players")
                .and_then(|n| n.value.as_int()),
            Some(64)
        );
    }

    #[test]
    fn test_builder_conflicting_readonly_overrides() {
        // Readonly node insertion
        let builder = StateFs::builder()
            .with_readonly_value("/protected/setting", "locked")
            .unwrap();

        // Attempting to overwrite a read-only setting must yield StoreError::ReadOnly
        let res = builder.with_value("/protected/setting", "tampered");
        assert!(res.is_err());
        match res.unwrap_err() {
            StateFsError::Store(statefs_core::StoreError::ReadOnly(path)) => {
                assert_eq!(path.to_string(), "/protected/setting");
            }
            other => panic!("Expected StoreError::ReadOnly, got: {:?}", other),
        }
    }

    #[test]
    #[cfg(all(feature = "toml", feature = "json", feature = "std"))]
    fn test_builder_error_propagation_on_invalid_sources() {
        // Corrupt TOML
        let toml_err = StateFs::builder().with_toml_str("invalid = [toml unclosed");
        assert!(toml_err.is_err());
        assert!(matches!(toml_err.unwrap_err(), StateFsError::Toml(_)));

        // Corrupt JSON
        let json_err = StateFs::builder().with_json_str("{\"unclosed\": ");
        assert!(json_err.is_err());
        assert!(matches!(json_err.unwrap_err(), StateFsError::Json(_)));

        // Non-existent file
        let io_err = StateFs::builder().with_toml_file("C:/non_existent_statefs_file.toml");
        assert!(io_err.is_err());
        assert!(matches!(io_err.unwrap_err(), StateFsError::Io(_)));
    }

    #[test]
    #[cfg(all(feature = "toml", feature = "serde"))]
    fn test_builder_serde_extract() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct ServerCfg {
            tickrate: u32,
            hostname: String,
        }

        let builder = StateFs::builder()
            .with_toml_str(
                r#"
                [server]
                tickrate = 128
                hostname = "De_Dust2 HLDS"
            "#,
            )
            .unwrap();

        let cfg: ServerCfg = builder.extract("/server").expect("extract ServerCfg");
        assert_eq!(
            cfg,
            ServerCfg {
                tickrate: 128,
                hostname: "De_Dust2 HLDS".to_string(),
            }
        );
    }
}
