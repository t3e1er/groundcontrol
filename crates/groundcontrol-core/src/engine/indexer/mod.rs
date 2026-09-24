//! Indexing pipeline, delta scanning, batching, commits, and re-embedding.

pub mod commit;
pub mod delta;
pub mod ingest;
pub mod reembed;
pub mod reindex;
pub mod stage_parse;
