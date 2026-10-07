//! # StateFS Core Nanokernel
//!
//! A pure, deterministic, `no_std`-capable in-memory hierarchical state trie.
//!
//! StateFS Core is completely decoupled from storage media, file systems, and domain logic.
//! It serves as the mathematical foundation for the StateFS ecosystem.
//!
//! ## Key Types
//!
//! - [`Path`]: Canonical segmented path representation (`/a/b/c`).
//! - [`Value`]: Universal strongly-typed value variant.
//! - [`Node`]: Envelope containing a value, revision counter, and metadata flags.
//! - [`Store`]: Primitive trait contract for state tree storage.
//! - [`MemStore`]: Zero-dependency in-memory prefix trie implementation of [`Store`].

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod backing;
pub mod error;
pub mod glob;
pub mod mem;
pub mod node;
pub mod path;
pub mod source;
pub mod store;
pub mod value;
pub mod watch;

pub use backing::{RawNode, SNAPSHOT_MAGIC, SNAPSHOT_VERSION, SnapshotHeader, StorageBacking};
pub use error::StoreError;
pub use glob::match_glob;
pub use mem::MemStore;
pub use node::Node;
pub use path::{Path, PathHandle, PathOptions};
pub use source::StateSource;
pub use store::Store;
pub use value::Value;
pub use watch::SubtreeWatcher;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nanokernel_end_to_end() {
        let mut store = MemStore::new();

        // 1. Insert hierarchical configuration
        store
            .insert(
                &Path::parse("/plugins/moderation/cvars/enabled"),
                Value::from(true),
            )
            .unwrap();
        store
            .insert(
                &Path::parse("/plugins/moderation/cvars/default_ban_mins"),
                Value::from(60),
            )
            .unwrap();
        store
            .insert(
                &Path::parse("/plugins/moderation/errors/not_found"),
                Value::from("Player not found."),
            )
            .unwrap();

        // 2. Query exact nodes
        let ban_mins = store
            .get(&Path::parse("/plugins/moderation/cvars/default_ban_mins"))
            .unwrap();
        assert_eq!(ban_mins.value.as_int(), Some(60));
        assert_eq!(ban_mins.revision, 1);

        // 3. Update node
        store
            .insert(
                &Path::parse("/plugins/moderation/cvars/default_ban_mins"),
                Value::from(120),
            )
            .unwrap();
        let updated = store
            .get(&Path::parse("/plugins/moderation/cvars/default_ban_mins"))
            .unwrap();
        assert_eq!(updated.value.as_int(), Some(120));
        assert_eq!(updated.revision, 2);

        // 4. Subtree inspection
        let all_paths = store.list_subpaths(&Path::parse("/plugins/moderation"));
        assert_eq!(all_paths.len(), 3);
    }

    #[test]
    fn test_subtree_watcher_o1() {
        let mut store = MemStore::new();
        let mut watcher = SubtreeWatcher::new(Path::parse("/server"));

        // Nothing yet
        assert!(!watcher.poll_changed(&store));

        // Insert unrelated node
        store
            .insert(&Path::parse("/client/volume"), Value::from(0.8))
            .unwrap();
        assert!(!watcher.poll_changed(&store));

        // Insert node in /server subtree
        store
            .insert(&Path::parse("/server/tickrate"), Value::from(128))
            .unwrap();
        assert!(watcher.poll_changed(&store));

        // Unmodified check
        assert!(!watcher.poll_changed(&store));

        // Deep modification in /server subtree
        store
            .insert(&Path::parse("/server/physics/gravity"), Value::from(800.0))
            .unwrap();
        assert!(watcher.poll_changed(&store));

        // Node deletion in subtree: watcher must detect deletion
        let mut gravity_watcher =
            SubtreeWatcher::attach(&store, Path::parse("/server/physics/gravity"));
        assert!(gravity_watcher.exists());
        assert!(!gravity_watcher.poll_changed(&store));

        store
            .remove(&Path::parse("/server/physics/gravity"))
            .unwrap();
        assert!(gravity_watcher.poll_changed(&store));
        assert!(!gravity_watcher.exists());
    }

    #[test]
    fn test_find_glob_queries() {
        let mut store = MemStore::new();
        store
            .insert(&Path::parse("/plugins/admin/enabled"), Value::from(true))
            .unwrap();
        store
            .insert(
                &Path::parse("/plugins/moderation/enabled"),
                Value::from(false),
            )
            .unwrap();
        store
            .insert(&Path::parse("/plugins/telemetry/interval"), Value::from(10))
            .unwrap();
        store
            .insert(&Path::parse("/players/1/health"), Value::from(100))
            .unwrap();

        // 1. Single wildcard
        let matches = store.find_glob("/plugins/*/enabled");
        assert_eq!(matches.len(), 2);

        // 2. Recursive wildcard
        let all_plugins = store.find_glob("/plugins/**");
        assert_eq!(all_plugins.len(), 3);
    }
}
