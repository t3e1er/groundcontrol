//! MCP HTTP Transport Client.
//!
//! Communicates with an MCP server over HTTP JSON-RPC 2.0.

use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use groundcontrol_common::{Error, Result};

use crate::client::transport::McpTransport;
use crate::transport::dispatch::{JsonRpcRequest, JsonRpcResponse};

/// HTTP transport client for connecting to remote or localhost MCP servers.
pub struct HttpMcpTransport {
    client: reqwest::Client,
    endpoint_url: String,
    api_key: Option<String>,
    next_id: AtomicI64,
}

impl HttpMcpTransport {
    /// Create a new HTTP transport targeting the given base URL or endpoint.
    pub fn new(url: &str) -> Self {
        let endpoint_url = if url.ends_with("/mcp") || url.ends_with("/jsonrpc") {
            url.to_string()
        } else if url.ends_with('/') {
            format!("{url}mcp")
        } else {
            format!("{url}/mcp")
        };

        Self {
            client: reqwest::Client::new(),
            endpoint_url,
            api_key: std::env::var("GROUNDCONTROL_API_KEY")
                .or_else(|_| std::env::var("GC_API_KEY"))
                .or_else(|_| std::env::var("CTXV_API_KEY"))
                .ok(),
            next_id: AtomicI64::new(1),
        }
    }

    /// Set an explicit API key for `x-api-key` header authorization.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Return the target endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint_url
    }
}

#[async_trait]
impl McpTransport for HttpMcpTransport {
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(id.into())),
            method: method.to_string(),
            params,
        };

        let mut req_builder = self.client.post(&self.endpoint_url).json(&request);
        if let Some(ref k) = self.api_key {
            req_builder = req_builder.header("x-api-key", k);
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| Error::Config(format!("HTTP request error: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Config(format!("HTTP error {status}: {text}")));
        }

        let rpc_res: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| Error::Config(format!("Failed to parse JSON-RPC response: {e}")))?;

        if let Some(err) = rpc_res.error {
            return Err(Error::Config(format!("MCP server error [{}]: {}", err.code, err.message)));
        }

        rpc_res
            .result
            .ok_or_else(|| Error::Config("Missing result in JSON-RPC response".to_string()))
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        };

        let mut req_builder = self.client.post(&self.endpoint_url).json(&request);
        if let Some(ref k) = self.api_key {
            req_builder = req_builder.header("x-api-key", k);
        }

        let _ = req_builder
            .send()
            .await
            .map_err(|e| Error::Config(format!("HTTP notification error: {e}")))?;

        Ok(())
    }
}
