//! Real-World Enterprise Production Configuration Benchmark:
//! StateFS vs config-rs (Standard Industry Choice).
//!
//! Scenario:
//! A microservice / game cluster configuration with messy, deeply nested,
//! heterogenous data: network timeouts, database pool settings, feature flags,
//! multi-region routing rules, rate limiters, and telemetry options.
//!
//! Workload A: Initial Build & Merge (Layering).
//! Workload B: Hot-Path Queries (Reading deeply nested configurations during high-load traffic).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use statefs_core::{MemStore, Path, Store, Value};

// 1. Raw messy enterprise TOML document
const BASE_CONFIG_TOML: &str = r#"
[server]
host = "0.0.0.0"
port = 27015
max_players = 64
tickrate = 128

[server.security]
enable_auth = true
jwt_secret = "very-secret-production-token-12345"
token_ttl_seconds = 86400

[server.security.rate_limiter]
enabled = true
max_requests_per_sec = 500
burst_limit = 1000

[database.primary]
url = "postgres://admin:password@db-primary.internal:5432/game_prod"
pool_size = 32
timeout_ms = 5000
keepalive = true

[database.replica]
url = "postgres://ro_user:password@db-replica.internal:5432/game_prod"
pool_size = 64
timeout_ms = 3000

[features.moderation]
enabled = true
auto_ban_on_speedhack = true
max_warnings = 3

[features.moderation.reasons]
cheating = "Permanent ban for third-party software"
griefing = "24-hour suspension for intentional team disruption"
toxicity = "1-hour silence for offensive chat language"

[telemetry]
endpoint = "https://otel-collector.internal:4317"
sample_rate = 0.05
export_interval_secs = 10
"#;

// 2. Production override layer (e.g. staging or per-node override)
const OVERRIDE_CONFIG_TOML: &str = r#"
[server]
port = 27020

[server.security.rate_limiter]
max_requests_per_sec = 2500

[database.primary]
pool_size = 128

[features.moderation]
auto_ban_on_speedhack = false
"#;

fn bench_enterprise_workloads(c: &mut Criterion) {
    let mut group = c.benchmark_group("enterprise_config_comparison");

    // -------------------------------------------------------------
    // Scenario 1: Setup & Layered Build (Base + Override)
    // -------------------------------------------------------------

    group.bench_function("config_rs/build_and_merge", |b| {
        b.iter(|| {
            let conf = config::Config::builder()
                .add_source(config::File::from_str(
                    black_box(BASE_CONFIG_TOML),
                    config::FileFormat::Toml,
                ))
                .add_source(config::File::from_str(
                    black_box(OVERRIDE_CONFIG_TOML),
                    config::FileFormat::Toml,
                ))
                .build()
                .unwrap();
            black_box(conf);
        })
    });

    group.bench_function("statefs/build_and_merge", |b| {
        b.iter(|| {
            let mut store = MemStore::new();

            // Populate base layer
            let base_val: toml::Value = toml::from_str(black_box(BASE_CONFIG_TOML)).unwrap();
            flatten_toml_to_statefs(&mut store, &Path::root(), &base_val);

            // Populate override layer
            let over_val: toml::Value = toml::from_str(black_box(OVERRIDE_CONFIG_TOML)).unwrap();
            flatten_toml_to_statefs(&mut store, &Path::root(), &over_val);

            black_box(store);
        })
    });

    // -------------------------------------------------------------
    // Scenario 2: Hot-Path Deeply Nested Value Query
    // Query: server.security.rate_limiter.max_requests_per_sec
    // Query: database.primary.pool_size
    // -------------------------------------------------------------

    // Pre-build config-rs instance
    let config_rs_instance = config::Config::builder()
        .add_source(config::File::from_str(
            BASE_CONFIG_TOML,
            config::FileFormat::Toml,
        ))
        .add_source(config::File::from_str(
            OVERRIDE_CONFIG_TOML,
            config::FileFormat::Toml,
        ))
        .build()
        .unwrap();

    // Pre-build statefs instance
    let mut statefs_instance = MemStore::new();
    let base_val: toml::Value = toml::from_str(BASE_CONFIG_TOML).unwrap();
    flatten_toml_to_statefs(&mut statefs_instance, &Path::root(), &base_val);
    let over_val: toml::Value = toml::from_str(OVERRIDE_CONFIG_TOML).unwrap();
    flatten_toml_to_statefs(&mut statefs_instance, &Path::root(), &over_val);

    // Verify values match
    assert_eq!(
        config_rs_instance
            .get_int("server.security.rate_limiter.max_requests_per_sec")
            .unwrap(),
        2500
    );
    assert_eq!(
        statefs_instance
            .get_str("/server/security/rate_limiter/max_requests_per_sec")
            .unwrap()
            .value
            .as_int(),
        Some(2500)
    );

    group.bench_function("config_rs/get_deep_nested_int", |b| {
        b.iter(|| {
            let val: i64 = config_rs_instance
                .get_int(black_box(
                    "server.security.rate_limiter.max_requests_per_sec",
                ))
                .unwrap();
            black_box(val);
        })
    });

    let statefs_path = Path::parse("/server/security/rate_limiter/max_requests_per_sec");
    group.bench_function("statefs/get_path_parsed", |b| {
        b.iter(|| {
            let node = statefs_instance.get(black_box(&statefs_path)).unwrap();
            let val = node.value.as_int().unwrap();
            black_box(val);
        })
    });

    group.bench_function("statefs/get_str_zero_alloc", |b| {
        b.iter(|| {
            let node = statefs_instance
                .get_str(black_box(
                    "/server/security/rate_limiter/max_requests_per_sec",
                ))
                .unwrap();
            let val = node.value.as_int().unwrap();
            black_box(val);
        })
    });

    // Multiple queries in single hot loop (simulating reading request config during traffic)
    group.bench_function("config_rs/multi_query_batch", |b| {
        b.iter(|| {
            let p1: i64 = config_rs_instance
                .get_int(black_box("database.primary.pool_size"))
                .unwrap();
            let p2: bool = config_rs_instance
                .get_bool(black_box("features.moderation.enabled"))
                .unwrap();
            let p3: String = config_rs_instance
                .get_string(black_box("features.moderation.reasons.cheating"))
                .unwrap();
            black_box((p1, p2, p3));
        })
    });

    group.bench_function("statefs/multi_query_batch_zero_alloc", |b| {
        b.iter(|| {
            let p1 = statefs_instance
                .get_str(black_box("/database/primary/pool_size"))
                .unwrap()
                .value
                .as_int()
                .unwrap();
            let p2 = statefs_instance
                .get_str(black_box("/features/moderation/enabled"))
                .unwrap()
                .value
                .as_bool()
                .unwrap();
            let p3 = statefs_instance
                .get_str(black_box("/features/moderation/reasons/cheating"))
                .unwrap()
                .value
                .as_str()
                .unwrap();
            black_box((p1, p2, p3));
        })
    });

    group.finish();
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

criterion_group!(benches, bench_enterprise_workloads);
criterion_main!(benches);
