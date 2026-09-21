//! Shared server state for GraphView HTTP and SSE routes.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::layout::GraphLayout;
use crate::loader::CorpusCatalog;
use crate::telemetry::TelemetryHub;

/// Shared application state across all Axum handlers.
#[derive(Clone)]
pub struct ServerState {
    /// Read-only corpus catalog.
    pub catalog: Arc<RwLock<CorpusCatalog>>,
    /// Multi-agent telemetry relay hub.
    pub telemetry: TelemetryHub,
    /// Upstream MCP daemon URL (for query proxying).
    pub daemon_url: String,
    /// Cached precomputed layouts to prevent redundant layout passes.
    pub layout_cache: Arc<RwLock<HashMap<String, GraphLayout>>>,
    /// Client authentication and tracking registry.
    pub clients: Arc<groundcontrol_common::ClientsRegistry>,
}

impl ServerState {
    /// Create new server state.
    pub fn new(catalog: CorpusCatalog, daemon_url: String) -> Self {
        let clients = Arc::new(groundcontrol_common::client::load_clients_config(None));
        Self {
            catalog: Arc::new(RwLock::new(catalog)),
            telemetry: TelemetryHub::new(1024, 256),
            daemon_url,
            layout_cache: Arc::new(RwLock::new(HashMap::new())),
            clients,
        }
    }
}
