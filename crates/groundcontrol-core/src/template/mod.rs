//! Note templates: YAML schema definitions, validation, and starter scaffolds.

pub mod loader;
pub mod model;
pub mod validator;

#[cfg(test)]
mod tests;

pub use model::{FieldSchema, FieldType, Severity, Template, ValidationIssue, ValidationResult};
