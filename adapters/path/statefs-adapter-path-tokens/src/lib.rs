//! # StateFS Path Prefix Tokenizer Adapter
//!
//! Replaces recurring path prefixes (e.g. `/server/settings/`, `/plugins/`)
//! with single-byte dictionary tokens to compress keys and accelerate routing.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use statefs_adapter_path_core::{InlinePath, PathOptimizer};

/// Entry in the prefix token dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixTokenEntry {
    pub token: u8,
    pub prefix: String,
}

/// Tokenizer that substitutes frequent path prefixes with compact token markers.
#[derive(Default, Clone, Debug)]
pub struct PrefixTokenizer {
    entries: Vec<PrefixTokenEntry>,
}

impl PrefixTokenizer {
    /// Creates a new empty prefix tokenizer.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Registers a prefix to be substituted by a specific byte token.
    pub fn register_prefix(mut self, token: u8, prefix: &str) -> Self {
        self.entries.push(PrefixTokenEntry {
            token,
            prefix: prefix.into(),
        });
        // Sort by longest prefix first for greedy matching
        self.entries
            .sort_by_key(|a| core::cmp::Reverse(a.prefix.len()));
        self
    }

    /// Attempts to compress an inline path by substituting matching prefixes.
    pub fn compress_inline(&self, path: &mut InlinePath) {
        let s = path.as_str();
        for entry in &self.entries {
            if let Some(remainder) = s.strip_prefix(&entry.prefix) {
                // Token header format: '~' + [token byte] + '/'
                let token_header = [b'~', entry.token, b'/'];
                let needed = token_header.len() + remainder.len();
                if needed <= 256 {
                    let mut temp = [0u8; 256];
                    temp[..token_header.len()].copy_from_slice(&token_header);
                    temp[token_header.len()..needed].copy_from_slice(remainder.as_bytes());
                    if let Ok(new_str) = core::str::from_utf8(&temp[..needed]) {
                        path.set(new_str);
                        return;
                    }
                }
            }
        }
    }

    /// Expands a tokenized inline path back to its full canonical prefix.
    pub fn expand_inline(&self, path: &mut InlinePath) {
        let bytes = path.as_str().as_bytes();
        if bytes.len() >= 3 && bytes[0] == b'~' && bytes[2] == b'/' {
            let token = bytes[1];
            for entry in &self.entries {
                if entry.token == token {
                    let remainder = &bytes[3..];
                    let prefix_bytes = entry.prefix.as_bytes();
                    let needed = prefix_bytes.len() + remainder.len();
                    if needed <= 256 {
                        let mut temp = [0u8; 256];
                        temp[..prefix_bytes.len()].copy_from_slice(prefix_bytes);
                        temp[prefix_bytes.len()..needed].copy_from_slice(remainder);
                        if let Ok(new_str) = core::str::from_utf8(&temp[..needed]) {
                            path.set(new_str);
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl PathOptimizer for PrefixTokenizer {
    #[inline]
    fn optimize(&self, path: &mut InlinePath) {
        self.compress_inline(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_tokenization_roundtrip() {
        let tokenizer = PrefixTokenizer::new()
            .register_prefix(b'S', "server/settings/")
            .register_prefix(b'P', "plugins/moderation/");

        let mut path = InlinePath::from_raw("server/settings/tickrate");
        tokenizer.compress_inline(&mut path);
        assert_eq!(path.as_str(), "~S/tickrate");

        tokenizer.expand_inline(&mut path);
        assert_eq!(path.as_str(), "server/settings/tickrate");
    }

    #[test]
    fn test_unmatched_prefix_leaves_input_intact() {
        let tokenizer = PrefixTokenizer::new().register_prefix(b'S', "server/settings/");
        let mut path = InlinePath::from_raw("client/graphics/fov");
        tokenizer.compress_inline(&mut path);
        assert_eq!(path.as_str(), "client/graphics/fov");
    }
}
