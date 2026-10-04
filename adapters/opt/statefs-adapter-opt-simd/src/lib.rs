//! # StateFS SIMD Path Scanner Adapter
//!
//! Accelerated separator finding (`/` and `\`) using SIMD vector instructions via `memchr`.
//!
//! Provides zero-allocation path segmentation with sub-nanosecond per-delimiter scanning.

#![cfg_attr(not(feature = "std"), no_std)]

/// Adapter providing SIMD-accelerated path segment scanning.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimdPathScanner;

impl SimdPathScanner {
    /// Creates a new SIMD path scanner adapter.
    pub const fn new() -> Self {
        Self
    }

    /// Iterates through path segments using SIMD instructions.
    #[inline(always)]
    pub fn for_each_segment<'a, F>(&self, path: &'a str, mut callback: F) -> bool
    where
        F: FnMut(&'a str) -> bool,
    {
        let trimmed = path.trim();
        let bytes = trimmed.as_bytes();

        #[cfg(feature = "simd")]
        {
            let mut start = 0;
            for pos in memchr::memchr2_iter(b'/', b'\\', bytes) {
                if pos > start {
                    // SAFETY: valid utf-8 substring from valid &str
                    let seg = unsafe { core::str::from_utf8_unchecked(&bytes[start..pos]) }.trim();
                    if !seg.is_empty() && !callback(seg) {
                        return false;
                    }
                }
                start = pos + 1;
            }

            if start < bytes.len() {
                let seg = unsafe { core::str::from_utf8_unchecked(&bytes[start..]) }.trim();
                if !seg.is_empty() && !callback(seg) {
                    return false;
                }
            }
            true
        }

        #[cfg(not(feature = "simd"))]
        {
            for seg in trimmed
                .split(['/', '\\'])
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                if !callback(seg) {
                    return false;
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_scanner_segments() {
        let scanner = SimdPathScanner::new();
        let mut segments = alloc::vec::Vec::new();
        extern crate alloc;

        scanner.for_each_segment("/server/settings/tickrate", |seg| {
            segments.push(seg);
            true
        });

        assert_eq!(segments, &["server", "settings", "tickrate"]);
    }
}
