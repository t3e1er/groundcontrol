//! Graph tools: `graph_match`, `graph_communities`, `trace_cross_corpus`.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use ctxvault_common::ports::GraphStore;
use ctxvault_common::{Error, Result};
use ctxvault_core::corpus_manager::CorpusManager;
use ctxvault_core::engine::Engine;

#[derive(Debug, Deserialize)]
pub(crate) struct GraphMatchParams {
    pub pattern: String,
    pub edge_class: Option<String>,
    #[serde(rename = "where")]
    pub where_clause: Option<String>,
    pub limit: Option<usize>,
    pub max_depth: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphCommunitiesParams {
    pub algorithm: Option<String>,
    pub view: Option<String>,
    pub include_density: Option<bool>,
    pub community_id: Option<usize>,
    pub limit: Option<usize>,
}

/// Execute a Cypher-Lite graph path query compiled to SQLite recursive CTE.
pub fn handle_graph_match(engine: &Engine, args: Value) -> Result<Value> {
    let params: GraphMatchParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let limit = params.limit.unwrap_or(20);
    let max_depth = params.max_depth.unwrap_or(3);

    let match_result = engine.graph_match(
        &params.pattern,
        params.edge_class.as_deref(),
        params.where_clause.as_deref(),
        limit,
        max_depth,
    )?;

    if params.format.as_deref() == Some("lean") {
        Ok(Value::String(crate::format::lean::format_lean_graph_match(&match_result)))
    } else {
        serde_json::to_value(match_result)
            .map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Detect communities via Leiden or Louvain, or architectural components overview.
pub fn handle_graph_communities(engine: &Engine, args: Value) -> Result<Value> {
    let params: GraphCommunitiesParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let view = params.view.as_deref().unwrap_or("raw");
    if view == "architecture" {
        let result = engine.graph().detect_communities_leiden();
        let densities = engine.graph().community_densities();
        let density_map: HashMap<usize, f64> =
            densities.into_iter().map(|d| (d.community_id, d.density)).collect();

        // If a specific community is requested, return that community with its members
        if let Some(target_id) = params.community_id {
            if let Some(comm) = result.communities.iter().find(|c| c.id == target_id) {
                let mut nodes = comm.members.clone();
                nodes.sort();
                let density = density_map.get(&target_id).copied().unwrap_or(0.0);
                let mem_limit = params.limit.unwrap_or(50).min(nodes.len());
                return Ok(serde_json::json!({
                    "component_id": target_id,
                    "node_count": nodes.len(),
                    "internal_density": density,
                    "members": &nodes[..mem_limit],
                    "total_members": nodes.len(),
                }));
            } else {
                return Err(Error::NotFound(format!("community_id {target_id} not found")));
            }
        }

        let mut clusters = Vec::new();
        let limit = params.limit.unwrap_or(10);

        let mut sorted_comms = result.communities.clone();
        sorted_comms.sort_by(|a, b| b.members.len().cmp(&a.members.len()));

        for comm in sorted_comms.into_iter().take(limit) {
            let comm_id = comm.id;
            let mut nodes = comm.members;
            nodes.sort();
            let density = density_map.get(&comm_id).copied().unwrap_or(0.0);

            let mut key_nodes: Vec<_> =
                nodes.iter().map(|n| (n.clone(), engine.graph().in_degree(n))).collect();
            key_nodes.sort_by(|a, b| b.1.cmp(&a.1));
            let top_key_nodes: Vec<String> =
                key_nodes.into_iter().take(5).map(|(n, _)| n).collect();

            clusters.push(serde_json::json!({
                "component_id": comm_id,
                "node_count": nodes.len(),
                "internal_density": density,
                "top_nodes": top_key_nodes,
            }));
        }

        return Ok(serde_json::json!({
            "algorithm": "leiden",
            "component_count": clusters.len(),
            "total_communities": result.communities.len(),
            "top_components_returned": clusters.len(),
            "modularity": result.modularity,
            "components": clusters,
        }));
    }

    let algo = params.algorithm.as_deref().unwrap_or("leiden");
    let result = match algo {
        "louvain" => engine.graph().detect_communities(),
        _ => engine.graph().detect_communities_leiden(),
    };

    if params.include_density.unwrap_or(false) {
        let densities = engine.graph().community_densities();
        let response = serde_json::json!({
            "communities": result.communities,
            "modularity": result.modularity,
            "iterations": result.iterations,
            "community_densities": densities,
        });
        Ok(response)
    } else {
        serde_json::to_value(result).map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Upper bound on `per_corpus_depth` / `max_corpus_hops` to keep the federated
/// walk bounded and protect query latency (invariant I3).
const MAX_FEDERATED_BOUND: usize = 10;

/// Handle `trace_cross_corpus`: a bounded federated graph traversal across
/// corpora, returning corpus-tagged nodes and cross-corpus hop records.
///
/// Parses the start point plus bounded depth/hop budgets (clamped to
/// `MAX_FEDERATED_BOUND`), delegates to `CorpusManager::federated_traverse`,
/// and serializes the resulting `FederatedTraversal` to JSON.
pub fn handle_trace_cross_corpus(manager: &CorpusManager, args: Value) -> Result<Value> {
    let start_corpus = args
        .get("start_corpus")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("trace_cross_corpus requires 'start_corpus'".to_string()))?;
    let start_node = args
        .get("start_node")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("trace_cross_corpus requires 'start_node'".to_string()))?;

    let per_corpus_depth = args
        .get("per_corpus_depth")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(3)
        .min(MAX_FEDERATED_BOUND);
    let max_corpus_hops = args
        .get("max_corpus_hops")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(3)
        .min(MAX_FEDERATED_BOUND);
    let continue_across = args.get("continue").and_then(Value::as_bool).unwrap_or(true);

    let traversal = manager.federated_traverse(
        start_corpus,
        start_node,
        per_corpus_depth,
        max_corpus_hops,
        continue_across,
    )?;

    serde_json::to_value(&traversal)
        .map_err(|e| Error::Config(format!("failed to serialize federated traversal: {}", e)))
}

/// Dummy placeholder for single-engine registration of `trace_cross_corpus`.
pub fn handle_trace_cross_corpus_dummy(_engine: &Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("trace_cross_corpus is a manager-level tool".to_string()))
}
