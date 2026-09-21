//! Shared types, traits, and error definitions for the groundcontrol engine.
//!
//! This crate contains no business logic — only the contracts that other crates
//! depend on. Keep it lean: adding a dependency here forces it on every consumer.
//!
//! In the ports-and-adapters layering it hosts the **port traits** in
//! [`ports`] — [`ports::MetadataCatalog`], [`ports::TextIndex`],
//! [`ports::VectorStore`], [`ports::GraphStore`], [`ports::EmbeddingProvider`],
//! and [`ports::SearchService`] — that the domain depends on, alongside the
//! domain [`types`] their signatures speak. Being dependency-light is what keeps
//! those ports backend-free.

pub mod client;
pub mod config;
pub mod error;
pub mod ports;
pub mod types;

pub use client::{ClientEntry, ClientsRegistry};
pub use error::{Error, Result};
