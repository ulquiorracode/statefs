//! # Unified Matrix Benchmark (Multi-Candidate & Multi-Load Grid)
//!
//! Evaluates:
//! - Candidates:
//!   1. `config-rs` (Industry Standard Baseline)
//!   2. `StateFS (No-Op Baseline / Arena Trie Only)`
//!   3. `StateFS (+ SIMD Segment Scanner)`
//!   4. `StateFS (+ SIMD + L1 Path Cache)`
//!   5. `StateFS (Full Stack via Scenario Resolver + stitch-rs Pipeline)`
//!
//! - Metrics:
//!   - Latency per operation (ns)
//!   - Throughput (ops/sec)
//!   - Resident Heap Memory (bytes / KB via Cap tracking)
//!   - Total Memory Allocations Count
//!
//! - Load Tiers:
//!   - 10 ops (Cold Start / Light)
//!   - 1,000 ops (Typical Frame / Medium)
//!   - 50,000 ops (High-Frequency Game Loop / Heavy Burst)

use cap::Cap;
use statefs_core::{MemStore, Path, Store, Value};
use statefs_opt::{
    QueryIntent, QueryScenarioResolver, StateFsContext, StateFsTerminal, StoreOptExt,
};
use std::alloc;
use std::hint::black_box;
use std::time::Instant;
use stitch_rs::pipeline::Pipeline;

#[global_allocator]
static ALLOCATOR: Cap<alloc::System> = Cap::new(alloc::System, usize::MAX);

const FIXTURE_GOLDSRC_TOML: &str = include_str!("../../statefs-core/benches/fixtures/goldsrc.toml");
const FIXTURE_PLUGINS_TOML: &str = include_str!("../../statefs-core/benches/fixtures/plugins.toml");
const FIXTURE_COMMON_LANG: &str = include_str!("../../statefs-core/benches/fixtures/common.toml");
const FIXTURE_MODERATION_LANG: &str =
    include_str!("../../statefs-core/benches/fixtures/moderation.toml");
const FIXTURE_MODERATION_BUNDLE: &str =
    include_str!("../../statefs-core/benches/fixtures/bundle.toml");

fn populate_statefs(store: &mut MemStore) {
    for (prefix, raw) in [
        ("core", FIXTURE_GOLDSRC_TOML),
        ("plugins", FIXTURE_PLUGINS_TOML),
        ("lang/common", FIXTURE_COMMON_LANG),
        ("lang/moderation", FIXTURE_MODERATION_LANG),
        ("bundles/moderation", FIXTURE_MODERATION_BUNDLE),
    ] {
        let toml_val: toml::Value = toml::from_str(raw).unwrap();
        let p = Path::parse(prefix);
        flatten_toml_to_statefs(store, &p, &toml_val);
    }
}

fn flatten_toml_to_statefs(store: &mut MemStore, current_path: &Path, val: &toml::Value) {
    match val {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let sub_path = current_path.join(k);
                flatten_toml_to_statefs(store, &sub_path, v);
            }
        }
        toml::Value::String(s) => {
            store.insert(current_path, Value::from(s.as_str())).unwrap();
        }
        toml::Value::Integer(i) => {
            store.insert(current_path, Value::from(*i)).unwrap();
        }
        toml::Value::Float(f) => {
            store.insert(current_path, Value::from(*f)).unwrap();
        }
        toml::Value::Boolean(b) => {
            store.insert(current_path, Value::from(*b)).unwrap();
        }
        _ => {}
    }
}

fn build_config_rs() -> config::Config {
    config::Config::builder()
        .add_source(config::File::from_str(
            FIXTURE_GOLDSRC_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            FIXTURE_PLUGINS_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            FIXTURE_COMMON_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            FIXTURE_MODERATION_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            FIXTURE_MODERATION_BUNDLE,
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap()
}

#[allow(dead_code)]
struct BenchResult {
    candidate: &'static str,
    iterations: usize,
    total_time_ns: u128,
    ns_per_op: f64,
    ops_per_sec: f64,
    heap_bytes: usize,
}

fn main() {
    println!(
        "\n=========================================================================================================="
    );
    println!(
        "                         STATEFS UNIFIED MATRIX BENCHMARK (MULTI-CANDIDATE GRID)                           "
    );
    println!(
        "==========================================================================================================\n"
    );

    let query_keys_statefs = [
        "core/server/security/rate_limiter/max_requests_per_sec",
        "core/server/settings/tickrate",
        "plugins/general/auto_load",
        "bundles/moderation/components/moderation/priority",
        "lang/common/common/yes",
    ];

    let query_keys_config = [
        "server.security.rate_limiter.max_requests_per_sec",
        "server.settings.tickrate",
        "general.auto_load",
        "components.moderation.priority",
        "common.yes",
    ];

    let test_scales = [10, 1_000, 50_000];

    for &scale in &test_scales {
        println!(
            "----------------------------------------------------------------------------------------------------------"
        );
        println!(">>> WORKLOAD SCALE: {} OPERATIONS", scale);
        println!(
            "----------------------------------------------------------------------------------------------------------"
        );
        println!(
            "{:<42} | {:>10} | {:>14} | {:>12} | {:>12}",
            "Candidate / Optimization Profile",
            "Time / Op",
            "Throughput",
            "Alloc / Heap",
            "Speedup"
        );
        println!(
            "----------------------------------------------------------------------------------------------------------"
        );

        let mut results = Vec::new();

        // 1. Candidate 1: config-rs
        {
            let before_mem = ALLOCATOR.allocated();
            let conf = build_config_rs();
            let conf_mem = ALLOCATOR.allocated() - before_mem;

            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_config[i % query_keys_config.len()];
                let val: Option<String> = conf.get_string(key).ok();
                black_box(val);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "config-rs (Industry Standard)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: conf_mem,
            });
        }

        // 2. Candidate 2: StateFS (No-Op Baseline / Arena Trie Only)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let store_mem = ALLOCATOR.allocated() - before_mem;

            // Pure get_by_segments traversal without SIMD and without cache
            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let segs = key.split('/');
                let node = store.get_by_segments(segs);
                black_box(node);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (No-Op Baseline: Pure Trie)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: store_mem,
            });
        }

        // 3. Candidate 3: StateFS (+ SIMD Segment Scanner)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let store_mem = ALLOCATOR.allocated() - before_mem;

            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let node = store.get_str(key);
                black_box(node);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (+ SIMD Segment Scanner)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: store_mem,
            });
        }

        // 4. Candidate 4: StateFS (+ SIMD + L1 Path Cache)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let mut resolver: QueryScenarioResolver<64> = store.with_l1_cache::<64>();
            let res_mem = ALLOCATOR.allocated() - before_mem;

            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let node = resolver.resolve_query(QueryIntent::hot_loop(key));
                black_box(node);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (+ SIMD + L1 Direct-Mapped Cache)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: res_mem,
            });
        }

        // 5. Candidate 5: StateFS (Full Stack via Scenario Resolver + stitch-rs Pipeline)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let resolver = store.with_l1_cache::<64>();
            let mut pipeline = Pipeline::on_terminal(StateFsTerminal::new(resolver));
            let pipe_mem = ALLOCATOR.allocated() - before_mem;

            let mut ctx = StateFsContext::default();
            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let res = pipeline.dispatch(&mut ctx, QueryIntent::hot_loop(key));
                let _ = black_box(res);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (stitch-rs Pipeline + Resolver + L1)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: pipe_mem,
            });
        }

        let baseline_time = results[0].ns_per_op;

        for r in &results {
            let speedup = baseline_time / r.ns_per_op;
            let time_str = if r.ns_per_op >= 1000.0 {
                format!("{:.2} µs", r.ns_per_op / 1000.0)
            } else {
                format!("{:.2} ns", r.ns_per_op)
            };

            let ops_str = if r.ops_per_sec >= 1_000_000.0 {
                format!("{:.2} M/s", r.ops_per_sec / 1_000_000.0)
            } else {
                format!("{:.2} K/s", r.ops_per_sec / 1_000.0)
            };

            let mem_str = format!("{:.2} KB", (r.heap_bytes as f64) / 1024.0);

            let speedup_str = if (speedup - 1.0).abs() < 0.05 {
                "1.0x (base)".to_string()
            } else {
                format!("{:.1}x faster", speedup)
            };

            println!(
                "{:<42} | {:>10} | {:>14} | {:>12} | {:>12}",
                r.candidate, time_str, ops_str, mem_str, speedup_str
            );
        }
        println!();
    }
}
