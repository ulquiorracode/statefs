//! # SIMD Path Scanner Adapter
//!
//! Accelerated separator finding (`/` and `\`) using SIMD vector instructions via `memchr`.
//!
//! Provides zero-allocation path segmentation with sub-nanosecond per-delimiter scanning.

use crate::passport::{AdapterContract, Passport, Visa, WorkloadScenario};

/// Adapter providing SIMD-accelerated path segment scanning.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimdScanner {
    passport: Passport,
}

impl SimdScanner {
    /// Creates a new SIMD path scanner adapter.
    pub const fn new() -> Self {
        Self {
            passport: Passport::new(
                "SimdScanner",
                4096,  // Capable of streaming large paths
                false, // Works on mutable or immutable data
                0,     // Always beneficial for deep paths
            ),
        }
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

impl AdapterContract for SimdScanner {
    type Key = str;
    type Output = ();

    #[inline(always)]
    fn passport(&self) -> &Passport {
        &self.passport
    }

    #[inline(always)]
    fn evaluate_visa(&self, _scenario: WorkloadScenario, key: &Self::Key) -> Visa {
        if key.len() > self.passport.max_key_len {
            return Visa::Rejected("Path length exceeds scanner maximum limit");
        }
        Visa::Admitted
    }
}
