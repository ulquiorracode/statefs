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

pub mod error;
pub mod mem;
pub mod node;
pub mod path;
pub mod store;
pub mod value;

pub use error::StoreError;
pub use mem::MemStore;
pub use node::Node;
pub use path::Path;
pub use store::Store;
pub use value::Value;

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
}
