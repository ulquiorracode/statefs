//! # StateFS Optimization Adapters (`statefs-opt`)
//!
//! A modular suite of composable optimization adapters, passport-based admission controllers,
//! and scenario resolvers for StateFS.
//!
//! ### Key Capabilities:
//! - **Adapter Passports**: Capabilities, limits, and cost declarations (`Passport`, `Visa`).
//! - **L1 Path Cache**: Compile-time bounded inline direct-mapped cache (`L1PathCache<const CAP: usize>`).
//! - **SIMD Path Scanner**: Accelerated delimiter scan via vector instructions (`SimdScanner`).
//! - **Scenario Resolvers**: Border regulators matching execution intents to optimal paths (`QueryScenarioResolver`).
//! - **Pipeline Bridge**: First-class integration with `stitch-rs` Sewing Machine Architecture.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cache;
pub mod ext;
pub mod passport;
pub mod resolver;
pub mod simd;
pub mod stitch;

pub use cache::{DEFAULT_CACHE_CAP, L1PathCache};
pub use ext::StoreOptExt;
pub use passport::{AdapterContract, Passport, Visa, WorkloadScenario};
pub use resolver::{QueryIntent, QueryScenarioResolver, ResolverMetrics};
pub use simd::SimdScanner;
pub use stitch::{StateFsContext, StateFsTerminal};

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::{MemStore, Path, Store, Value};

    #[test]
    fn test_adapter_passport_and_resolver_flow() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();
        store
            .insert(&Path::parse("/server/motd"), Value::from("Welcome!"))
            .unwrap();

        let mut resolver = store.with_l1_cache::<32>();

        // 1. Hot loop scenario: First query is a cache miss, populates cache
        let node1 = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("node must exist");
        assert_eq!(node1.value.as_int(), Some(128));
        assert_eq!(resolver.metrics.cache_misses, 1);
        assert_eq!(resolver.metrics.cache_hits, 0);

        // 2. Hot loop scenario: Second query is a direct L1 cache hit!
        let node2 = resolver
            .resolve_query(QueryIntent::hot_loop("/server/tickrate"))
            .expect("node must exist");
        assert_eq!(node2.value.as_int(), Some(128));
        assert_eq!(resolver.metrics.cache_hits, 1);

        // 3. BootLoading scenario: Passport rejects caching to prevent cache pollution
        let _ = resolver.resolve_query(QueryIntent::boot_load("/server/motd"));
        assert_eq!(resolver.metrics.cache_bypasses, 1);
    }
}
