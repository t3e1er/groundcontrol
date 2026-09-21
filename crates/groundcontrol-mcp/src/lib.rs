//! MCP protocol layer: tool registration, transport, request handling.
//!
//! This crate translates MCP JSON-RPC tool calls into `groundcontrol-core` operations
//! and formats responses. It contains no domain logic.
//!
//! As an upper layer it depends only on the `groundcontrol-common` ports (including
//! `SearchService`) and domain types, plus the `Engine` / `CorpusManager`
//! orchestrators — it names **no** concrete backend type (`rusqlite`, `tantivy`,
//! `hnsw_rs`, `petgraph`, `ort`), reaching every capability through a port.

pub mod client;
pub mod format;
pub mod tools;
pub mod transport;
