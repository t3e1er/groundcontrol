//! Server-Sent Events (SSE) telemetry relay for AI agent graph activations.
//!
//! Subscribes to the core `groundcontrol` MCP daemon and relays real-time agent
//! activity (tool invocations, searched queries, accessed nodes) to connected
//! browsers.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

/// Structured agent activity event emitted on MCP tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivation {
    /// Timestamp in UNIX milliseconds.
    #[serde(default)]
    pub timestamp: u64,
    /// Tool name (e.g. search, get_snippet, graph_match, write_note).
    pub tool: String,
    /// Connected AI client ID (e.g. "antigravity", "claude", "gemini").
    #[serde(default, alias = "client")]
    pub client_id: Option<String>,
    /// Connected AI client display name.
    #[serde(default)]
    pub client_name: Option<String>,
    /// Associated theme color (e.g. "#38bdf8").
    #[serde(default)]
    pub client_color: Option<String>,
    /// Target corpus name, if scoped.
    #[serde(default)]
    pub corpus: Option<String>,
    /// Search query or Cypher pattern, if applicable.
    #[serde(default, alias = "summary")]
    pub query: Option<String>,
    /// Repository paths of nodes activated or touched.
    #[serde(default, alias = "hits")]
    pub paths: Vec<String>,
    /// Execution duration in milliseconds.
    #[serde(default)]
    pub duration_ms: f64,
    /// Whether the tool execution succeeded.
    #[serde(default = "default_true")]
    pub success: bool,
}

fn default_true() -> bool {
    true
}

/// Telemetry hub holding circular ring buffer and broadcast channel.
#[derive(Clone)]
pub struct TelemetryHub {
    tx: broadcast::Sender<AgentActivation>,
    history: Arc<RwLock<Vec<AgentActivation>>>,
    max_history: usize,
}

impl TelemetryHub {
    /// Create a new telemetry hub.
    pub fn new(capacity: usize, max_history: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx, history: Arc::new(RwLock::new(Vec::with_capacity(max_history))), max_history }
    }

    /// Broadcast a new agent activation and append to ring buffer.
    pub async fn publish(&self, activation: AgentActivation) {
        let mut hist = self.history.write().await;
        if hist.len() >= self.max_history {
            hist.remove(0);
        }
        hist.push(activation.clone());
        let _ = self.tx.send(activation);
    }

    /// Subscribe to real-time activation events.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentActivation> {
        self.tx.subscribe()
    }

    /// Get a snapshot of recent activations.
    pub async fn recent_history(&self) -> Vec<AgentActivation> {
        self.history.read().await.clone()
    }
}

/// Background task to consume SSE events from core MCP daemon and relay to hub.
pub fn spawn_telemetry_relay(daemon_url: String, hub: TelemetryHub, daemon_key: Option<String>) {
    tokio::spawn(async move {
        let endpoint = format!("{}/events/activations", daemon_url.trim_end_matches('/'));
        info!(daemon = %daemon_url, "Starting SSE telemetry relay background listener");

        let client = reqwest::Client::new();
        let mut backoff = Duration::from_millis(500);

        loop {
            debug!(endpoint = %endpoint, "Connecting to daemon SSE stream...");
            let mut req = client.get(&endpoint);
            if let Some(ref k) = daemon_key {
                req = req.header("x-api-key", k);
            }
            match req.send().await {
                Ok(mut response) if response.status().is_success() => {
                    info!("Connected to groundcontrol daemon telemetry stream");
                    backoff = Duration::from_millis(500);

                    loop {
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                let text = String::from_utf8_lossy(&chunk);
                                for line in text.lines() {
                                    let trimmed = line.trim();
                                    if let Some(data) = trimmed.strip_prefix("data:") {
                                        if let Ok(activation) =
                                            serde_json::from_str::<AgentActivation>(data.trim())
                                        {
                                            hub.publish(activation).await;
                                        }
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                warn!(error = %e, "SSE chunk read error");
                                break;
                            }
                        }
                    }
                }
                Ok(resp) => {
                    debug!(status = %resp.status(), "Daemon telemetry endpoint not ready");
                }
                Err(e) => {
                    debug!(error = %e, "Daemon connection failed; will retry");
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
    });
}
