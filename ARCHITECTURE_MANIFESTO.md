# StateFS: Universal Hierarchical State & Virtual Document System
## Architectural Manifesto & Design Specification

> **Status:** Proposal / Incubation  
> **Author:** Architecture Working Group  
> **Target Scope:** Autonomous Open-Source Organization & Crate Ecosystem  
> **Validated Name Candidates:** `statefs`, `treevfs`, `polyvfs`, `triefs` (Free on crates.io & GitHub Organizations)

---

## 1. Executive Summary & Vision

Modern software architectures suffer from fragmented state management. Applications juggle disk-based configuration files (TOML, YAML, JSON), runtime settings, memory stores, database entities, metrics, and localization strings using completely disjointed APIs and paradigms.

**StateFS** is a universal, microkernel-based **Hierarchical Structured State and Virtual Document Graph**. It elevates the Unix principle *"Everything is a File"* into modern typed systems programming: **"Every piece of state is an addressable node in a reactive virtual tree."**

StateFS is **domain-agnostic**. It knows nothing about game engines, web frameworks, or specific storage media. It provides a pure mathematical abstraction of a hierarchical document graph with pluggable mounts, reactive delta observers, codecs, and bidirectional lenses.

---

## 2. Core Design Principles

1. **Nanokernel Purity (`no_std` Capable):**
   The core crate (`statefs-core`) contains zero heavy dependencies, zero file I/O, zero network sockets, and zero thread-pool runtimes. It is a deterministic in-memory trie data structure with \(O(k)\) path lookups.
2. **Composition over Inheritance (`has-a` Aggregation):**
   All higher-level functionality (transactions, change-watching, schema validation, OS file synchronization) is implemented as layered decorators wrapping the core store.
3. **Decoupled Physical Mounts (VFS Pattern):**
   Nodes do not know where they reside physically. A logical path (`/plugins/moderation/cvars/ban_time`) is dynamically routed via a Mount Table to an in-memory node, a TOML file on disk, or an SQLite database.
4. **Zero-Cost Codec & Driver Isolation:**
   If a consumer only needs TOML serialization, YAML and JSON codecs are never compiled into the binary.
5. **Bidirectional Lenses (Projections):**
   External environments (e.g. GoldSrc/Source console CVARs, REST APIs, i18n catalogs) are modeled as lenses projecting slices of the virtual tree into external domains.

---

## 3. Layered Onion Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│ Layer 4: Domain Bridges & Lenses                                       │
│ • CvarLens (Console Variable adapter for HLDS/Source)                  │
│ • I18nLens (Multi-dimensional language selector: /i18n/{lang}/*)       │
│ • RestLens / WebSocket Streaming / CLI Query Tool                      │
└──────────────────────────────────▲─────────────────────────────────────┘
                                   │
┌──────────────────────────────────┴─────────────────────────────────────┐
│ Layer 3: Virtual File System & Codecs (VFS / Mounts)                   │
│ • MountRegistry (Virtual path routing table)                           │
│ • Codecs: TOML, JSON, YAML, FlexBuffers, Bincode                       │
│ • Drivers: FsDriver (OS disk), MemoryDriver, SqliteDriver             │
└──────────────────────────────────▲─────────────────────────────────────┘
                                   │
┌──────────────────────────────────┴─────────────────────────────────────┐
│ Layer 2: Behavioral Decorators (Extensions / Middlewares)              │
│ • ReactiveTree (Event stream, change notifications, glob matching)     │
│ • TransactionalTree (ACID, Copy-on-Write snapshots, rollback)          │
│ • ValidatedTree (Schema constraints, range bounds, type invariants)    │
│ • WatcherSync (OS file-watcher integration via inotify / kqueue)       │
└──────────────────────────────────▲─────────────────────────────────────┘
                                   │
┌──────────────────────────────────┴─────────────────────────────────────┐
│ Layer 1: NANOKERNEL (statefs-core)                                     │
│ • Path: Segmented canonical path (`a/b/c`)                             │
│ • Value: Strongly typed scalar, array, map, raw bytes                  │
│ • Node: Data container with revision/version counter                   │
│ • Store: Primitive memory trie (get, insert, remove, iter)             │
│ • Zero dependencies, no_std compatible, zero allocation in hot reads   │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Nanokernel Specification (`statefs-core`)

### 4.1 Canonical Path (`Path`)
Paths are sequence of UTF-8 string segments, immune to OS-specific separators (`/` vs `\`):
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Path(Vec<String>);

impl Path {
    pub fn parse(raw: &str) -> Self;
    pub fn join(&self, sub: &str) -> Self;
    pub fn parent(&self) -> Option<Path>;
    pub fn segments(&self) -> &[String];
}
```

### 4.2 Generic Value (`Value`)
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Map(HashMap<String, Value>),
}
```

### 4.3 Node & Store Contract
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub value: Value,
    pub revision: u64,
}

pub trait Store {
    fn get(&self, path: &Path) -> Option<&Node>;
    fn insert(&mut self, path: &Path, value: Value) -> Result<Option<Node>, StoreError>;
    fn remove(&mut self, path: &Path) -> Result<Option<Node>, StoreError>;
    fn list_children(&self, prefix: &Path) -> Vec<Path>;
}
```

---

## 5. Extensibility & Middleware Models

### 5.1 Reactive Layer (`statefs-reactive`)
Decorates any `Store` with an event bus emitting fine-grained mutation diffs:
```rust
#[derive(Debug, Clone)]
pub enum MutationEvent {
    Created { path: Path, new: Value },
    Updated { path: Path, old: Value, new: Value },
    Deleted { path: Path, old: Value },
}

pub trait EventBus: Send + Sync {
    fn publish(&self, event: MutationEvent);
}

pub struct ReactiveStore<S: Store, B: EventBus> {
    inner: S,
    bus: B,
}
```

### 5.2 Transactional Layer (`statefs-tx`)
Provides ACID-like commit/rollback functionality using Copy-on-Write (CoW) shadow tries:
```rust
pub struct Transaction<'a, S: Store> {
    base: &'a S,
    staging: HashMap<Path, Option<Value>>,
}

impl<'a, S: Store> Transaction<'a, S> {
    pub fn set(&mut self, path: Path, val: Value);
    pub fn commit(self, target: &mut S) -> Result<ChangeSet, TxError>;
    pub fn rollback(self);
}
```

---

## 6. VFS Mounts & Codecs (`statefs-vfs`)

### 6.1 Codec Contract
```rust
pub trait Codec: Send + Sync {
    fn encode(&self, value: &Value) -> Result<Vec<u8>, CodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<Value, CodecError>;
}
```

### 6.2 Driver Contract
```rust
pub trait Driver: Send + Sync {
    fn load_tree(&self) -> Result<HashMap<Path, Value>, DriverError>;
    fn flush_tree(&mut self, changes: &[(Path, Value)]) -> Result<(), DriverError>;
}
```

### 6.3 Mount Table
The Mount Table intercepts `Path` operations and delegates execution to the corresponding physical driver:
```rust
let mut vfs = VfsTree::new();

// Mount configs as TOML files
vfs.mount(Path::parse("configs/plugins"), FsDriver::new("data/configs", TomlCodec));

// Mount volatile session states in RAM
vfs.mount(Path::parse("runtime/sessions"), MemoryDriver::new());

// Mount database-backed persistence
vfs.mount(Path::parse("database/accounts"), SqliteDriver::new("data/auth.db"));
```

---

## 7. Package Taxonomy (`proj-group-member`)

The organization workspace follows strict modular packaging:

| Package Name | Category | Description |
|:---|:---|:---|
| **`statefs-core`** | Core | Pure nanokernel trie, Path, Value, Node, Store trait (`no_std`) |
| **`statefs-reactive`** | Extension | Event bus, subscriptions, path globbing, diff stream |
| **`statefs-tx`** | Extension | ACID transactions, CoW snapshots, atomic commit/rollback |
| **`statefs-schema`** | Extension | Validation constraints, bounds, invariants, schema contracts |
| **`statefs-vfs`** | VFS | Mount table, virtual path router, driver aggregation |
| **`statefs-codec-toml`** | Codec | TOML encoding / decoding integration |
| **`statefs-codec-json`** | Codec | JSON / JSON5 encoding / decoding integration |
| **`statefs-codec-yaml`** | Codec | YAML encoding / decoding integration |
| **`statefs-codec-bin`** | Codec | High-performance binary codec (Bincode / FlexBuffers) |
| **`statefs-driver-fs`** | Driver | OS File system driver with disk sync |
| **`statefs-driver-notify`** | Driver | Native OS file-watcher sync (inotify, kqueue, IOCP) |
| **`statefs-driver-sqlite`**| Driver | SQLite backend storage driver |
| **`statefs`** | Facade | "Batteries-included" umbrella crate with selective feature flags |

---

## 8. Verified Name Candidates & Availability

Validation performed on **crates.io** and **GitHub API** on October 2, 2026:

| Candidate | crates.io Status | GitHub Org Status | Semantic Alignment |
|:---|:---:|:---:|:---|
| **`statefs`** | **FREE** | **FREE** | **Exceptional** (State File System; matches Linux sysfs/procfs philosophy) |
| **`treevfs`** | **FREE** | **FREE** | **High** (Tree Virtual File System; direct and descriptive) |
| **`statepath`** | **FREE** | **FREE** | **High** (Addresses state addressing, path-based access) |
| **`polyvfs`** | **FREE** | **FREE** | **Medium** (Polymorphic VFS; emphasizes multiple backends) |
| **`triefs`** | **FREE** | **FREE** | **Medium** (Trie-based File System; algorithmically focused) |

**Recommendation:** Adopt **`statefs`** (Organization: `statefs-rs` or `statefs`). It is succinct, industrial, and accurately captures the paradigm.

---

## 9. Integration with GoldSrc.rs

Inside the `goldsrc-rs` repository, StateFS is consumed as an external dependency:

```toml
[dependencies]
statefs = { version = "0.1", features = ["reactive", "vfs", "toml", "fs-notify"] }
```

A dedicated bridge crate (`goldsrc-configfs` or `goldsrc-tree-bridge`) provides engine-specific lenses:
- **`CvarLens`**: Projects `/plugins/*/cvars/*` nodes into engine `cvar_t` handles with bidirectional change propagation.
- **`I18nLens`**: Maps parameterized language dimensions (`/i18n/{lang}/*`) to connected player locales (`Player.lang()`).
- **`GrsExec`**: Dispatches structured TOML/JSON presets through the StateFS Mount Router.
