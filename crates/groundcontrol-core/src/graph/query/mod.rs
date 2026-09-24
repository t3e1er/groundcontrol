//! Cypher-Lite Linear Path Query DSL & Recursive Expansion Engine.
//!
//! Implements a constrained, deterministic ASCII path pattern grammar
//! for heterogeneous graph expansion across code symbols and documentation notes.

pub mod ast;
pub mod engine;
pub mod parser;

#[cfg(test)]
mod tests;

pub use ast::{EdgePattern, NodePattern, PathPattern, QueryDirection};
pub use engine::QueryEngine;
pub use parser::parse_path_pattern;
