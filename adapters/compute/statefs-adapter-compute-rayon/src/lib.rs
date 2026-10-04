//! # StateFS Rayon Parallel Compute Adapter
//!
//! Provides thread-pool parallel batch lookups across disjoint paths.

use rayon::prelude::*;
use statefs_core::{MemStore, Node};

/// Executes batch lookups in parallel using Rayon worker threads.
pub fn parallel_batch_lookup<'a>(store: &'a MemStore, paths: &[&'a str]) -> Vec<Option<&'a Node>> {
    paths.par_iter().map(|path| store.get_str(path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use statefs_core::{Path, Store, Value};

    #[test]
    fn test_parallel_batch_lookup() {
        let mut store = MemStore::new();
        store.insert(&Path::parse("/a/b"), Value::from(1)).unwrap();
        store.insert(&Path::parse("/c/d"), Value::from(2)).unwrap();

        let paths = ["/a/b", "/c/d", "/e/f"];
        let results = parallel_batch_lookup(&store, &paths);

        assert_eq!(results.len(), 3);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
        assert!(results[2].is_none());
    }
}
