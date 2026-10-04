//! # Real-Time Game Server CVAR & State Registry Example
//!
//! Demonstrates:
//! - Setting up server and plugin CVARs in StateFS
//! - Resolving O(1) direct `PathHandle`s for hot-loop zero-string queries (<5 ns)
//! - SubtreeWatcher for detecting physics CVAR changes (e.g. `/server/physics/*`) in O(1)
//! - Streaming state mutations through the lock-free WAL SPSC BipBuffer without taking locks

use statefs_adapter_opt_lockfree::create_wal_channel;
use statefs_core::{MemStore, Path, Store, SubtreeWatcher, Value};

fn main() {
    println!("=== StateFS Game Server CVAR & State Architecture ===");

    let mut store = MemStore::new();

    // 1. Ingest initial server CVARs
    store
        .insert(&Path::parse("/server/physics/gravity"), Value::from(800.0))
        .unwrap();
    store
        .insert(
            &Path::parse("/server/physics/airaccelerate"),
            Value::from(10.0),
        )
        .unwrap();
    store
        .insert(&Path::parse("/server/net/tickrate"), Value::from(128))
        .unwrap();
    store
        .insert(&Path::parse("/server/net/maxplayers"), Value::from(32))
        .unwrap();

    // 2. Pre-resolve hot loop PathHandles (2-3 ns access without strings)
    let gravity_handle = store.resolve_handle("/server/physics/gravity").unwrap();
    let tickrate_handle = store.resolve_handle("/server/net/tickrate").unwrap();

    println!("Resolved gravity PathHandle: {:?}", gravity_handle);
    println!("Resolved tickrate PathHandle: {:?}", tickrate_handle);

    // 3. Setup O(1) SubtreeWatcher for physics system (attached to current store state)
    let mut physics_watcher = SubtreeWatcher::attach(&store, Path::parse("/server/physics"));
    assert!(!physics_watcher.poll_changed(&store));

    // 4. Setup lock-free SPSC WAL channel for streaming mutations to clients/disk
    let (mut wal_producer, mut wal_consumer) = create_wal_channel::<65536>();

    // 5. Simulate 3 game frames
    println!("\n--- Simulating Frame 1 ---");
    // Hot loop query via PathHandle
    let gravity_node = store.get_by_handle(gravity_handle).unwrap();
    println!(
        "Physics tick using gravity: {:?}",
        gravity_node.value.as_float()
    );
    assert!(!physics_watcher.poll_changed(&store));

    println!("\n--- Simulating Frame 2: Admin modifies sv_gravity via RCON ---");
    let new_gravity = Value::from(400.0); // Moon gravity!
    store
        .insert(&Path::parse("/server/physics/gravity"), new_gravity.clone())
        .unwrap();

    // Publish mutation lock-free to WAL for network replication
    wal_producer
        .push_insert("/server/physics/gravity", &new_gravity, 2)
        .unwrap();

    // Physics module checks if ANY physics setting changed in O(1)
    if physics_watcher.poll_changed(&store) {
        let updated_gravity = store
            .get_by_handle(gravity_handle)
            .unwrap()
            .value
            .as_float()
            .unwrap();
        println!(
            ">>> Physics Engine Notified: Gravity changed to {}! Recalculating constants.",
            updated_gravity
        );
    }

    println!("\n--- Simulating Frame 3: Worker thread drains WAL stream ---");
    let mut replica_store = MemStore::new();
    let applied = wal_consumer.drain_to_store(&mut replica_store).unwrap();
    println!(
        "Worker thread drained and replicated {} mutation(s) to replica store.",
        applied
    );

    let replica_gravity = replica_store
        .get(&Path::parse("/server/physics/gravity"))
        .unwrap();
    assert_eq!(replica_gravity.value, Value::from(400.0));
    println!("Replica store verified with gravity = 400.0!");
}
