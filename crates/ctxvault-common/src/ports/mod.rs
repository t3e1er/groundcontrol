//! Ports for the ctxvault hexagonal (ports-and-adapters) architecture.
//!
//! # Hexagonal intent
//!
//! ctxvault is organized as **ports and adapters**. A *port* is a contract —
//! a trait — that expresses a capability the domain needs (metadata catalog,
//! full-text index, vector store, graph store, embedding provider, search
//! dispatch). An *adapter* is a concrete backend that satisfies a port
//! (Tantivy behind the text-index port, HNSW behind the vector port, SQLite
//! behind the catalog port, and so on).
//!
//! Ports are defined **low** — as close to the shared domain as possible — so
//! that consumers depend on the contract, never on a concrete backend. Adapters
//! live in `ctxvault-core`, implement these ports, and keep their backend types
//! (`rusqlite::Connection`, `tantivy::*`, `hnsw_rs::*`, `petgraph::*`, `ort::*`)
//! encapsulated: a backend type must never leak across a port boundary. The
//! **composition root** in `ctxvault-cli` is the only place that names concrete
//! adapters and injects them.
//!
//! This module is the home for the dependency-light port traits. Keeping this
//! module free of heavy dependencies preserves the crate layering
//! (`common` ← `core` ← `mcp` ← `cli`): adding an infrastructure crate here
//! would force it on every consumer.
//!
//! # Generic-vs-trait-object decision (per port)
//!
//! How a port is *wired* (monomorphized generic bound vs. `dyn` trait object)
//! is a consumption decision recorded here so later wiring tasks stay
//! consistent. It does **not** change how anything is stored today; this block
//! is a decision record, not a wiring change.
//!
//! Verified ownership: `Engine` owns each backend **by value**
//! (`graph: KnowledgeGraph`, `catalog: Store`, `text_index: BM25Index`,
//! `vectors: VectorIndex`, `embedder: Embedder`), reached only through `&` /
//! `&mut`. There is no `Arc<dyn …>`, and no runtime backend swap anywhere (the
//! only clone is internal to a `save`). Nothing requires dynamic dispatch.
//!
//! Therefore:
//!
//! - **Hot-path ports → generics with trait bounds.** `MetadataCatalog`,
//!   `TextIndex`, `VectorStore`, `GraphStore`, and `EmbeddingProvider` sit on
//!   the retrieval/index hot path and have exactly one concrete adapter each,
//!   owned by value. They will be wired as **generic type parameters with trait
//!   bounds** (monomorphized, zero-cost, statically dispatched) — never as
//!   `Arc<dyn _>` / `Box<dyn _>`.
//! - **Swap-point ports → trait objects.** `dyn` is reserved for a genuine
//!   runtime-swap seam (plugin-style backend chosen at run time). **No port in
//!   this refactor's scope needs `dyn` today** — there is no such seam.
//!   `SearchService` is the only candidate that might warrant evaluation (its
//!   mode dispatch could become a swap point); until a real runtime swap exists,
//!   it too stays generic.
//!
//! In short: **hot-path = generic; swap-point = dyn; none currently need
//! `dyn`.**
//!
//! # Port-home decision
//!
//! Every consumed surface of the six ports is already **domain-typed** — the
//! methods that callers actually use return [`crate::types`] domain types
//! (`SearchResult`, `Edge`, `Document`, `CodeSymbol`, `Vec<f32>`, …), not the
//! backends' own types. The concrete infrastructure types stay private inside
//! their adapters. Because the contracts do not require any heavy crate, **all
//! six port traits live here in `ctxvault-common::ports`**:
//!
//! - **`MetadataCatalog`** — in `ctxvault-common`. The SQLite `Store`'s public
//!   surface exchanges domain records (`FileRecord`, `ChunkRecord`,
//!   `EdgeTypeRecord`, `CodeSymbol`, config/state values); `rusqlite::Connection`
//!   never appears in a public signature.
//! - **`TextIndex`** — in `ctxvault-common`. The Tantivy `BM25Index`'s used
//!   surface takes `Chunk`/query strings and returns
//!   [`crate::types::SearchResult`], threading a [`crate::types::Modality`]
//!   filter; `tantivy::*` types stay internal.
//! - **`VectorStore`** — in `ctxvault-common`. The HNSW index's used surface
//!   exchanges plain `Vec<f32>` vectors and a [`crate::types::Modality`] filter;
//!   `hnsw_rs::*` stays internal. Its result/metadata types
//!   [`crate::types::VectorSearchResult`] and [`crate::types::VectorMeta`]
//!   are plain data, independent of `hnsw_rs`.
//! - **`GraphStore`** — in `ctxvault-common`. The Petgraph `KnowledgeGraph`'s
//!   used surface exchanges `Document`/`Edge`/path-and-title domain values and
//!   returns domain types; `petgraph::NodeIndex` and friends stay internal.
//! - **`EmbeddingProvider`** — in `ctxvault-common`. The `Embedder`'s used
//!   surface takes `&str`/`&[&str]` and returns `Vec<f32>` / `Vec<Vec<f32>>`;
//!   `ort::*` and tokenizer types stay internal.
//! - **`SearchService`** — in `ctxvault-common`. Search-mode dispatch and RRF
//!   fusion operate purely over [`crate::types::SearchResult`] and
//!   [`crate::types::Modality`]; the fusion helpers already speak only domain
//!   types.

pub mod catalog;
pub mod embedding;
pub mod extractor;
pub mod graph_store;
pub mod search;
pub mod text_index;
pub mod vector_store;

pub use catalog::*;
pub use embedding::*;
pub use extractor::*;
pub use graph_store::*;
pub use search::*;
pub use text_index::*;
pub use vector_store::*;
