//! BM25 keyword search strategy.

use groundcontrol_common::ports::TextIndex;
use groundcontrol_common::types::{Modality, SearchResult};
use groundcontrol_common::Result;

/// Simple BM25 keyword search, restricted to the requested [`Modality`].
pub fn search_bm25(
    bm25: &impl TextIndex,
    query: &str,
    limit: usize,
    modality: Modality,
) -> Result<Vec<SearchResult>> {
    bm25.search_with_modality(query, limit, modality)
}
