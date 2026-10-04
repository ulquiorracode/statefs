//! # StateFS Path Optimization Core Port & Hub
//!
//! Provides the primary [`PathOptimizer`] port abstraction and the recursive
//! [`PathOptimizerHub`] allowing multiple specialized path compression and
//! normalization adapters to be chained together via `with_step`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

/// Fixed-capacity inline path buffer (256 bytes) avoiding heap allocations on hot paths.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InlinePath {
    buffer: [u8; 256],
    len: usize,
}

impl Default for InlinePath {
    fn default() -> Self {
        Self::new()
    }
}

impl InlinePath {
    /// Creates an empty inline path buffer.
    pub const fn new() -> Self {
        Self {
            buffer: [0u8; 256],
            len: 0,
        }
    }

    /// Creates an inline path buffer initialized from a string slice.
    pub fn from_raw(s: &str) -> Self {
        let mut p = Self::new();
        p.set(s);
        p
    }
}

impl From<&str> for InlinePath {
    #[inline]
    fn from(s: &str) -> Self {
        Self::from_raw(s)
    }
}

impl InlinePath {
    /// Sets the contents of the inline path.
    pub fn set(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(256);
        self.buffer[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.len = copy_len;
    }

    /// Borrows the current contents as a valid UTF-8 string slice.
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buffer[..self.len]).unwrap_or("")
    }

    /// Length in bytes.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Port abstraction for path optimization, tokenization, or normalization adapters.
pub trait PathOptimizer: Send + Sync {
    /// Optimizes or mutates the path in-place within the inline buffer.
    fn optimize(&self, path: &mut InlinePath);
}

/// Composite hub orchestrating a sequence of path optimizers without fixed limits.
///
/// Any number of micro-adapters can be appended using [`with_step`](PathOptimizerHub::with_step).
#[derive(Default)]
pub struct PathOptimizerHub {
    steps: Vec<Box<dyn PathOptimizer>>,
}

impl PathOptimizerHub {
    /// Creates a new empty optimizer hub.
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Appends a new path optimizer step to the hub.
    ///
    /// Following the recursive composite pattern, this method is named `with_step`
    /// at all levels of hierarchy.
    pub fn with_step<O: PathOptimizer + 'static>(mut self, step: O) -> Self {
        self.steps.push(Box::new(step));
        self
    }

    /// Appends a boxed path optimizer step.
    pub fn with_boxed_step(mut self, step: Box<dyn PathOptimizer>) -> Self {
        self.steps.push(step);
        self
    }

    /// Runs all chained optimization steps in sequence over the input path.
    pub fn run(&self, path: &mut InlinePath) {
        for step in &self.steps {
            step.optimize(path);
        }
    }
}

impl PathOptimizer for PathOptimizerHub {
    #[inline]
    fn optimize(&self, path: &mut InlinePath) {
        self.run(path);
    }
}

/// Zero-cost compile-time chained optimizer combining two concrete optimizers.
pub struct ChainedOptimizer<A, B> {
    first: A,
    second: B,
}

impl<A, B> ChainedOptimizer<A, B> {
    pub const fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

impl<A: PathOptimizer, B: PathOptimizer> PathOptimizer for ChainedOptimizer<A, B> {
    #[inline(always)]
    fn optimize(&self, path: &mut InlinePath) {
        self.first.optimize(path);
        self.second.optimize(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PrefixTrimmer;
    impl PathOptimizer for PrefixTrimmer {
        fn optimize(&self, path: &mut InlinePath) {
            if path.as_str().starts_with('/') {
                let mut temp = [0u8; 256];
                let len = path.len() - 1;
                temp[..len].copy_from_slice(&path.buffer[1..path.len()]);
                path.buffer[..len].copy_from_slice(&temp[..len]);
                path.len = len;
            }
        }
    }

    struct SuffixTrimmer;
    impl PathOptimizer for SuffixTrimmer {
        fn optimize(&self, path: &mut InlinePath) {
            let s = path.as_str();
            if let Some(stripped) = s.strip_suffix('/') {
                path.len = stripped.len();
            }
        }
    }

    #[test]
    fn test_hub_chaining() {
        let hub = PathOptimizerHub::new()
            .with_step(PrefixTrimmer)
            .with_step(SuffixTrimmer);

        let mut path = InlinePath::from_raw("/server/settings/");
        hub.optimize(&mut path);
        assert_eq!(path.as_str(), "server/settings");
    }

    #[test]
    fn test_nested_hub_with_step() {
        let inner_hub = PathOptimizerHub::new().with_step(PrefixTrimmer);
        let outer_hub = PathOptimizerHub::new()
            .with_step(inner_hub) // Hub itself is an optimizer step!
            .with_step(SuffixTrimmer);

        let mut path = InlinePath::from_raw("/plugins/admin/");
        outer_hub.optimize(&mut path);
        assert_eq!(path.as_str(), "plugins/admin");
    }
}
