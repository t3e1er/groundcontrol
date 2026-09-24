//! File classification and exclusion pattern matching.

pub mod classifier;
pub mod exclude;

pub use classifier::{FileClassification, FileClassifier};
pub use exclude::ExcludeMatcher;
