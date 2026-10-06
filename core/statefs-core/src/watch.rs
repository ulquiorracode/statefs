//! # Zero-Cost Subtree Watcher & Reactive Observer
//!
//! Provides O(1) change detection for arbitrary tree prefixes using
//! monotonic subtree revisions.

use crate::mem::MemStore;
use crate::path::Path;

/// A lightweight watcher tracking monotonic changes in a subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtreeWatcher {
    path: Path,
    last_revision: u64,
    exists: bool,
}

impl SubtreeWatcher {
    /// Creates a new watcher for the specified path prefix starting at revision 0.
    pub fn new(path: Path) -> Self {
        Self {
            path,
            last_revision: 0,
            exists: false,
        }
    }

    /// Attaches a watcher to an existing store, synchronizing with its current revision.
    pub fn attach(store: &MemStore, path: Path) -> Self {
        let rev = store.subtree_revision(&path);
        Self {
            path,
            last_revision: rev.unwrap_or(0),
            exists: rev.is_some(),
        }
    }

    /// Checks if the target subtree has been modified since the last check.
    ///
    /// If modified, created, or deleted, returns `true` and updates the internal watermark.
    /// Operates in O(1) without scanning subtree descendants.
    pub fn poll_changed(&mut self, store: &MemStore) -> bool {
        match store.subtree_revision(&self.path) {
            Some(rev) => {
                if !self.exists || rev > self.last_revision {
                    self.exists = true;
                    self.last_revision = rev;
                    true
                } else {
                    false
                }
            }
            None => {
                if self.exists {
                    self.exists = false;
                    self.last_revision = 0;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Returns `true` if the target subtree currently exists.
    pub fn exists(&self) -> bool {
        self.exists
    }

    /// Returns the last observed revision watermark.
    pub fn last_revision(&self) -> u64 {
        self.last_revision
    }

    /// Returns the target path of this watcher.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
