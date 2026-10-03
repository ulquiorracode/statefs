# StateFS

[![CI](https://github.com/statefs-rs/statefs/actions/workflows/ci.yml/badge.svg)](https://github.com/statefs-rs/statefs/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Language-Rust%202024-orange.svg)](https://www.rust-lang.org/)
[![no_std](https://img.shields.io/badge/no__std-compatible-brightgreen.svg)](#nanokernel-purity)

**StateFS** is an ultra-fast, zero-allocation hierarchical structured state & virtual configuration engine written in Rust.

It elevates the Unix philosophy *"Everything is a File"* into high-performance typed systems programming: **every piece of application state, configuration, localization, and telemetry is an addressable node in a dense, reactive virtual tree**.

---

## ⚡ Performance: Real-World Enterprise Benchmark

StateFS was benchmarked against the industry-standard [`config-rs`](https://github.com/mehcode/config-rs) under a **real production workload** using configurations from a live dedicated server (deeply nested configurations, localization dictionaries, plugin bundles, and reactive server rules):

```text
=================================================================================================
                                  REAL SERVER PRODUCTION BENCHMARK
=================================================================================================
  Metric / Scenario             config-rs (Industry Standard)   StateFS (Arena + SIMD)   Speedup
-------------------------------------------------------------------------------------------------
  Single Key Query              1,028.40 ns (1.02 µs)           89.95 ns                 11.4x faster 🔥
  Multi-Query Batch             3,330.70 ns (3.33 µs)           233.75 ns                14.2x faster ⚡
  Full Server Boot Ingestion    349.02 µs                       232.36 µs                1.5x faster
  Resident Heap RAM             38.88 KB (39,811 bytes)         27.60 KB (28,260 bytes)  -29% RAM 💾
=================================================================================================
```

*Measurements conducted with Criterion (100 samples, 95% confidence intervals) using global `Cap` memory tracking.*

---

## 💎 Key Highlights

- **Sub-90ns Lookups**: Benchmarked at **89.95 ns** for deep path queries (`/server/security/rate_limiter/max_requests_per_sec`).
- **Dense Arena Trie**: Nodes are stored in a contiguous `Vec<ArenaNode>` using 32-bit indices (`u32`) instead of pointer-heavy `Box` or recursive maps.
- **StringPool Interning**: Redundant path segments (`"plugins"`, `"moderation"`, `"cvars"`) are deduplicated into a single continuous buffer.
- **Zero-Allocation Hot Path**: Lookup by raw string (`get_str`) requires **0 bytes of heap allocation**.
- **SIMD Path Segmentation**: Accelerated separator scanning using `memchr2` (AVX2 / SSE4.2 / NEON).
- **Nanokernel Purity (`no_std`)**: Core data structures require zero OS dependencies, zero file I/O, and zero background threads.

---

## 🏗️ Architecture

StateFS is architected around the **Dense Left-Child / Right-Sibling Arena**:

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

## 🚀 Quickstart

Add `statefs-core` to your `Cargo.toml`:

```toml
[dependencies]
statefs-core = "0.1"
```

### Basic Usage

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

## 📦 Workspace Layout

- **[`crates/statefs-core`](crates/statefs-core)**: Pure `no_std` nanokernel (Arena Trie, StringPool, Path, Value, Store).
- **[`crates/statefs-codec-toml`](crates/statefs-codec-toml)** *(Planned)*: Native zero-copy TOML serialization & streaming deserialization.
- **[`crates/statefs-codec-bin`](crates/statefs-codec-bin)** *(Planned)*: Flat binary memory-mapped (`memmap2`) snapshot codec.

---

## 📜 License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
at your option.
