//! MCP Client transport trait definition.

use async_trait::async_trait;
use serde_json::Value;

use groundcontrol_common::Result;

/// Pluggable asynchronous transport layer for an MCP client.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC 2.0 request and await the corresponding result value.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value>;

    /// Send a one-way notification without expecting a response.
    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let _ = method;
        let _ = params;
        Ok(())
    }

    /// Cleanly close the transport.
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}
