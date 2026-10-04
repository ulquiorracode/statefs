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
- `statefs-adapter-opt-lockfree`: lock-free SPSC continuous BipBuffer WAL mutation stream adapter via `bbqueue`.
- `statefs-adapter-bridge-config`: drop-in high-performance compatibility bridge for `config-rs`.
- `statefs-codec-json`: streaming hierarchical JSON ingestion and serializer for symmetric interoperability.
- `statefs-adapter-path-core`: `PathOptimizer` trait port, stack-allocated `InlinePath`, and fractal `PathOptimizerHub` allowing arbitrary nested step composition.
- `statefs-adapter-path-tokens`: dictionary prefix compressor and expander (`PrefixTokenizer`) for high-frequency path prefixes.
- `statefs-adapter-path-handles`: direct-mapped index cache (`PathHandleCache`) bypassing string parsing in hot loops.
- `PathHandle(pub u32)` for direct O(1) index access to arena nodes in 2-3 ns.
- Subtree revision tracking (`subtree_revision: u64`) with cascading updates and zero-cost O(1) `SubtreeWatcher`.
- Glob pattern matching (`find_glob`) with support for single `*` and recursive `**` wildcards.
- Production runnable examples under `runtime/statefs-runtime/examples/`: `game_server_cvars`, `zero_copy_snapshot`, and `reactive_watch`.
- `statefs-vfs-core`: `VfsProvider` trait port, longest-prefix `VfsMountHub`, and layered `VfsOverlay` search paths.
- `statefs-vfs-disk`: sandboxed physical directory mount provider with directory traversal attack prevention.
- `statefs-vfs-package`: in-memory and zero-copy continuous package container provider (WAD3 / PAK / FlatArchive).
- `matrix_bench`: multi-candidate unified matrix benchmark grid comparing 8 tiers across scales and measuring cold boot / WAL throughput / PathHandle direct lookup.
