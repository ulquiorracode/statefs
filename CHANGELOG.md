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
- Domain-isolated clean architecture layout (`core/`, `runtime/`, `codecs/`, `adapters/`).
- `StorageBacking` and `RawNode` (`repr(C)`) decoupled memory ports in `statefs-core`.
- `statefs-runtime`: orchestrator with `QueryScenarioResolver` and `stitch-rs` U-cycle integration.
- `statefs-codec-toml`: streaming hierarchical TOML ingestion and encoder.
- `statefs-codec-bin`: zero-copy binary snapshot serializer with header validation.
- `statefs-adapter-opt-cache`: compile-time bounded L1 Direct-Mapped inline path cache.
- `statefs-adapter-opt-simd`: standalone AVX2 / SSE4.2 SIMD path segment scanner via `memchr2`.
- `statefs-adapter-opt-mmap`: zero-copy `memmap2` and `zerocopy` physical storage backing.
- `statefs-adapter-bridge-config`: drop-in high-performance compatibility bridge for `config-rs`.
- `statefs-adapter-compute-rayon`: thread-pool parallel batch query executor via `rayon`.
- `matrix_bench`: multi-candidate unified matrix benchmark grid comparing 6 tiers across scales.
