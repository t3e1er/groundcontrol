//! MCP tool definitions: maps tool names → core engine calls.
//!
//! Each tool is a named handler function that takes `(&mut Engine, Value)` or
//! `(&Engine, Value)` and returns `Result<Value>`. The [`ToolRegistry`] and
//! [`MultiCorpusToolRegistry`] manage registration, fan-out, and dispatch.

pub mod graph;
pub mod read;
pub mod registry;
pub mod search;
pub mod system;
pub mod template;
pub mod write;

#[cfg(test)]
mod tests;

pub use registry::{MultiCorpusToolRegistry, ToolHandler, ToolInfo, ToolProfile, ToolRegistry};
