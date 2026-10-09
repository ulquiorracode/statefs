//! # Unified Matrix Benchmark (Multi-Candidate & Multi-Load Grid)
//!
//! Evaluates:
//! - Candidates:
//!   1. `config-rs` (Industry Standard Baseline)
//!   2. `StateFS (No-Op Baseline / Pure Arena Trie)`
//!   3. `StateFS (+ SIMD Segment Scanner Adapter)`
//!   4. `StateFS (+ SIMD + L1 Direct-Mapped Path Cache Adapter)`
//!   5. `StateFS (stitch-rs Pipeline + Scenario Resolver)`
//!   6. `StateFS (config-rs Drop-in Compatibility Bridge)`
//!   7. `StateFS (Lock-Free WAL SPSC Push via bbqueue)`
//!
//! - Metrics:
//!   - Latency per operation (ns)
//!   - Throughput (ops/sec)
//!   - Resident Heap Memory (bytes / KB via Cap tracking)
//!
//! - Load Tiers:
//!   - 10 ops (Cold Start / Light)
//!   - 1,000 ops (Typical Frame / Medium)
//!   - 50,000 ops (High-Frequency Game Loop / Heavy Burst)

use cap::Cap;
use statefs_adapter_bridge_config::ConfigBridge;
use statefs_adapter_opt_lockfree::create_wal_channel;
use statefs_adapter_opt_mmap::MmapStorageBacking;
use statefs_adapter_path_handles::PathHandleCache;
use statefs_codec_bin::export_snapshot;
use statefs_codec_toml::ingest_toml_str;
use statefs_core::{MemStore, Store, Value};
use statefs_runtime::{
    QueryIntent, QueryScenarioResolver, RuntimeExt, StateFsContext, StateFsTerminal,
};
use std::alloc;
use std::hint::black_box;
use std::time::Instant;
use stitch_rs::pipeline::Pipeline;

#[global_allocator]
static ALLOCATOR: Cap<alloc::System> = Cap::new(alloc::System, usize::MAX);

const FIXTURE_GOLDSRC_TOML: &str =
    include_str!("../../../core/statefs-core/benches/fixtures/goldsrc.toml");
const FIXTURE_PLUGINS_TOML: &str =
    include_str!("../../../core/statefs-core/benches/fixtures/plugins.toml");
const FIXTURE_COMMON_LANG: &str =
    include_str!("../../../core/statefs-core/benches/fixtures/common.toml");
const FIXTURE_MODERATION_LANG: &str =
    include_str!("../../../core/statefs-core/benches/fixtures/moderation.toml");
const FIXTURE_MODERATION_BUNDLE: &str =
    include_str!("../../../core/statefs-core/benches/fixtures/bundle.toml");

fn populate_statefs(store: &mut MemStore) {
    for (prefix, raw) in [
        ("core", FIXTURE_GOLDSRC_TOML),
        ("plugins", FIXTURE_PLUGINS_TOML),
        ("lang/common", FIXTURE_COMMON_LANG),
        ("lang/moderation", FIXTURE_MODERATION_LANG),
        ("bundles/moderation", FIXTURE_MODERATION_BUNDLE),
    ] {
        ingest_toml_str(store, prefix, raw).unwrap();
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

    // ----------------------------------------------------------------------------------------------------------
    // PART 1: COLD START / BOOTSTRAP LATENCY BENCHMARK
    // ----------------------------------------------------------------------------------------------------------
    println!(">>> BENCHMARK SECTION: COLD START & INGESTION LATENCY (BOOT-TIME)");
    println!(
        "----------------------------------------------------------------------------------------------------------"
    );
    println!(
        "{:<44} | {:>14} | {:>14} | {:>12}",
        "Candidate Bootstrap Source", "Boot Time (µs)", "Throughput (ops)", "Heap Alloc"
    );
    println!(
        "----------------------------------------------------------------------------------------------------------"
    );

    // 1. config-rs boot from 5 raw TOML files
    let config_boot_us = {
        let before_mem = ALLOCATOR.allocated();
        let start = Instant::now();
        let cfg = build_config_rs();
        let elapsed = start.elapsed().as_micros();
        let heap_used = ALLOCATOR.allocated() - before_mem;
        let _ = black_box(cfg);
        println!(
            "{:<44} | {:>11} µs | {:>14} | {:>9.2} KB",
            "config-rs (5 TOML strings parse)",
            elapsed,
            "1 cold build",
            (heap_used as f64) / 1024.0
        );
        elapsed
    };

    // 2. StateFS boot from 5 raw TOML files
    let _statefs_boot_us = {
        let before_mem = ALLOCATOR.allocated();
        let start = Instant::now();
        let mut store = MemStore::new();
        populate_statefs(&mut store);
        let elapsed = start.elapsed().as_micros();
        let heap_used = ALLOCATOR.allocated() - before_mem;
        let _ = black_box(store);
        let speedup = (config_boot_us as f64) / (elapsed.max(1) as f64);
        println!(
            "{:<44} | {:>11} µs | {:>10.1}x base | {:>9.2} KB",
            "StateFS (5 TOML streaming ingestion)",
            elapsed,
            speedup,
            (heap_used as f64) / 1024.0
        );
        elapsed
    };

    // 3. StateFS Zero-Copy Mmap Snapshot boot (True full store export of all 5 configs)
    {
        let mut store = MemStore::new();
        populate_statefs(&mut store);

        let temp_snapshot_path = std::env::temp_dir().join("statefs_matrix_boot.bin");
        // Dump the entire populated store (all 5 TOMLs converted to arena nodes and interned strings)
        export_snapshot(&store, &temp_snapshot_path).unwrap();

        let before_mem = ALLOCATOR.allocated();
        let start = Instant::now();
        let mmap_backing = MmapStorageBacking::open(&temp_snapshot_path).unwrap();
        let elapsed_ns = start.elapsed().as_nanos();
        let heap_used = ALLOCATOR.allocated() - before_mem;
        let _ = black_box(&mmap_backing);

        let elapsed_us = (elapsed_ns as f64) / 1000.0;
        let speedup = (config_boot_us as f64) / elapsed_us.max(0.001);
        let time_str = format!("{:.2} µs ({} ns)", elapsed_us, elapsed_ns);
        println!(
            "{:<44} | {:>14} | {:>10.1}x base | {:>9.2} KB",
            "StateFS (Full Store Mmap Image Open)",
            time_str,
            speedup,
            (heap_used as f64) / 1024.0
        );

        let _ = std::fs::remove_file(temp_snapshot_path);
    }
    println!();

    // ----------------------------------------------------------------------------------------------------------
    // PART 2: QUERY & MUTATION RUNTIME MATRIX
    // ----------------------------------------------------------------------------------------------------------

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
            "{:<44} | {:>10} | {:>14} | {:>12} | {:>12}",
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

        // 2. Candidate 2: StateFS (No-Op Baseline / Pure Arena Trie)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let store_mem = ALLOCATOR.allocated() - before_mem;

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

        // 3. Candidate 3: StateFS (+ SIMD Segment Scanner Adapter)
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
                candidate: "StateFS (+ SIMD Path Scanner Adapter)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: store_mem,
            });
        }

        // 4. Candidate 4: StateFS (+ SIMD + L1 Direct-Mapped Path Cache Adapter)
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
                candidate: "StateFS (+ SIMD + L1 Cache Adapter)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: res_mem,
            });
        }

        // 5. Candidate 5: StateFS (stitch-rs Pipeline + Scenario Resolver)
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
                candidate: "StateFS (stitch-rs Pipeline + Resolver)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: pipe_mem,
            });
        }

        // 6. Candidate 6: StateFS (config-rs Drop-in Compatibility Bridge)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let mut bridge = ConfigBridge::<64>::new(store);
            let bridge_mem = ALLOCATOR.allocated() - before_mem;

            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let val: Result<String, _> = bridge.get_string(key);
                let _ = black_box(val);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (config-rs Drop-in Bridge)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: bridge_mem,
            });
        }

        // 7. Candidate 7: StateFS (Honest Full-Cycle WAL SPSC: Push + Drain to MemStore)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let (mut producer, mut consumer) = create_wal_channel::<65536>();
            let wal_mem = ALLOCATOR.allocated() - before_mem;

            let test_val = Value::from(100);
            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let _ = black_box(producer.push_insert(key, &test_val, i as u64));
                if i % 32 == 0 {
                    let _ = black_box(consumer.drain_to_store_fast(&mut store));
                }
            }
            let _ = black_box(consumer.drain_to_store_fast(&mut store));
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (Honest WAL: Push + DrainToStore)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: wal_mem,
            });
        }

        // 7a. WAL Stage Breakdown: Push Only (Ringbuffer Framing + Grant Commit)
        {
            let (mut producer, mut consumer) = create_wal_channel::<65536>();
            let test_val = Value::from(100);
            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let _ = black_box(producer.push_insert(key, &test_val, i as u64));
                if i % 32 == 0 {
                    let _ = black_box(consumer.discard_all());
                }
            }
            let _ = black_box(consumer.discard_all());
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "  -> WAL Breakdown [1/3]: Push Only (Framing+Commit)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: 0,
            });
        }


        // 7b. WAL Stage Breakdown: Apply Only (Direct MemStore Path::parse + Insert)
        {
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let test_val = Value::from(100);
            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let parsed = statefs_core::Path::parse(key);
                let _ = black_box(store.insert(&parsed, test_val.clone()));
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "  -> WAL Breakdown [2/3]: Apply Only (Path::parse+Insert)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: 0,
            });
        }


        // 8. Candidate 8: StateFS (PathHandle Direct O(1) Index Lookup)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let mut handle_cache = PathHandleCache::<64>::new();
            let handle_mem = ALLOCATOR.allocated() - before_mem;

            // Pre-warm handle cache
            for key in query_keys_statefs {
                let _ = handle_cache.resolve_or_lookup(&store, key);
            }

            let start = Instant::now();
            for i in 0..scale {
                let key = query_keys_statefs[i % query_keys_statefs.len()];
                let node = handle_cache.get(key).and_then(|h| store.get_by_handle(h));
                let _ = black_box(node);
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (PathHandle O(1) Direct Lookup)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: handle_mem,
            });
        }

        // 9. Candidate 9: StateFS (Real-World Game Loop: 95% Read + 5% Write with Epoch Invalidation)
        {
            let before_mem = ALLOCATOR.allocated();
            let mut store = MemStore::new();
            populate_statefs(&mut store);
            let mut handle_cache = PathHandleCache::<64>::new();
            let mixed_mem = ALLOCATOR.allocated() - before_mem;

            let mutation_val = Value::from(500);
            let mutation_path = statefs_core::Path::parse("core/server/settings/tickrate");

            let start = Instant::now();
            for i in 0..scale {
                if i % 20 == 0 {
                    // 5% Writes: update store and invalidate or trigger epoch roll
                    let _ = store.insert(&mutation_path, mutation_val.clone());
                } else {
                    // 95% Reads: safe lookup through handle cache with epoch check
                    let key = query_keys_statefs[i % query_keys_statefs.len()];
                    let node = handle_cache
                        .resolve_or_lookup(&store, key)
                        .and_then(|h| store.get_by_handle(h));
                    let _ = black_box(node);
                }
            }
            let elapsed = start.elapsed().as_nanos();

            results.push(BenchResult {
                candidate: "StateFS (Mixed: 95% Read + 5% Write Loop)",
                iterations: scale,
                total_time_ns: elapsed,
                ns_per_op: (elapsed as f64) / (scale as f64),
                ops_per_sec: ((scale as f64) / (elapsed as f64)) * 1_000_000_000.0,
                heap_bytes: mixed_mem,
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
                "{:<44} | {:>10} | {:>14} | {:>12} | {:>12}",
                r.candidate, time_str, ops_str, mem_str, speedup_str
            );
        }
        println!();
    }
}
