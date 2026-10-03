# Changelog

All notable changes to StateFS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- First public release of `statefs-core` nanokernel.
- Dense Left-Child / Right-Sibling `ArenaNode` prefix tree architecture over 32-bit indices.
- Global string deduplication (`StringPool`) with zero heap duplication for recurring keys.
- SIMD-accelerated path segment scanning via `memchr2` (AVX2, SSE4.2, NEON).
- Configurable path validation policies (`PathOptions`: Default, DotNotation, Permissive).
- Zero-allocation hot path query `MemStore::get_str` avoiding path struct allocations.
- Real-world production server benchmarks comparing StateFS vs `config-rs`.
- Portable benchmark test fixtures under `benches/fixtures`.
- Automated GitHub Actions CI workflow covering test suite, clippy, and code formatting.
- `statefs-opt` crate: modular optimization adapters, passports, and scenario resolvers.
- `Passport` & `Visa` admission control contracts decouples optimization adapters from core storage.
- `L1PathCache<const CAP: usize>`: inline compile-time bounded direct-mapped L1 cache.
- `SimdScanner`: vector-accelerated path segment scanning.
- `QueryScenarioResolver`: first-level policy regulator directing queries according to task scenarios (`BootLoading`, `SteadyStateLoop`).
- `stitch-rs` integration bridge: `StateFsTerminal` and monomorphic pipeline execution.
- Multi-candidate unified matrix benchmark (`matrix_bench`) comparing 5 optimization tiers across scales.
