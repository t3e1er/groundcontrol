//! HTTP and SSE route handlers for GraphView.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::layout::tiered::{
    build_tier_0_overview, build_tier_1_corpus, build_tier_2_local, compute_corpus_centers,
    TIER_0_BUDGET, TIER_1_DEFAULT_BUDGET,
};
use crate::layout::{ClusterMode, GraphLayout};
use crate::server::state::ServerState;
use crate::telemetry::AgentActivation;
use crate::wire::BinaryWireEncoder;

/// Query parameters for format selection.
#[derive(Debug, Deserialize)]
pub struct FormatQuery {
    /// Desired response format: "binary" (default) or "json".
    pub format: Option<String>,
    /// Clustering mode: "directory" or "community".
    pub cluster_mode: Option<ClusterMode>,
    /// Maximum node budget across all corpora.
    pub budget: Option<usize>,
}

/// Query parameters for corpus graph retrieval.
#[derive(Debug, Deserialize)]
pub struct CorpusQuery {
    /// Maximum node budget (default: 25,000).
    pub budget: Option<usize>,
    /// Desired response format: "binary" or "json".
    pub format: Option<String>,
    /// Clustering mode: "directory" or "community".
    pub cluster_mode: Option<ClusterMode>,
}

/// Query parameters for local ego subgraph.
#[derive(Debug, Deserialize)]
pub struct SubgraphQuery {
    /// Target corpus name.
    pub corpus: Option<String>,
    /// Central/focal node path.
    pub center: String,
    /// Neighborhood expansion depth (default: 2 hops).
    pub hops: Option<usize>,
    /// Desired response format: "binary" or "json".
    pub format: Option<String>,
    /// Clustering mode: "directory" or "community".
    pub cluster_mode: Option<ClusterMode>,
}

/// Search/query request payload.
#[derive(Debug, Deserialize)]
pub struct SearchQueryRequest {
    /// Search terms or Cypher pattern.
    pub query: String,
    /// Corpus filter.
    pub corpus: Option<String>,
}

/// Search match response.
#[derive(Debug, Serialize)]
pub struct SearchQueryResponse {
    /// Matched node paths.
    pub matched_paths: Vec<String>,
    /// Matched node IDs in the current layout, if matched.
    pub matched_ids: Vec<u32>,
}

/// Helper to serialize layout as either binary or JSON.
fn format_layout_response(layout: &GraphLayout, format: Option<&str>) -> Response {
    if format == Some("json") {
        Json(layout).into_response()
    } else {
        let bytes = BinaryWireEncoder::encode(layout);
        (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CONTENT_DISPOSITION, "inline; filename=\"graph.bin\""),
            ],
            bytes,
        )
            .into_response()
    }
}

/// Server status probe handler.
pub async fn handle_status(State(state): State<ServerState>) -> Json<Value> {
    let catalog = state.catalog.read().await;
    let corpora: Vec<Value> = catalog
        .corpus_names()
        .iter()
        .map(|name| {
            if let Some(snap) = catalog.get_corpus(name) {
                serde_json::json!({
                    "name": name,
                    "nodes": snap.graph.node_count(),
                    "edges": snap.graph.edge_count(),
                })
            } else {
                serde_json::json!({ "name": name })
            }
        })
        .collect();

    Json(serde_json::json!({
        "status": "ready",
        "service": "groundcontrol-graphview",
        "corpora": corpora,
        "daemon_url": state.daemon_url,
    }))
}

/// List all available corpora.
pub async fn handle_corpora(State(state): State<ServerState>) -> Json<Value> {
    let catalog = state.catalog.read().await;
    let names = catalog.corpus_names();
    Json(serde_json::json!({ "corpora": names }))
}

/// Return 3D centers and metadata for each corpus cloud in Galaxy view.
pub async fn handle_clouds(State(state): State<ServerState>) -> Json<Value> {
    let catalog = state.catalog.read().await;
    let corpus_names = catalog.corpus_names();
    let centers = compute_corpus_centers(&catalog);
    let mut clouds = Vec::new();

    for name in &corpus_names {
        let Some(snapshot) = catalog.get_corpus(name) else {
            continue;
        };
        let center = centers.get(name).copied().unwrap_or([0.0, 0.0, 0.0]);

        clouds.push(serde_json::json!({
            "name": name,
            "center": [center[0], center[1], center[2]],
            "nodes": snapshot.graph.node_count(),
            "edges": snapshot.graph.edge_count(),
        }));
    }

    Json(serde_json::json!({ "clouds": clouds }))
}

/// Tier 0: Galaxy Overview handler.
pub async fn handle_overview(
    State(state): State<ServerState>,
    Query(q): Query<FormatQuery>,
) -> Response {
    let mode = q.cluster_mode.unwrap_or_default();
    let budget = q.budget.unwrap_or(TIER_0_BUDGET);
    let cache_key = format!("overview:all:{budget}:{mode:?}");
    {
        let cache = state.layout_cache.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            return format_layout_response(cached, q.format.as_deref());
        }
    }

    let catalog = state.catalog.read().await;
    let layout = build_tier_0_overview(&catalog, budget, mode);
    drop(catalog);

    {
        let mut cache = state.layout_cache.write().await;
        cache.insert(cache_key, layout.clone());
    }

    format_layout_response(&layout, q.format.as_deref())
}

/// Tier 1: Corpus Shell handler.
pub async fn handle_corpus(
    State(state): State<ServerState>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<CorpusQuery>,
) -> Response {
    let budget = q.budget.unwrap_or(TIER_1_DEFAULT_BUDGET);
    let mode = q.cluster_mode.unwrap_or_default();
    let cache_key = format!("corpus:{}:{}:{:?}", name, budget, mode);

    {
        let cache = state.layout_cache.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            return format_layout_response(cached, q.format.as_deref());
        }
    }

    let catalog = state.catalog.read().await;
    let Some(snapshot) = catalog.get_corpus(&name) else {
        return (StatusCode::NOT_FOUND, format!("Corpus '{}' not found", name)).into_response();
    };

    let layout = build_tier_1_corpus(&snapshot, budget, mode);
    drop(catalog);

    {
        let mut cache = state.layout_cache.write().await;
        cache.insert(cache_key, layout.clone());
    }

    format_layout_response(&layout, q.format.as_deref())
}

/// Tier 2: Local Ego Subgraph handler.
pub async fn handle_subgraph(
    State(state): State<ServerState>,
    Query(q): Query<SubgraphQuery>,
) -> Response {
    let hops = q.hops.unwrap_or(2).clamp(1, 4);
    let mode = q.cluster_mode.unwrap_or_default();
    let catalog = state.catalog.read().await;

    let snapshot = if let Some(ref c_name) = q.corpus {
        catalog.get_corpus(c_name)
    } else {
        // Fallback: pick first corpus containing node
        catalog.corpus_names().iter().find_map(|c| {
            let s = catalog.get_corpus(c)?;
            if s.graph.contains_node(&q.center) {
                Some(s)
            } else {
                None
            }
        })
    };

    let Some(snapshot) = snapshot else {
        return (
            StatusCode::NOT_FOUND,
            format!("Node '{}' not found in any loaded corpus", q.center),
        )
            .into_response();
    };

    match build_tier_2_local(&snapshot, &q.center, hops, mode) {
        Some(layout) => format_layout_response(&layout, q.format.as_deref()),
        None => (StatusCode::NOT_FOUND, format!("Failed to build subgraph for '{}'", q.center))
            .into_response(),
    }
}

/// Search/Query handler matching nodes.
pub async fn handle_query(
    State(state): State<ServerState>,
    Json(req): Json<SearchQueryRequest>,
) -> Json<SearchQueryResponse> {
    let query_lower = req.query.to_lowercase();
    let catalog = state.catalog.read().await;
    let mut matched_paths = Vec::new();

    let target_corpora =
        if let Some(ref c) = req.corpus { vec![c.clone()] } else { catalog.corpus_names() };

    for name in target_corpora {
        if let Some(snap) = catalog.get_corpus(&name) {
            for path in snap.graph.node_paths() {
                if path.to_lowercase().contains(&query_lower) {
                    matched_paths.push(path);
                    if matched_paths.len() >= 200 {
                        break;
                    }
                }
            }
        }
    }

    Json(SearchQueryResponse { matched_paths, matched_ids: Vec::new() })
}

/// Real-time SSE telemetry activation stream.
pub async fn handle_sse_activations(
    State(state): State<ServerState>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let rx = state.telemetry.subscribe();
    let history = state.telemetry.recent_history().await;

    info!("[SSE] Client connected to real-time activation telemetry stream");

    // Emit recent history first
    let history_events = history.into_iter().filter_map(|act| {
        serde_json::to_string(&act)
            .ok()
            .map(|data| Ok(Event::default().event("activation").data(data)))
    });
    let history_stream = stream::iter(history_events);

    // Then stream real-time events
    let broadcast_stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(act) => {
                    if let Ok(data) = serde_json::to_string(&act) {
                        let ev = Event::default().event("activation").data(data);
                        return Some((Ok(ev), rx));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    });

    let combined = history_stream.chain(broadcast_stream);

    Sse::new(combined).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
}

/// Return configured client profiles and visual identities.
pub async fn handle_clients(
    State(state): State<ServerState>,
) -> Json<groundcontrol_common::ClientsRegistry> {
    Json((*state.clients).clone())
}

/// Inject synthetic test activation (for testing/demo purposes).
pub async fn handle_inject_activation(
    State(state): State<ServerState>,
    headers: axum::http::HeaderMap,
    Json(mut activation): Json<AgentActivation>,
) -> StatusCode {
    if state.clients.require_auth {
        let client_key = headers
            .get("x-api-key")
            .or_else(|| headers.get("x-client-key"))
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
            });

        match client_key {
            Some(k) if !k.is_empty() && state.clients.is_valid_key(k) => {}
            _ => return StatusCode::UNAUTHORIZED,
        }
    }

    if activation.client_color.is_none() {
        if let Some(entry) = state.clients.resolve(None, activation.client_id.as_deref()) {
            if activation.client_id.is_none() {
                activation.client_id = Some(entry.id.clone());
            }
            if activation.client_name.is_none() {
                activation.client_name = Some(entry.name.clone());
            }
            activation.client_color = Some(entry.color.clone());
        }
    }
    state.telemetry.publish(activation).await;
    StatusCode::ACCEPTED
}

/// Fallback route providing basic HTML dashboard if static UI is not yet compiled.
pub async fn handle_embedded_html() -> impl IntoResponse {
    let html = include_str!("../embedded_dashboard.html");
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
}

/// Create the full Axum router.
pub fn create_router(state: ServerState) -> Router {
    Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/corpora", get(handle_corpora))
        .route("/api/clients", get(handle_clients))
        .route("/api/graph/clouds", get(handle_clouds))
        .route("/api/graph/overview", get(handle_overview))
        .route("/api/graph/corpus/{name}", get(handle_corpus))
        .route("/api/graph/subgraph", get(handle_subgraph))
        .route("/api/graph/query", post(handle_query))
        .route(
            "/api/events/activations",
            get(handle_sse_activations).post(handle_inject_activation),
        )
        .route("/api/mcp/activity", post(handle_inject_activation))
        .fallback(handle_embedded_html)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
