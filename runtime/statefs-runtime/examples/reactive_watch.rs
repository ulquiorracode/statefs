//! # Reactive Subtree Watch & Glob Queries Example
//!
//! Demonstrates:
//! - Pattern-based querying using `find_glob` with `*` and `**`
//! - Reactive change detection via `SubtreeWatcher` operating in O(1)

use statefs_core::{MemStore, Path, Store, SubtreeWatcher, Value};

fn main() {
    println!("=== StateFS Reactive Subtree Watcher & Glob Querying ===");

    let mut store = MemStore::new();

    // 1. Populate modular configuration
    store
        .insert(&Path::parse("/plugins/admin/enabled"), Value::from(true))
        .unwrap();
    store
        .insert(
            &Path::parse("/plugins/admin/cvars/immunity"),
            Value::from(1),
        )
        .unwrap();
    store
        .insert(&Path::parse("/plugins/stats/enabled"), Value::from(false))
        .unwrap();
    store
        .insert(
            &Path::parse("/plugins/stats/cvars/rank_bots"),
            Value::from(false),
        )
        .unwrap();
    store
        .insert(
            &Path::parse("/plugins/telemetry/enabled"),
            Value::from(true),
        )
        .unwrap();

    // 2. Perform Glob Queries
    println!("\n--- Single Wildcard: /plugins/*/enabled ---");
    let enabled_plugins = store.find_glob("/plugins/*/enabled");
    for (path, node) in &enabled_plugins {
        println!("  {} -> {:?}", path, node.value);
    }
    assert_eq!(enabled_plugins.len(), 3);

    println!("\n--- Recursive Wildcard: /plugins/** ---");
    let all_plugin_entries = store.find_glob("/plugins/**");
    for (path, node) in &all_plugin_entries {
        println!("  {} -> {:?}", path, node.value);
    }
    assert_eq!(all_plugin_entries.len(), 5);

    // 3. Subtree Watcher Demo
    println!("\n--- O(1) Subtree Change Detection ---");
    let mut stats_watcher = SubtreeWatcher::attach(&store, Path::parse("/plugins/stats"));
    assert!(!stats_watcher.poll_changed(&store));

    println!("Modifying /plugins/stats/cvars/rank_bots...");
    store
        .insert(
            &Path::parse("/plugins/stats/cvars/rank_bots"),
            Value::from(true),
        )
        .unwrap();

    if stats_watcher.poll_changed(&store) {
        println!(">>> Event: Subtree /plugins/stats has changed! Triggering reload.");
    }
    assert!(!stats_watcher.poll_changed(&store));
}
