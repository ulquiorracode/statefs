//! Real Dedicated Server Production Benchmark (StateFS vs config-rs).
//!
//! Loads real server configurations from `C:\Users\Administrator\Desktop\server\cstrike`:
//! - `addons\goldsrc\goldsrc.toml`
//! - `addons\goldsrc\configs\plugins.toml`
//! - `addons\goldsrc\data\lang\common.toml`
//! - Plugin localization dictionaries and bundles
//!
//! Measures:
//! 1. Memory consumption & Total Heap Allocations (via Cap allocator).
//! 2. Production startup & ingestion latency.
//! 3. Hot-path game tick / network packet query latency.

use cap::Cap;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use statefs_core::{MemStore, Path, Store, Value};
use std::alloc;

#[global_allocator]
static ALLOCATOR: Cap<alloc::System> = Cap::new(alloc::System, usize::MAX);

// Read production configs from the server directory
const PROD_GOLDSRC_TOML: &str =
    include_str!(r"C:\Users\Administrator\Desktop\server\cstrike\addons\goldsrc\goldsrc.toml");
const PROD_PLUGINS_TOML: &str = include_str!(
    r"C:\Users\Administrator\Desktop\server\cstrike\addons\goldsrc\configs\plugins.toml"
);
const PROD_COMMON_LANG: &str = include_str!(
    r"C:\Users\Administrator\Desktop\server\cstrike\addons\goldsrc\data\lang\common.toml"
);
const PROD_MODERATION_LANG: &str =
    include_str!(r"D:\Repo\GoldSrc.rs\goldsrc-rs\plugins\moderation\lang\moderation.toml");
const PROD_MODERATION_BUNDLE: &str =
    include_str!(r"D:\Repo\GoldSrc.rs\goldsrc-rs\plugins\moderation\bundle.toml");

fn bench_real_server_workloads(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_server_prod_workload");

    // =========================================================================
    // Workload 1: Full Server Startup & Configuration Ingestion (All 5 Files)
    // =========================================================================

    group.bench_function("config_rs/full_server_boot_ingestion", |b| {
        b.iter(|| {
            let conf = config::Config::builder()
                .add_source(config::File::from_str(
                    black_box(PROD_GOLDSRC_TOML),
                    config::FileFormat::Toml,
                ))
                .add_source(config::File::from_str(
                    black_box(PROD_PLUGINS_TOML),
                    config::FileFormat::Toml,
                ))
                .add_source(config::File::from_str(
                    black_box(PROD_COMMON_LANG),
                    config::FileFormat::Toml,
                ))
                .add_source(config::File::from_str(
                    black_box(PROD_MODERATION_LANG),
                    config::FileFormat::Toml,
                ))
                .add_source(config::File::from_str(
                    black_box(PROD_MODERATION_BUNDLE),
                    config::FileFormat::Toml,
                ))
                .build()
                .unwrap();
            black_box(conf);
        })
    });

    group.bench_function("statefs/full_server_boot_ingestion", |b| {
        b.iter(|| {
            let mut store = MemStore::new();

            for (prefix, raw) in [
                ("core", PROD_GOLDSRC_TOML),
                ("plugins", PROD_PLUGINS_TOML),
                ("lang/common", PROD_COMMON_LANG),
                ("lang/moderation", PROD_MODERATION_LANG),
                ("bundles/moderation", PROD_MODERATION_BUNDLE),
            ] {
                let toml_val: toml::Value = toml::from_str(black_box(raw)).unwrap();
                let p = Path::parse(prefix);
                flatten_toml_to_statefs(&mut store, &p, &toml_val);
            }

            black_box(store);
        })
    });

    // =========================================================================
    // Workload 2: Hot-Path Game Tick Queries (Server checking settings in tick)
    // =========================================================================

    // Pre-build statefs instance with real server configs
    let mut statefs_prod = MemStore::new();
    for (prefix, raw) in [
        ("core", PROD_GOLDSRC_TOML),
        ("plugins", PROD_PLUGINS_TOML),
        ("lang/common", PROD_COMMON_LANG),
        ("lang/moderation", PROD_MODERATION_LANG),
        ("bundles/moderation", PROD_MODERATION_BUNDLE),
    ] {
        let toml_val: toml::Value = toml::from_str(raw).unwrap();
        let p = Path::parse(prefix);
        flatten_toml_to_statefs(&mut statefs_prod, &p, &toml_val);
    }

    // Pre-build config-rs instance with real server configs
    let config_rs_prod = config::Config::builder()
        .add_source(config::File::from_str(
            PROD_GOLDSRC_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_PLUGINS_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_COMMON_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_MODERATION_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_MODERATION_BUNDLE,
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap();

    // Query 1: Single setting lookup
    group.bench_function("config_rs/query_plugin_priority", |b| {
        b.iter(|| {
            let val: i64 = config_rs_prod
                .get_int(black_box("components.moderation.priority"))
                .unwrap();
            black_box(val);
        })
    });

    group.bench_function("statefs/query_plugin_priority_zero_alloc", |b| {
        b.iter(|| {
            let val = statefs_prod
                .get_str(black_box(
                    "/bundles/moderation/components/moderation/priority",
                ))
                .unwrap()
                .value
                .as_int()
                .unwrap();
            black_box(val);
        })
    });

    // Query 2: Disciplinary Sanction / Chat Translation Lookup (Simulating incoming chat / kick)
    group.bench_function("config_rs/i18n_translation_lookup", |b| {
        b.iter(|| {
            let msg: String = config_rs_prod
                .get_string(black_box("translations.ru.kick_broadcast"))
                .unwrap();
            black_box(msg);
        })
    });

    group.bench_function("statefs/i18n_translation_lookup_zero_alloc", |b| {
        b.iter(|| {
            let msg = statefs_prod
                .get_str(black_box("/lang/moderation/translations/ru/kick_broadcast"))
                .unwrap()
                .value
                .as_str()
                .unwrap();
            black_box(msg);
        })
    });

    group.finish();

    // Print exact memory footprint breakdown to stdout during benchmark
    print_memory_footprint();
}

fn print_memory_footprint() {
    let before_config = ALLOCATOR.allocated();
    let _conf = config::Config::builder()
        .add_source(config::File::from_str(
            PROD_GOLDSRC_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_PLUGINS_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_COMMON_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_MODERATION_LANG,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            PROD_MODERATION_BUNDLE,
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap();
    let config_rs_bytes = ALLOCATOR.allocated() - before_config;

    let before_statefs = ALLOCATOR.allocated();
    let mut store = MemStore::new();
    for (prefix, raw) in [
        ("core", PROD_GOLDSRC_TOML),
        ("plugins", PROD_PLUGINS_TOML),
        ("lang/common", PROD_COMMON_LANG),
        ("lang/moderation", PROD_MODERATION_LANG),
        ("bundles/moderation", PROD_MODERATION_BUNDLE),
    ] {
        let toml_val: toml::Value = toml::from_str(raw).unwrap();
        let p = Path::parse(prefix);
        flatten_toml_to_statefs(&mut store, &p, &toml_val);
    }
    let statefs_bytes = ALLOCATOR.allocated() - before_statefs;

    println!("\n=======================================================");
    println!("     REAL SERVER PRODUCTION MEMORY USAGE BREAKDOWN      ");
    println!("=======================================================");
    println!(
        "  -> config-rs Resident Heap  : {:>8} bytes ({:.2} KB)",
        config_rs_bytes,
        config_rs_bytes as f64 / 1024.0
    );
    println!(
        "  -> StateFS Resident Heap    : {:>8} bytes ({:.2} KB)",
        statefs_bytes,
        statefs_bytes as f64 / 1024.0
    );
    println!("=======================================================\n");
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
        toml::Value::Array(arr) => {
            let vals: Vec<Value> = arr
                .iter()
                .filter_map(|v| match v {
                    toml::Value::String(s) => Some(Value::from(s.as_str())),
                    toml::Value::Integer(i) => Some(Value::from(*i)),
                    toml::Value::Boolean(b) => Some(Value::from(*b)),
                    _ => None,
                })
                .collect();
            store.insert(current_path, Value::Array(vals)).unwrap();
        }
        toml::Value::Datetime(dt) => {
            store
                .insert(current_path, Value::from(dt.to_string()))
                .unwrap();
        }
    }
}

criterion_group!(benches, bench_real_server_workloads);
criterion_main!(benches);
