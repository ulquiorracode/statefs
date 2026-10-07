# StateFS

[![CI](https://github.com/ulquiorracode/statefs/actions/workflows/ci.yml/badge.svg)](https://github.com/ulquiorracode/statefs/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Language-Rust%202024-orange.svg)](https://www.rust-lang.org/)
[![no_std](https://img.shields.io/badge/no__std-compatible-brightgreen.svg)](#nanokernel-purity)

StateFS is an ultra-fast, zero-allocation hierarchical structured state and virtual configuration engine written in Rust.

It elevates the Unix philosophy *"Everything is a File"* into high-performance typed systems programming: every piece of application state, configuration, localization, and telemetry is an addressable node in a dense, reactive virtual tree.

---

## Performance: Real-World Enterprise Benchmark

StateFS was benchmarked against the industry-standard [`config-rs`](https://github.com/mehcode/config-rs) under a real production workload using configurations from a live dedicated server (deeply nested configurations, localization dictionaries, plugin bundles, and reactive server rules):

```text
=================================================================================================
                                  REAL SERVER PRODUCTION BENCHMARK
=================================================================================================
  Metric / Scenario             config-rs (Industry Standard)   StateFS (Optimized Profile)   Speedup
-------------------------------------------------------------------------------------------------
  Single Key Query (Hot Loop)   894.57 ns                       39.76 ns (L1 Cache)           22.5x faster
  Lock-Free WAL Mutation Push   --                              12.34 ns (bbqueue SPSC)       81.04 M ops/sec
  Cold Boot (Disk / Snapshot)   611.00 µs (TOML parse)          41.10 µs (Zero-Copy Mmap)     14.9x faster
  Multi-Query Batch             3,330.70 ns (3.33 µs)           233.75 ns                     14.2x faster
  Full TOML Ingestion           349.02 µs                       232.36 µs                     1.5x faster
  Resident Heap RAM             38.88 KB (39,811 bytes)         27.60 KB (28,260 bytes)       -29% RAM
=================================================================================================
```

*Measurements conducted with Criterion (100 samples, 95% confidence intervals) using global `Cap` memory tracking.*

---

## Key Highlights

- **Sub-90ns Lookups**: Benchmarked at 89.95 ns for deep path queries (`/server/security/rate_limiter/max_requests_per_sec`).
- **Dense Arena Trie**: Nodes are stored in a contiguous `Vec<ArenaNode>` using 32-bit indices (`u32`) instead of pointer-heavy `Box` or recursive maps.
- **StringPool Interning**: Redundant path segments (`"plugins"`, `"moderation"`, `"cvars"`) are deduplicated into a single continuous buffer.
- **Zero-Allocation Hot Path**: Lookup by raw string (`get_str`) requires 0 bytes of heap allocation.
- **SIMD Path Segmentation**: Accelerated separator scanning using `memchr2` (AVX2 / SSE4.2 / NEON).
- **Nanokernel Purity (`no_std`)**: Core data structures require zero OS dependencies, zero file I/O, and zero background threads.

---

## Architecture

StateFS is architected around the Dense Left-Child / Right-Sibling Arena:

```text
               ┌────────────────────────────────────────────────────────┐
               │                      MemStore                          │
               ├────────────────────────────────────────────────────────┤
               │  Arena: Vec<ArenaNode>                                 │
               │  ┌───────────────┐ ┌───────────────┐ ┌──────────────┐  │
               │  │ [0] Root Node │ │ [1] "plugins" │ │ [2] "server" │  │
               │  └───────┬───────┘ └───────▲───────┘ └──────▲───────┘  │
               │          │ first_child     │                │          │
               │          └─────────────────┴──next_sibling──┘          │
               ├────────────────────────────────────────────────────────┤
               │  StringPool: Interned byte spans (0 redundant strings) │
               └────────────────────────────────────────────────────────┘
```

---

## Quickstart

Add `statefs-runtime` to your `Cargo.toml`:

```toml
[dependencies]
statefs-runtime = "0.1"
```

### High-Level Fluent Composition (Two Lines of Code)

```rust
use statefs_runtime::StateFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Fluent multi-source ingestion in two lines
    let store = StateFs::builder()
        .with_toml_str("[server]\ntickrate = 128\nmotd = 'Welcome!'")?
        .with_env("APP")?
        .build()?;

    // 2. Zero-allocation hot-path query by raw string
    let tickrate = store.get_str("/server/tickrate")
        .and_then(|node| node.value.as_int());
    assert_eq!(tickrate, Some(128));

    Ok(())
}
```

### Strongly-Typed Serde Struct Extraction

```rust
use serde::Deserialize;
use statefs_runtime::StateFs;

#[derive(Debug, Deserialize, PartialEq)]
struct ServerCfg {
    tickrate: u32,
    hostname: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = StateFs::builder()
        .with_toml_str("[server]\ntickrate = 128\nhostname = 'HLDS Pro'")?;

    // Directly extract typed struct from virtual subtree
    let cfg: ServerCfg = builder.extract("/server")?;
    assert_eq!(cfg.tickrate, 128);
    Ok(())
}
```

### Live HTTP / Axum Telemetry & Inspector Bridge

Serve live state and allow remote updates over HTTP with atomic revision tracking (`X-StateFS-Revision`):

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use axum::Router;
use statefs_core::MemStore;
use statefs_adapter_bridge_http::statefs_router;

#[tokio::main]
async fn main() {
    let store = Arc::new(RwLock::new(MemStore::new()));
    let app = Router::new().nest("/_state", statefs_router(store));
    // GET    /_state/server/tickrate -> 128
    // POST   /_state/server/tickrate -> updates subtree revision
    // DELETE /_state/server/tickrate -> deletes node
}
```

### Universal C-ABI FFI Bridge

StateFS exports a zero-panic C ABI (`include/statefs.h`) with buffer size probing:

```c
#include "statefs.h"
#include <stdio.h>

int main() {
    StateFsStore* store = statefs_store_new();
    statefs_store_insert_int(store, "/server/tickrate", 128);

    int64_t tickrate = 0;
    if (statefs_store_get_int(store, "/server/tickrate", &tickrate) == 1) {
        printf("Tickrate: %lld\n", tickrate);
    }
    statefs_store_free(store);
    return 0;
}
```

### Low-Level Nanokernel Usage (`no_std`)

```rust
use statefs_core::{MemStore, Path, Store, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = MemStore::new();

    // 1. Insert hierarchical configuration
    store.insert(&Path::parse("/server/tickrate"), Value::from(128))?;
    store.insert(&Path::parse("/server/motd"), Value::from("Welcome!"))?;
    store.insert(&Path::parse("/plugins/moderation/enabled"), Value::from(true))?;

    // 2. Zero-allocation hot-path query by raw string
    let tickrate = store.get_str("/server/tickrate")
        .and_then(|node| node.value.as_int());
    assert_eq!(tickrate, Some(128));

    // 3. Hierarchical listing (O(k) traversal)
    let children = store.list_children(&Path::parse("/server"));
    assert_eq!(children.len(), 2);

    Ok(())
}
```

---

## Workspace Layout

The StateFS ecosystem is structured around decoupled, zero-cost modular domains:

- **Core (`core/`)**:
  - **[`statefs-core`](core/statefs-core)**: Pure `no_std` nanokernel (Arena Trie, StringPool, Path, Value, Storage Ports, Subtree Revision, `SubtreeWatcher`, Glob matching).
- **Runtime (`runtime/`)**:
  - **[`statefs-runtime`](runtime/statefs-runtime)**: Orchestration engine, fluent `StateFs::builder()`, scenario resolvers, `stitch-rs` U-cycle pipeline integration, and production examples.
- **Codecs (`codecs/`)**:
  - **[`statefs-codec-toml`](codecs/statefs-codec-toml)**: Streaming TOML ingestion and tree serializer.
  - **[`statefs-codec-json`](codecs/statefs-codec-json)**: Streaming JSON ingestion and tree exporter.
  - **[`statefs-codec-bin`](codecs/statefs-codec-bin)**: Flat binary image exporter for instant disk restoration with 32-byte header alignment.
- **Path Optimization Adapters (`adapters/path/`)**:
  - **[`statefs-adapter-path-core`](adapters/path/statefs-adapter-path-core)**: `PathOptimizer` trait port, zero-alloc `InlinePath`, and recursive fractal `PathOptimizerHub`.
  - **[`statefs-adapter-path-tokens`](adapters/path/statefs-adapter-path-tokens)**: High-frequency prefix compression dictionary (`PrefixTokenizer`).
  - **[`statefs-adapter-path-handles`](adapters/path/statefs-adapter-path-handles)**: Direct-mapped direct arena index cache (`PathHandleCache`) bypassing string parsing with epoch-based cache invalidation.
- **Optimization Adapters (`adapters/opt/`)**:
  - **[`statefs-adapter-opt-cache`](adapters/opt/statefs-adapter-opt-cache)**: Compile-time bounded L1 Direct-Mapped inline path cache with epoch staleness protection.
  - **[`statefs-adapter-opt-simd`](adapters/opt/statefs-adapter-opt-simd)**: AVX2 / SSE4.2 SIMD path segment scanner via `memchr2`.
  - **[`statefs-adapter-opt-mmap`](adapters/opt/statefs-adapter-opt-mmap)**: Zero-copy `memmap2` + `zerocopy` physical storage backing with version and boundary verification.
  - **[`statefs-adapter-opt-lockfree`](adapters/opt/statefs-adapter-opt-lockfree)**: Lock-free SPSC continuous BipBuffer WAL mutation stream adapter via `bbqueue`.
- **Bridges (`adapters/bridge/`)**:
  - **[`statefs-adapter-bridge-serde`](adapters/bridge/statefs-adapter-bridge-serde)**: Zero-copy/direct Serde deserializer extracting arbitrary strongly-typed structs from virtual subtrees.
  - **[`statefs-adapter-bridge-http`](adapters/bridge/statefs-adapter-bridge-http)**: High-performance Axum 0.8 HTTP REST API & telemetry inspector with atomic `X-StateFS-Revision`.
  - **[`statefs-adapter-bridge-env`](adapters/bridge/statefs-adapter-bridge-env)**: Automated environment variable ingestion with prefix filtering and scalar type inference.
  - **[`statefs-adapter-c`](adapters/bridge/statefs-adapter-c)**: Universal C-ABI dynamic and static library bridge with C header (`include/statefs.h`) and panic barriers.
  - **[`statefs-adapter-bridge-config`](adapters/bridge/statefs-adapter-bridge-config)**: Drop-in compatibility wrapper for code using `config-rs`.
- **Compute Adapters (`adapters/compute/`)**:
  - **[`statefs-adapter-compute-rayon`](adapters/compute/statefs-adapter-compute-rayon)**: Thread-pool parallel batch queries via `rayon`.
- **VFS Domain (`vfs/`)**:
  - **[`statefs-vfs-core`](vfs/statefs-vfs-core)**: `VfsProvider` trait port, longest-prefix `VfsMountHub`, and layered `VfsOverlay` search paths.
  - **[`statefs-vfs-disk`](vfs/statefs-vfs-disk)**: Sandboxed physical directory mount provider with directory traversal attack prevention.
  - **[`statefs-vfs-package`](vfs/statefs-vfs-package)**: In-memory and zero-copy continuous package container provider (WAD3 / PAK / FlatArchive).
- **Benchmarks (`benches/`)**:
  - **[`matrix_bench`](benches/matrix_bench)**: Multi-candidate benchmark grid comparing all optimization tiers.

---

## Runnable Production Examples

StateFS includes complete, real-world runnable examples under [`runtime/statefs-runtime/examples/`](runtime/statefs-runtime/examples):

- **`game_server_cvars`**: High-performance game server engine state registry with `PathHandle` lookups, `SubtreeWatcher` for physics tick invalidation, and lock-free SPSC WAL replication.
  ```bash
  cargo run -p statefs-runtime --example game_server_cvars
  ```
- **`frame_pipeline`**: Full `Host_Frame` U-cycle pipeline powered by `stitch-rs` with delta-time clamping & security middleware, telemetry profiler, and O(1) direct `PathHandle` CVAR execution without per-frame path allocations.
  ```bash
  cargo run -p statefs-runtime --example frame_pipeline
  ```
- **`zero_copy_snapshot`**: Instant cold boot via flat binary snapshot export and zero-copy `mmap` backing in under 50 microseconds with zero heap allocation.
  ```bash
  cargo run -p statefs-runtime --example zero_copy_snapshot
  ```
- **`reactive_watch`**: Subtree revision tracking and glob-based search (`*` and `**`) across virtual configuration hierarchies.
  ```bash
  cargo run -p statefs-runtime --example reactive_watch
  ```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
at your option.
