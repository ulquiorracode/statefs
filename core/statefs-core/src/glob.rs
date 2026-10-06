//! # Zero-Cost Glob Pattern Matcher
//!
//! Performs path matching over the arena trie supporting:
//! - Single segment wildcards: `*`
//! - Multi-segment recursive wildcards: `**`
//! - Exact literal segments: `foo`, `bar`

use crate::mem::MemStore;
use crate::node::Node;
use crate::path::Path;
use alloc::borrow::ToOwned;
use alloc::vec::Vec;

/// Maximum recursion depth allowed during glob traversal to prevent stack exhaustion.
const MAX_GLOB_DEPTH: usize = 64;

/// Matches state tree nodes against a glob pattern.
///
/// Supports:
/// - `*`: matches exactly one path segment.
/// - `**`: matches zero or more path segments recursively.
/// - literal text: matches exact segment name.
pub fn match_glob<'a>(store: &'a MemStore, pattern: &str) -> Vec<(Path, &'a Node)> {
    let segments: Vec<&str> = pattern
        .split(['/', '\\'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let mut results = Vec::new();
    let mut current_path: Vec<&'a str> = Vec::new();

    walk(store, 0, &segments, 0, &mut current_path, &mut results);

    // Deduplicate by path
    results.dedup_by(|a, b| a.0 == b.0);
    results
}

fn walk<'a>(
    store: &'a MemStore,
    current_node: u32,
    pattern: &[&str],
    pattern_idx: usize,
    current_path: &mut Vec<&'a str>,
    results: &mut Vec<(Path, &'a Node)>,
) {
    if current_path.len() >= MAX_GLOB_DEPTH {
        return;
    }

    if pattern_idx == pattern.len() {
        if let Some(node) = store.node_value(current_node) {
            let path = Path::from_segments(current_path.iter().map(|&s| s.to_owned()));
            results.push((path, node));
        }
        return;
    }

    let seg = pattern[pattern_idx];

    if seg == "**" {
        // Case 1: ** matches 0 segments
        walk(
            store,
            current_node,
            pattern,
            pattern_idx + 1,
            current_path,
            results,
        );

        // Case 2: ** matches 1 segment and continues
        let mut child = store.node_first_child(current_node);
        while child != u32::MAX {
            let child_sym = store.node_symbol_str(child);
            current_path.push(child_sym);

            walk(store, child, pattern, pattern_idx, current_path, results);

            current_path.pop();
            child = store.node_next_sibling(child);
        }
        return;
    }

    let mut child = store.node_first_child(current_node);
    while child != u32::MAX {
        let child_sym = store.node_symbol_str(child);

        if seg == "*" || seg == child_sym {
            current_path.push(child_sym);
            walk(
                store,
                child,
                pattern,
                pattern_idx + 1,
                current_path,
                results,
            );
            current_path.pop();
        }

        child = store.node_next_sibling(child);
    }
}
