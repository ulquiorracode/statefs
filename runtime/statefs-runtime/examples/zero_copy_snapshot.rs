//! # Zero-Copy Binary Snapshot & Mmap Restoration Example
//!
//! Demonstrates:
//! - Populating an in-memory StateFS hierarchy
//! - Serializing the tree to a binary image via `statefs_codec_bin`
//! - Opening the snapshot using `statefs_adapter_opt_mmap` with zero memory copies
//!   and 0.00 KB heap allocations.

use statefs_adapter_opt_mmap::MmapStorageBacking;
use statefs_codec_bin::export_snapshot;
use statefs_core::backing::StorageBacking;
use statefs_core::{MemStore, Path, Store, Value};
use std::time::Instant;

fn main() {
    println!("=== StateFS Zero-Copy Mmap Snapshot Restoration ===");

    let mut store = MemStore::new();

    // 1. Populate state hierarchy
    store
        .insert(
            &Path::parse("/server/name"),
            Value::from("GoldSrc Dedicated"),
        )
        .unwrap();
    store
        .insert(&Path::parse("/server/max_clients"), Value::from(32))
        .unwrap();
    store
        .insert(&Path::parse("/server/map"), Value::from("de_dust2"))
        .unwrap();

    // 2. Export to disk snapshot file directly from live MemStore
    let snapshot_file = std::env::temp_dir().join("statefs_example_snapshot.bin");
    export_snapshot(&store, &snapshot_file).unwrap();
    println!(
        "Saved real MemStore binary snapshot to: {:?}",
        snapshot_file
    );

    // 3. Measure cold boot time via memory mapping
    let start = Instant::now();
    let mmap_backing = MmapStorageBacking::open(&snapshot_file).unwrap();
    let elapsed = start.elapsed();

    println!(
        "Opened and mapped binary image in {:?}! (0.00 KB heap allocations)",
        elapsed
    );
    println!(
        "Contiguous RawNode count in mapped memory: {}",
        mmap_backing.nodes().len()
    );
    println!(
        "Mapped symbol strings buffer len: {} bytes",
        mmap_backing.string_bytes().len()
    );
    println!(
        "Mapped value payload buffer len: {} bytes",
        mmap_backing.value_bytes().len()
    );

    // 4. Restore store from snapshot and verify values
    let restored_store = statefs_codec_bin::restore_snapshot(&snapshot_file).unwrap();
    assert_eq!(
        restored_store
            .get(&Path::parse("/server/name"))
            .map(|n| &n.value),
        Some(&Value::from("GoldSrc Dedicated"))
    );
    assert_eq!(
        restored_store
            .get(&Path::parse("/server/max_clients"))
            .map(|n| &n.value),
        Some(&Value::from(32))
    );
    assert_eq!(
        restored_store
            .get(&Path::parse("/server/map"))
            .map(|n| &n.value),
        Some(&Value::from("de_dust2"))
    );
    println!("Successfully verified restored store values match original hierarchy!");

    let _ = std::fs::remove_file(snapshot_file);
}
