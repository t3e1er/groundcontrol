//! `groundcontrol-graphview`: Standalone 3D knowledge graph visualizer and agent telemetry substrate.
//!
//! Provides a zero-overhead sidecar service for visualizing massive ($1\text{M}+$ node)
//! polyglot code and markdown documentation knowledge graphs in real-time 3D WebGL.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod layout;
pub mod loader;
pub mod server;
pub mod telemetry;
pub mod wire;

use std::path::PathBuf;

pub use error::{GraphViewError, Result};
pub use layout::GraphLayout;
pub use loader::{CorpusCatalog, CorpusSnapshot};
pub use server::{create_router, ServerState};
pub use telemetry::{spawn_telemetry_relay, AgentActivation, TelemetryHub};
pub use wire::BinaryWireEncoder;

use tracing::info;

/// Run the GraphView sidecar HTTP server until aborted.
pub async fn run_graphview_server(
    bind_addr: &str,
    corpora_dir: Option<PathBuf>,
    daemon_url: Option<String>,
) -> Result<()> {
    let catalog = CorpusCatalog::load_all(corpora_dir)?;
    let daemon = daemon_url.unwrap_or_else(|| "http://127.0.0.1:9090".to_string());

    let state = ServerState::new(catalog, daemon.clone());
    let daemon_key = state.clients.daemon_key.clone();

    // Spawn background SSE relay to daemon
    spawn_telemetry_relay(daemon, state.telemetry.clone(), daemon_key);

    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;

    eprintln!(
        "\n\
        +========================================================================+\n\
        |  GroundControl GraphView 3D Dashboard is Running                            |\n\
        |                                                                        |\n\
        |  * Web UI:        http://{:<45}|\n\
        |  * API Overview:  http://{:<45}/api/graph/overview     |\n\
        |  * Telemetry SSE: http://{:<45}/api/events/activations |\n\
        +========================================================================+\n",
        local_addr, local_addr, local_addr
    );
    info!(addr = %local_addr, "groundcontrol-graphview server listening");

    axum::serve(listener, app).await?;
    Ok(())
}
