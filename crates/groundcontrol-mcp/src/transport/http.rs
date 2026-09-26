//! MCP Localhost HTTP & Server-Sent Events (SSE) Transport.
//!
//! Provides a streamable HTTP JSON-RPC 2.0 server using `axum`, enabling
//! tool querying, liveness health checks, and SSE session streams.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use serde_json::Value;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use groundcontrol_common::ports::MetadataCatalog;
use groundcontrol_common::{Error, Result};
use groundcontrol_core::corpus_manager::CorpusManager;

use crate::tools::MultiCorpusToolRegistry;
use crate::transport::dispatch::{
    dispatch_multi_read, dispatch_multi_write, format_rpc_response, is_read_only_request_multi,
    make_error_response, JsonRpcRequest, PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION,
};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Format a concise summary description of an MCP request for logging.
fn describe_request(req: &JsonRpcRequest) -> String {
    if req.method == "tools/call" {
        if let Some(params) = &req.params {
            let tool = params.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let args = params
                .get("arguments")
                .map(|a| {
                    let s = a.to_string();
                    if s.len() > 80 {
                        format!("{}...", &s[..77])
                    } else {
                        s
                    }
                })
                .unwrap_or_else(|| "{}".to_string());
            format!("tools/call [{tool}] args: {args}")
        } else {
            "tools/call [unknown]".to_string()
        }
    } else {
        req.method.clone()
    }
}

/// Options for configuring the HTTP server and background daemon.
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// Whether running in background daemon mode.
    pub daemon: bool,
    /// Idle shutdown timeout (e.g. Duration::from_secs(30 * 60)).
    pub idle_timeout: Option<Duration>,
    /// Continuously watch corpus directories for changes and incrementally reindex.
    pub watch: bool,
}

fn current_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Server State Structs
// ---------------------------------------------------------------------------

/// Telemetry event emitted when an agent executes an MCP tool.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentActivation {
    /// Timestamp in UNIX milliseconds.
    pub timestamp: u64,
    /// Tool name (e.g. search, get_snippet, graph_match, write_note).
    pub tool: String,
    /// Connected AI client ID (e.g. "antigravity", "claude", "gemini").
    #[serde(default)]
    pub client_id: Option<String>,
    /// Connected AI client display name.
    #[serde(default)]
    pub client_name: Option<String>,
    /// Associated theme color (e.g. "#38bdf8").
    #[serde(default)]
    pub client_color: Option<String>,
    /// Target corpus name, if scoped.
    pub corpus: Option<String>,
    /// Search query or Cypher pattern, if applicable.
    pub query: Option<String>,
    /// Repository paths of nodes activated or touched.
    pub paths: Vec<String>,
    /// Execution duration in milliseconds.
    pub duration_ms: f64,
    /// Whether the tool execution succeeded.
    pub success: bool,
}

/// Shared state for multi-corpus HTTP server.
#[derive(Clone)]
pub struct MultiCorpusServerState {
    /// Thread-safe reader-writer reference to the multi-corpus manager.
    pub manager: Arc<RwLock<CorpusManager>>,
    /// Registered multi-corpus MCP tool handlers.
    pub registry: Arc<MultiCorpusToolRegistry>,
    /// Timestamp (UNIX epoch seconds) of last incoming request/activity.
    pub last_activity: Arc<AtomicU64>,
    /// Number of active SSE/client streams.
    pub active_sessions: Arc<AtomicU64>,
    /// Broadcast channel for agent activity telemetry.
    pub activations: tokio::sync::broadcast::Sender<AgentActivation>,
    /// Client authentication and tracking registry.
    pub clients: Arc<groundcontrol_common::ClientsRegistry>,
    /// Timestamp when server process started (UNIX epoch seconds).
    pub started_at: u64,
    /// Channel to signal graceful server shutdown.
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// In-flight indexing progress if an index or sync operation is active.
    pub active_indexing: Arc<RwLock<Option<groundcontrol_core::engine::IndexingProgress>>>,
}

impl MultiCorpusServerState {
    /// Create a new server state initialized with current timestamp.
    pub fn new(
        manager: Arc<RwLock<CorpusManager>>,
        registry: Arc<MultiCorpusToolRegistry>,
    ) -> Self {
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        Self::with_shutdown(manager, registry, shutdown_tx)
    }

    /// Create server state with an explicit shutdown channel.
    pub fn with_shutdown(
        manager: Arc<RwLock<CorpusManager>>,
        registry: Arc<MultiCorpusToolRegistry>,
        shutdown_tx: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        let now = current_timestamp_secs();
        let (activations, _) = tokio::sync::broadcast::channel(1024);
        let clients = Arc::new(groundcontrol_common::client::load_clients_config(None));
        Self {
            manager,
            registry,
            last_activity: Arc::new(AtomicU64::new(now)),
            active_sessions: Arc::new(AtomicU64::new(0)),
            activations,
            clients,
            started_at: now,
            shutdown_tx,
            active_indexing: Arc::new(RwLock::new(None)),
        }
    }
}

// ---------------------------------------------------------------------------
// Public Server Entry Points
// ---------------------------------------------------------------------------

/// Start the localhost HTTP MCP server with default options.
pub async fn run_http_server_multi(
    bind_addr: &str,
    manager: CorpusManager,
    registry: MultiCorpusToolRegistry,
) -> Result<()> {
    run_http_server_multi_with_options(bind_addr, manager, registry, ServerOptions::default()).await
}

/// Start the localhost HTTP MCP server with multi-corpus routing and daemon options.
pub async fn run_http_server_multi_with_options(
    bind_addr: &str,
    manager: CorpusManager,
    registry: MultiCorpusToolRegistry,
    options: ServerOptions,
) -> Result<()> {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let state = MultiCorpusServerState::with_shutdown(
        Arc::new(RwLock::new(manager)),
        Arc::new(registry),
        shutdown_tx.clone(),
    );

    let app = Router::new()
        .route("/mcp", post(handle_jsonrpc_multi).get(handle_sse))
        .route("/jsonrpc", post(handle_jsonrpc_multi).get(handle_sse))
        .route("/", post(handle_jsonrpc_multi).get(handle_sse))
        .route("/sse", get(handle_sse).post(handle_jsonrpc_multi))
        .route("/health", get(handle_health_multi))
        .route("/status", get(handle_health_multi))
        .route("/shutdown", post(handle_shutdown))
        .route("/sync", post(handle_admin_sync))
        .route("/events/activations", get(handle_activations_sse))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(bind_addr).await.map_err(Error::Io)?;
    let local_addr = listener.local_addr().map_err(Error::Io)?;

    let corpora_count = {
        let mgr = state.manager.read().await;
        mgr.corpus_names().len()
    };

    eprintln!(
        "\n\
        +========================================================================+\n\
        |  groundcontrol Multi-Corpus MCP Server is Ready                        |\n\
        |                                                                        |\n\
        |  * Listening on: http://{:<47}|\n\
        |  * Corpora:      {:<47}|\n\
        |  * Endpoints:    /sse, /mcp, /health                                   |\n\
        +========================================================================+\n",
        local_addr,
        format!("{} configured corpora", corpora_count)
    );
    info!(addr = %local_addr, daemon = options.daemon, watch = options.watch, "multi-corpus MCP HTTP server listening");

    if options.watch {
        let mgr_clone = state.manager.clone();
        let watch_cb = std::sync::Arc::new(move |name: &str, root_path: &std::path::Path| {
            groundcontrol_core::watcher::spawn_corpus_watcher(
                name.to_string(),
                root_path.to_path_buf(),
                mgr_clone.clone(),
                Duration::from_millis(500),
            );
        });
        state.manager.write().await.set_on_corpus_mounted(watch_cb.clone());
        let paths = {
            let mgr = state.manager.read().await;
            mgr.corpus_paths()
        };
        for (name, root_path) in paths {
            watch_cb(&name, &root_path);
        }
    }

    // If running in daemon mode with idle timeout, spawn background watchdog.
    if options.daemon {
        if let Some(timeout) = options.idle_timeout {
            let last_act = state.last_activity.clone();
            let active_sess = state.active_sessions.clone();
            let tx = state.shutdown_tx.clone();
            let timeout_secs = timeout.as_secs();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let sessions = active_sess.load(Ordering::Relaxed);
                    let last = last_act.load(Ordering::Relaxed);
                    let now = current_timestamp_secs();
                    if sessions == 0 && now.saturating_sub(last) >= timeout_secs {
                        info!(timeout_secs, "daemon idle timeout reached, initiating shutdown");
                        let _ = tx.send(true);
                        break;
                    }
                }
            });
        }
    }

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        })
        .await
        .map_err(|e| Error::Config(format!("server error: {e}")))?;

    // Flush SQLite WAL checkpoints on clean shutdown.
    {
        let mgr = state.manager.read().await;
        for name in mgr.corpus_names() {
            if let Ok(engine) = mgr.get_engine(name) {
                let _ = engine.store().checkpoint();
            }
        }
    }
    info!("multi-corpus server shutdown complete; SQLite WAL checkpoints flushed");

    // Remove daemon PID file if it belongs to this process
    if let Some(pid_info) = groundcontrol_common::config::read_daemon_pid() {
        if pid_info.pid == std::process::id() {
            let _ = groundcontrol_common::config::remove_daemon_pid();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP Request Handlers
// ---------------------------------------------------------------------------

/// Extract authentication key from HTTP headers or query string parameters.
fn extract_auth_key(headers: &HeaderMap, params: &HashMap<String, String>) -> Option<String> {
    headers
        .get("x-api-key")
        .or_else(|| headers.get("x-client-key"))
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(String::from)
        })
        .or_else(|| params.get("api_key").cloned())
        .or_else(|| params.get("token").cloned())
}

/// Extract client ID from HTTP headers or query string parameters.
fn extract_client_id(headers: &HeaderMap, params: &HashMap<String, String>) -> Option<String> {
    headers
        .get("x-client-id")
        .or_else(|| headers.get("x-api-client-id"))
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| params.get("client_id").cloned())
}

/// Process single or batch JSON-RPC request for multi-corpus server.
async fn handle_jsonrpc_multi(
    State(state): State<MultiCorpusServerState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let client_key = extract_auth_key(&headers, &params);
    let client_id = extract_client_id(&headers, &params);

    if state.clients.require_auth {
        match client_key.as_deref() {
            Some(k) if !k.is_empty() && state.clients.is_valid_key(k) => {}
            _ => {
                warn!("Unauthorized MCP JSON-RPC request rejected");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(make_error_response(
                        Value::Null,
                        -32000,
                        "Unauthorized: missing or invalid x-api-key",
                    )),
                )
                    .into_response();
            }
        }
    }

    let resolved_client = state.clients.resolve(client_key.as_deref(), client_id.as_deref());

    if body.is_array() {
        let requests: Vec<JsonRpcRequest> = match serde_json::from_value(body) {
            Ok(reqs) => reqs,
            Err(e) => {
                warn!(error = %e, "Invalid JSON-RPC batch payload (multi-corpus)");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(make_error_response(Value::Null, -32700, &e.to_string())),
                )
                    .into_response();
            }
        };

        let all_read_only = requests.iter().all(|r| is_read_only_request_multi(r, &state.registry));
        let mut responses = Vec::new();

        if all_read_only {
            let manager = state.manager.read().await;
            for req in requests {
                let req_id = REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst);
                let desc = describe_request(&req);
                info!(req_id, "[REQ #{req_id}] --> {} (multi-corpus)", desc);

                let start = Instant::now();
                let res = dispatch_multi_read(&req, &*manager, &state.registry);
                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

                match &res {
                    Ok(_) => {
                        info!(
                            req_id,
                            duration_ms = elapsed_ms,
                            "[RES #{req_id}] <-- Success ({:.2}ms)",
                            elapsed_ms
                        );
                    }
                    Err(e) => {
                        warn!(req_id, duration_ms = elapsed_ms, error = %e, "[RES #{req_id}] <-- Error: {} ({:.2}ms)", e, elapsed_ms);
                    }
                }

                if let Some(activation) =
                    extract_activation(&req, &res, elapsed_ms, resolved_client)
                {
                    let _ = state.activations.send(activation);
                }

                if let Some(id) = req.id {
                    responses.push(format_rpc_response(id, res));
                }
            }
        } else {
            let mut manager = state.manager.write().await;
            for req in requests {
                let req_id = REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst);
                let desc = describe_request(&req);
                info!(req_id, "[REQ #{req_id}] --> {} (multi-corpus)", desc);

                let start = Instant::now();
                let res = dispatch_multi_write(&req, &mut *manager, &state.registry);
                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

                match &res {
                    Ok(_) => {
                        info!(
                            req_id,
                            duration_ms = elapsed_ms,
                            "[RES #{req_id}] <-- Success ({:.2}ms)",
                            elapsed_ms
                        );
                    }
                    Err(e) => {
                        warn!(req_id, duration_ms = elapsed_ms, error = %e, "[RES #{req_id}] <-- Error: {} ({:.2}ms)", e, elapsed_ms);
                    }
                }

                if let Some(activation) =
                    extract_activation(&req, &res, elapsed_ms, resolved_client)
                {
                    let _ = state.activations.send(activation);
                }

                if let Some(id) = req.id {
                    responses.push(format_rpc_response(id, res));
                }
            }
        }

        Json(responses).into_response()
    } else {
        let req: JsonRpcRequest = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Invalid JSON-RPC payload (multi-corpus)");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(make_error_response(Value::Null, -32700, &e.to_string())),
                )
                    .into_response();
            }
        };

        state.last_activity.store(current_timestamp_secs(), Ordering::Relaxed);

        let req_id = REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let desc = describe_request(&req);
        info!(req_id, "[REQ #{req_id}] --> {} (multi-corpus)", desc);

        let start = Instant::now();
        let is_read = is_read_only_request_multi(&req, &state.registry);
        let res = if is_read {
            let manager = state.manager.read().await;
            dispatch_multi_read(&req, &*manager, &state.registry)
        } else {
            let mut manager = state.manager.write().await;
            dispatch_multi_write(&req, &mut *manager, &state.registry)
        };
        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

        match &res {
            Ok(_) => {
                info!(
                    req_id,
                    duration_ms = elapsed_ms,
                    "[RES #{req_id}] <-- Success ({:.2}ms)",
                    elapsed_ms
                );
            }
            Err(e) => {
                warn!(req_id, duration_ms = elapsed_ms, error = %e, "[RES #{req_id}] <-- Error: {} ({:.2}ms)", e, elapsed_ms);
            }
        }

        if let Some(activation) = extract_activation(&req, &res, elapsed_ms, resolved_client) {
            let _ = state.activations.send(activation);
        }

        if let Some(id) = req.id {
            let rpc_res = format_rpc_response(id, res);
            Json(rpc_res).into_response()
        } else {
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// Server-Sent Events stream for agent telemetry activations.
pub async fn handle_activations_sse(
    State(state): State<MultiCorpusServerState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if state.clients.require_auth {
        let client_key = extract_auth_key(&headers, &params);
        match client_key.as_deref() {
            Some(k) if !k.is_empty() && state.clients.is_valid_key(k) => {}
            _ => {
                warn!("Unauthorized MCP SSE activations connection rejected");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }

    let rx = state.activations.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(act) => {
                    if let Ok(data) = serde_json::to_string(&act) {
                        let ev = Event::default().event("activation").data(data);
                        return Some((Ok::<Event, Infallible>(ev), rx));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
        .into_response()
}

fn extract_activation(
    req: &JsonRpcRequest,
    res: &Result<Value>,
    duration_ms: f64,
    client: Option<&groundcontrol_common::ClientEntry>,
) -> Option<AgentActivation> {
    if req.method != "tools/call" {
        return None;
    }
    let params = req.params.as_ref()?;
    let tool = params.get("name")?.as_str()?.to_string();
    let args = params.get("arguments");

    let corpus = args.and_then(|a| a.get("corpus").and_then(|c| c.as_str()).map(String::from));
    let query = args.and_then(|a| a.get("query").and_then(|q| q.as_str()).map(String::from));
    let mut paths = Vec::new();

    if let Some(a) = args {
        if let Some(p) = a.get("path").and_then(|p| p.as_str()) {
            paths.push(p.to_string());
        }
        if let Some(ps) = a.get("paths").and_then(|ps| ps.as_array()) {
            for p in ps {
                if let Some(s) = p.as_str() {
                    if !paths.contains(&s.to_string()) {
                        paths.push(s.to_string());
                    }
                }
            }
        }
        if let Some(sym) = a.get("symbol").and_then(|s| s.as_str()) {
            if !paths.contains(&sym.to_string()) {
                paths.push(sym.to_string());
            }
        }
    }

    if let Ok(val) = res {
        // Direct single path
        if let Some(p) = val.get("path").and_then(|p| p.as_str()) {
            if !paths.contains(&p.to_string()) {
                paths.push(p.to_string());
            }
        }
        // Traditional hits array
        if let Some(hits) = val.get("hits").and_then(|h| h.as_array()) {
            for h in hits.iter().take(5) {
                if let Some(p) = h.get("path").and_then(|p| p.as_str()) {
                    if !paths.contains(&p.to_string()) {
                        paths.push(p.to_string());
                    }
                }
            }
        }
        // Search response: code.results
        if let Some(results) =
            val.get("code").and_then(|c| c.get("results")).and_then(|r| r.as_array())
        {
            for r in results.iter().take(4) {
                if let Some(p) = r.get("path").and_then(|p| p.as_str()) {
                    if !paths.contains(&p.to_string()) {
                        paths.push(p.to_string());
                    }
                }
            }
        }
        // Search response: docs.results
        if let Some(results) =
            val.get("docs").and_then(|d| d.get("results")).and_then(|r| r.as_array())
        {
            for r in results.iter().take(4) {
                if let Some(p) = r.get("path").and_then(|p| p.as_str()) {
                    if !paths.contains(&p.to_string()) {
                        paths.push(p.to_string());
                    }
                }
            }
        }
        // General results array
        if let Some(results) = val.get("results").and_then(|r| r.as_array()) {
            for r in results.iter().take(5) {
                if let Some(p) = r.get("path").and_then(|p| p.as_str()) {
                    if !paths.contains(&p.to_string()) {
                        paths.push(p.to_string());
                    }
                } else if let Some(p) = r.as_str() {
                    if !paths.contains(&p.to_string()) {
                        paths.push(p.to_string());
                    }
                }
            }
        }
    }

    Some(AgentActivation {
        timestamp: current_timestamp_secs() * 1000,
        tool,
        client_id: client.map(|c| c.id.clone()),
        client_name: client.map(|c| c.name.clone()),
        client_color: client.map(|c| c.color.clone()),
        corpus,
        query,
        paths,
        duration_ms,
        success: res.is_ok(),
    })
}

/// Server-Sent Events stream for MCP session handshake.
pub async fn handle_sse(
    State(state): State<MultiCorpusServerState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    use futures_util::StreamExt;
    if state.clients.require_auth {
        let client_key = extract_auth_key(&headers, &params);
        match client_key.as_deref() {
            Some(k) if !k.is_empty() && state.clients.is_valid_key(k) => {}
            _ => {
                warn!("Unauthorized MCP SSE stream connection rejected");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
    }
    state.active_sessions.fetch_add(1, Ordering::SeqCst);
    state.last_activity.store(current_timestamp_secs(), Ordering::Relaxed);
    info!("[SSE] --> Client opened SSE event stream handshake");
    let session_event = Event::default().event("endpoint").data("/mcp");

    let stream = stream::once(async move { Ok::<Event, Infallible>(session_event) })
        .chain(stream::pending());

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
        .into_response()
}

/// Non-blocking liveness health check and status report for multi-corpus server.
async fn handle_health_multi(State(state): State<MultiCorpusServerState>) -> Json<Value> {
    info!("[HEALTH] --> Multi-corpus health check probe received");
    state.last_activity.store(current_timestamp_secs(), Ordering::Relaxed);
    let (corpora_count, corpus_names, status) = match state.manager.try_read() {
        Ok(manager) => {
            let names: Vec<String> = manager.corpus_names().into_iter().map(String::from).collect();
            (names.len(), names, "healthy")
        }
        Err(_) => (0, Vec::new(), "busy"),
    };

    let uptime = current_timestamp_secs().saturating_sub(state.started_at);
    let active_indexing = state.active_indexing.read().await.clone();

    let mut response = serde_json::json!({
        "status": if active_indexing.is_some() { "indexing" } else { status },
        "pid": std::process::id(),
        "uptime_seconds": uptime,
        "server": SERVER_NAME,
        "version": SERVER_VERSION,
        "protocol": PROTOCOL_VERSION,
        "corpora_count": corpora_count,
        "corpora": corpus_names,
        "active_sessions": state.active_sessions.load(Ordering::Relaxed)
    });

    if let Some(progress) = active_indexing {
        if let Some(obj) = response.as_object_mut() {
            obj.insert(
                "indexing_progress".to_string(),
                serde_json::to_value(progress).unwrap_or(Value::Null),
            );
        }
    }

    Json(response)
}

/// Administrative endpoint to trigger graceful server shutdown.
async fn handle_shutdown(State(state): State<MultiCorpusServerState>) -> Json<Value> {
    info!("[ADMIN] --> Graceful shutdown signal received via HTTP");
    let _ = state.shutdown_tx.send(true);
    Json(serde_json::json!({
        "status": "shutting_down",
        "pid": std::process::id()
    }))
}

/// Administrative endpoint to trigger incremental delta scan.
async fn handle_admin_sync(
    State(state): State<MultiCorpusServerState>,
    axum::extract::Json(payload): axum::extract::Json<Value>,
) -> Json<Value> {
    info!("[ADMIN] --> Sync request received via HTTP");
    let target = payload.get("corpus").and_then(|v| v.as_str()).map(String::from);
    let mut manager = state.manager.write().await;
    let targets: Vec<String> = if let Some(t) = target {
        if !manager.has_corpus(&t) {
            return Json(serde_json::json!({
                "status": "error",
                "error": format!("corpus '{}' not found", t)
            }));
        }
        vec![t]
    } else {
        manager.corpus_names().into_iter().map(String::from).collect()
    };

    let active_indexing = state.active_indexing.clone();
    let mut results = serde_json::Map::new();

    for name in targets {
        if let Ok(engine) = manager.get_engine_mut(&name) {
            let active_clone = active_indexing.clone();
            let progress_cb: groundcontrol_core::engine::ProgressCallback =
                Arc::new(move |p: &groundcontrol_core::engine::IndexingProgress| {
                    let active = active_clone.clone();
                    let p_cloned = p.clone();
                    tokio::spawn(async move {
                        let mut lock = active.write().await;
                        *lock = Some(p_cloned);
                    });
                });

            match engine.delta_scan_with_progress(50, Some(progress_cb)) {
                Ok(delta) => {
                    results.insert(
                        name.clone(),
                        serde_json::json!({
                            "status": "ok",
                            "new_files": delta.new_files.len(),
                            "modified_files": delta.modified_files.len(),
                            "deleted_files": delta.deleted_files.len()
                        }),
                    );
                }
                Err(e) => {
                    results.insert(
                        name.clone(),
                        serde_json::json!({
                            "status": "error",
                            "error": e.to_string()
                        }),
                    );
                }
            }
        }
    }

    let mut lock = state.active_indexing.write().await;
    *lock = None;

    Json(serde_json::json!({
        "status": "completed",
        "corpora": results
    }))
}
