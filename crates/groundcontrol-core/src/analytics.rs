//! Analytics tools: density analysis, semantic gap detection, split suggestions, coverage reports.
//!
//! These tools provide insights into corpus quality, index coverage, and opportunities
//! for improving retrieval performance.

use std::path::Path;

use groundcontrol_common::Result;
use serde::{Deserialize, Serialize};

use crate::graph::KnowledgeGraph;
use crate::index::BM25Index;
use crate::vector_index::VectorIndex;

// ---------------------------------------------------------------------------
// analyze_density
// ---------------------------------------------------------------------------

/// Result of graph density analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DensityReport {
    /// Total number of nodes in the graph.
    pub total_nodes: usize,
    /// Total number of edges.
    pub total_edges: usize,
    /// Overall graph density (edges / max_possible_edges).
    pub density: f64,
    /// Nodes with no edges (orphans).
    pub orphans: Vec<String>,
    /// Top N most-connected nodes (hubs).
    pub hubs: Vec<HubInfo>,
    /// Density breakdown per tag (if tags available).
    pub tag_density: Vec<TagDensity>,
    /// Per-community density statistics (from Louvain detection).
    pub community_stats: Vec<CommunityDensityInfo>,
}

/// Information about a hub node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubInfo {
    /// Node path.
    pub path: String,
    /// Total degree (in + out edges).
    pub degree: usize,
    /// Inbound edge count.
    pub in_degree: usize,
    /// Outbound edge count.
    pub out_degree: usize,
}

/// Density information for a specific tag group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDensity {
    /// Tag name.
    pub tag: String,
    /// Number of nodes with this tag.
    pub node_count: usize,
    /// Number of edges between nodes sharing this tag.
    pub internal_edges: usize,
}

/// Per-community density statistics for the density report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityDensityInfo {
    /// Community id.
    pub community_id: usize,
    /// Number of members in this community.
    pub member_count: usize,
    /// Number of internal edges within the community.
    pub internal_edges: usize,
    /// Density of internal connections.
    pub density: f64,
}

/// Analyze graph density, identifying hubs and orphans.
pub fn analyze_density(graph: &KnowledgeGraph, top_hubs: usize) -> DensityReport {
    let stats = graph.stats();

    let total_nodes = stats.node_count;
    let total_edges = stats.edge_count;

    // Density = edges / (nodes * (nodes - 1)) for directed graph.
    let max_edges = if total_nodes > 1 { total_nodes * (total_nodes - 1) } else { 1 };
    let density = total_edges as f64 / max_edges as f64;

    // Find orphans.
    let orphans = graph.orphan_paths();

    // Find hubs (most connected nodes) using node_degree_list.
    let degrees = graph.node_degree_list();
    let mut hubs: Vec<HubInfo> = degrees
        .into_iter()
        .map(|(path, in_deg, out_deg)| HubInfo {
            path,
            degree: in_deg + out_deg,
            in_degree: in_deg,
            out_degree: out_deg,
        })
        .collect();
    hubs.sort_by(|a, b| b.degree.cmp(&a.degree));
    hubs.truncate(top_hubs);

    // Compute per-community density statistics.
    let community_densities = graph.community_densities();
    let community_stats: Vec<CommunityDensityInfo> = community_densities
        .into_iter()
        .map(|cd| CommunityDensityInfo {
            community_id: cd.community_id,
            member_count: cd.node_count,
            internal_edges: cd.internal_edges,
            density: cd.density,
        })
        .collect();

    DensityReport {
        total_nodes,
        total_edges,
        density,
        orphans,
        hubs,
        tag_density: Vec::new(), // Tag density requires store access, left empty here.
        community_stats,
    }
}

// ---------------------------------------------------------------------------
// find_semantic_gaps
// ---------------------------------------------------------------------------

/// A semantic gap: a query where BM25 and vector search produce divergent results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticGap {
    /// The test query that revealed the gap.
    pub query: String,
    /// Documents found only by BM25 (not in vector top-K).
    pub bm25_only: Vec<String>,
    /// Documents found only by vector search (not in BM25 top-K).
    pub vector_only: Vec<String>,
    /// Overlap ratio (0.0 = completely disjoint, 1.0 = identical results).
    pub overlap_ratio: f64,
}

/// Find queries where BM25 and vector search disagree.
///
/// Takes a set of test queries and compares BM25 vs vector results for each.
/// Queries with low overlap indicate potential embedding blind spots.
///
/// - `bm25`: BM25 index to search.
/// - `vector_index`: Vector index to search.
/// - `queries`: Test queries to evaluate.
/// - `query_embeddings`: Pre-computed embeddings for each query (same order as queries).
/// - `top_k`: Number of results to compare per query.
pub fn find_semantic_gaps(
    bm25: &BM25Index,
    vector_index: &VectorIndex,
    queries: &[&str],
    query_embeddings: &[Vec<f32>],
    top_k: usize,
) -> Result<Vec<SemanticGap>> {
    let mut gaps = Vec::new();

    for (query, embedding) in queries.iter().zip(query_embeddings.iter()) {
        // Get BM25 results.
        let bm25_results = bm25.search(query, top_k)?;
        let bm25_paths: std::collections::HashSet<String> =
            bm25_results.iter().map(|r| r.path.clone()).collect();

        // Get vector results (analytics spans both modalities).
        let vector_results = vector_index.search(
            embedding,
            top_k,
            false,
            groundcontrol_common::types::Modality::Both,
        )?;
        let vector_paths: std::collections::HashSet<String> =
            vector_results.iter().map(|r| r.doc_path.clone()).collect();

        // Compute overlap.
        let overlap: std::collections::HashSet<&String> =
            bm25_paths.intersection(&vector_paths).collect();
        let union_size = bm25_paths.len() + vector_paths.len() - overlap.len();
        let overlap_ratio =
            if union_size > 0 { overlap.len() as f64 / union_size as f64 } else { 1.0 };

        let bm25_only: Vec<String> = bm25_paths.difference(&vector_paths).cloned().collect();
        let vector_only: Vec<String> = vector_paths.difference(&bm25_paths).cloned().collect();

        gaps.push(SemanticGap { query: query.to_string(), bm25_only, vector_only, overlap_ratio });
    }

    // Sort by overlap ratio ascending (most divergent first).
    gaps.sort_by(|a, b| {
        a.overlap_ratio.partial_cmp(&b.overlap_ratio).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(gaps)
}

// ---------------------------------------------------------------------------
// suggest_splits
// ---------------------------------------------------------------------------

/// A chunk that may benefit from being split into smaller pieces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitSuggestion {
    /// Document path.
    pub path: String,
    /// Chunk index.
    pub chunk_index: usize,
    /// Length of the chunk in characters.
    pub char_count: usize,
    /// Reason for suggesting a split.
    pub reason: String,
    /// Coherence score (lower = less coherent, more likely to benefit from split).
    pub coherence_score: f64,
}

/// Suggest chunks that might benefit from splitting.
///
/// Heuristics:
/// - Chunks that are very long (near max token limit)
/// - Chunks with multiple heading-level transitions
/// - Chunks with high topic diversity (measured by distinct section keywords)
///
/// This is a heuristic-based approach that doesn't require embeddings.
pub fn suggest_splits(
    store: &crate::persistence::Store,
    corpus_root: Option<&Path>,
    max_chunk_chars: usize,
) -> Result<Vec<SplitSuggestion>> {
    let files = store.list_files()?;
    let mut suggestions = Vec::new();

    for file in &files {
        let chunks = store.get_chunks_for_file(&file.path)?;

        for chunk in &chunks {
            let text_opt = corpus_root.and_then(|root| {
                let full = root.join(&file.path);
                let mut f = std::fs::File::open(&full).ok()?;
                use std::io::{Read, Seek, SeekFrom};
                f.seek(SeekFrom::Start(chunk.start_byte as u64)).ok()?;
                let len = chunk.end_byte.saturating_sub(chunk.start_byte);
                let mut buf = vec![0u8; len];
                f.read_exact(&mut buf).ok()?;
                String::from_utf8(buf).ok()
            });

            let char_count = text_opt
                .as_ref()
                .map(|t| t.len())
                .unwrap_or_else(|| chunk.end_byte.saturating_sub(chunk.start_byte));

            // Heuristic 1: Very long chunks.
            if char_count > max_chunk_chars {
                suggestions.push(SplitSuggestion {
                    path: file.path.clone(),
                    chunk_index: chunk.chunk_index,
                    char_count,
                    reason: format!(
                        "chunk exceeds {} characters ({} chars)",
                        max_chunk_chars, char_count
                    ),
                    coherence_score: 0.3,
                });
                continue;
            }

            if let Some(ref text) = text_opt {
                // Heuristic 2: Multiple headings within a single chunk.
                let heading_count = text.lines().filter(|line| line.starts_with('#')).count();
                if heading_count > 1 {
                    let coherence = 1.0 / (heading_count as f64);
                    suggestions.push(SplitSuggestion {
                        path: file.path.clone(),
                        chunk_index: chunk.chunk_index,
                        char_count,
                        reason: format!("chunk contains {} headings", heading_count),
                        coherence_score: coherence,
                    });
                    continue;
                }

                // Heuristic 3: Very diverse content (many paragraph breaks relative to size).
                let paragraph_count = text.split("\n\n").count();
                if paragraph_count > 5 && char_count > 500 {
                    let coherence = 1.0 - (paragraph_count as f64 / 10.0).min(0.8);
                    suggestions.push(SplitSuggestion {
                        path: file.path.clone(),
                        chunk_index: chunk.chunk_index,
                        char_count,
                        reason: format!(
                            "chunk has {} paragraphs (high topic diversity)",
                            paragraph_count
                        ),
                        coherence_score: coherence,
                    });
                }
            }
        }
    }

    // Sort by coherence score ascending (least coherent first).
    suggestions.sort_by(|a, b| {
        a.coherence_score.partial_cmp(&b.coherence_score).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(suggestions)
}

// ---------------------------------------------------------------------------
// coverage_report
// ---------------------------------------------------------------------------

/// Coverage report result for a test query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCoverage {
    /// The test query.
    pub query: String,
    /// Documents retrieved for this query.
    pub retrieved: Vec<String>,
    /// Number of documents retrieved.
    pub retrieved_count: usize,
}

/// Coverage report showing which notes are never retrieved across a set of test queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Total number of notes in the corpus.
    pub total_notes: usize,
    /// Number of notes retrieved by at least one query.
    pub covered_notes: usize,
    /// Notes never retrieved by any test query (dead zones).
    pub uncovered_notes: Vec<String>,
    /// Coverage ratio (covered / total).
    pub coverage_ratio: f64,
    /// Per-query retrieval details.
    pub per_query: Vec<QueryCoverage>,
}

/// Generate a coverage report: for a set of test queries, find which notes are never retrieved.
///
/// - `bm25`: BM25 index to search.
/// - `queries`: Set of test queries.
/// - `all_note_paths`: All known note paths in the corpus.
/// - `top_k`: Number of results to consider per query.
pub fn coverage_report(
    bm25: &BM25Index,
    queries: &[&str],
    all_note_paths: &[String],
    top_k: usize,
) -> Result<CoverageReport> {
    let mut ever_retrieved: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut per_query = Vec::new();

    for query in queries {
        let results = bm25.search(query, top_k)?;
        let retrieved: Vec<String> = results.iter().map(|r| r.path.clone()).collect();

        for path in &retrieved {
            let _ = ever_retrieved.insert(path.clone());
        }

        per_query.push(QueryCoverage {
            query: query.to_string(),
            retrieved_count: retrieved.len(),
            retrieved,
        });
    }

    let total_notes = all_note_paths.len();
    let covered_notes = ever_retrieved.len();
    let uncovered_notes: Vec<String> =
        all_note_paths.iter().filter(|p| !ever_retrieved.contains(p.as_str())).cloned().collect();
    let coverage_ratio =
        if total_notes > 0 { covered_notes as f64 / total_notes as f64 } else { 1.0 };

    Ok(CoverageReport { total_notes, covered_notes, uncovered_notes, coverage_ratio, per_query })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::types::EdgeProvenance;

    #[test]
    fn test_analyze_density_empty_graph() {
        let graph = KnowledgeGraph::new();
        let report = analyze_density(&graph, 5);
        assert_eq!(report.total_nodes, 0);
        assert_eq!(report.total_edges, 0);
    }

    #[test]
    fn test_analyze_density_with_data() {
        let mut graph = KnowledgeGraph::new();
        graph.add_edge(
            "A",
            "B",
            "Link",
            1.0,
            EdgeProvenance::Wikilink,
            groundcontrol_common::config::EdgeClass::Structural,
        );
        graph.add_edge(
            "B",
            "C",
            "Link",
            1.0,
            EdgeProvenance::Wikilink,
            groundcontrol_common::config::EdgeClass::Structural,
        );
        graph.add_edge(
            "A",
            "C",
            "Link",
            1.0,
            EdgeProvenance::Wikilink,
            groundcontrol_common::config::EdgeClass::Structural,
        );
        // D is orphan (no edges to/from).
        graph.ensure_node("D");

        let report = analyze_density(&graph, 3);
        assert_eq!(report.total_nodes, 4);
        assert_eq!(report.total_edges, 3);
        assert!(report.density > 0.0);

        // D should be an orphan.
        assert!(report.orphans.contains(&"D".to_string()));

        // A should be a hub (highest degree: 2 outbound).
        assert!(!report.hubs.is_empty());
        assert_eq!(report.hubs[0].path, "A");
    }

    #[test]
    fn test_find_semantic_gaps_disjoint() {
        use crate::vector_index::VectorIndex;
        use groundcontrol_common::types::Chunk;

        let mut bm25 = BM25Index::open_in_memory().unwrap();
        let mut vi = VectorIndex::new(4, 100, 200, 16);

        // BM25 has doc A, vector has doc B.
        let chunks = vec![Chunk::new("A", 0, "alpha beta gamma", 0, 16)];
        bm25.add_document("A", Some("Alpha"), &[], &chunks).unwrap();
        bm25.commit().unwrap();

        // Add B to vector index only.
        let vec_b: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
        vi.add(&vec_b, "B", Some(0), false, "docs").unwrap();

        let query_emb = vec![1.0, 0.0, 0.0, 0.0];
        let gaps = find_semantic_gaps(&bm25, &vi, &["alpha"], &[query_emb], 5).unwrap();

        assert_eq!(gaps.len(), 1);
        // BM25 finds A, vector finds B — no overlap.
        assert!(gaps[0].bm25_only.contains(&"A".to_string()));
        assert!(gaps[0].vector_only.contains(&"B".to_string()));
        assert_eq!(gaps[0].overlap_ratio, 0.0);
    }

    #[test]
    fn test_coverage_report_identifies_dead_zones() {
        use groundcontrol_common::types::Chunk;

        let mut bm25 = BM25Index::open_in_memory().unwrap();

        let chunks_a = vec![Chunk::new("A", 0, "rust systems programming", 0, 24)];
        let chunks_b = vec![Chunk::new("B", 0, "python data science", 0, 19)];
        bm25.add_document("A", Some("Rust"), &[], &chunks_a).unwrap();
        bm25.add_document("B", Some("Python"), &[], &chunks_b).unwrap();
        bm25.commit().unwrap();

        let all_paths = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let queries = &["rust programming"];

        let report = coverage_report(&bm25, queries, &all_paths, 10).unwrap();

        assert_eq!(report.total_notes, 3);
        // Only A should be retrieved.
        assert!(report.covered_notes >= 1);
        // C is never indexed, so it's uncovered.
        assert!(report.uncovered_notes.contains(&"C".to_string()));
        // B might not be retrieved for "rust programming".
        assert!(report.coverage_ratio < 1.0);
    }
}
