use criterion::{Criterion, black_box, criterion_group, criterion_main};
use statefs_core::{MemStore, Path, PathOptions, Store, Value};

fn bench_path_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("path_parsing");

    let uri = "/plugins/moderation/cvars/default_ban_time";
    let dot = "plugins.moderation.cvars.default_ban_time";

    group.bench_function("parse_default_uri", |b| {
        b.iter(|| {
            let path = Path::parse(black_box(uri));
            black_box(path);
        })
    });

    group.bench_function("parse_dot_notation", |b| {
        b.iter(|| {
            let path =
                Path::parse_with_options(black_box(dot), black_box(&PathOptions::DOT_NOTATION));
            black_box(path);
        })
    });

    group.finish();
}

fn bench_memstore_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memstore");

    // Pre-populate store with 1,000 hierarchical nodes
    let mut store = MemStore::new();
    let paths: Vec<Path> = (0..1_000)
        .map(|i| Path::parse(&format!("/services/worker_{}/config/concurrency", i)))
        .collect();

    for path in &paths {
        store.insert(path, Value::from(16)).unwrap();
    }

    let target_path = &paths[500];

    group.bench_function("get_existing_node", |b| {
        b.iter(|| {
            let node = store.get(black_box(target_path));
            black_box(node);
        })
    });

    group.bench_function("insert_overwrite", |b| {
        b.iter(|| {
            store
                .insert(black_box(target_path), Value::from(32))
                .unwrap();
        })
    });

    let prefix = Path::parse("/services/worker_500");
    group.bench_function("list_subpaths", |b| {
        b.iter(|| {
            let sub = store.list_subpaths(black_box(&prefix));
            black_box(sub);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_path_parsing, bench_memstore_operations);
criterion_main!(benches);
