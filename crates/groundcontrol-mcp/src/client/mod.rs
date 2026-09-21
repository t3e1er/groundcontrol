//! Full-Featured Model Context Protocol (MCP) Client.
//!
//! Provides strongly-typed client access to any standard MCP server
//! over HTTP or Stdio transports.

pub mod http;
pub mod stdio;
pub mod transport;

use serde_json::Value;
use std::path::Path;

use groundcontrol_common::Result;

pub use http::HttpMcpTransport;
pub use stdio::StdioMcpTransport;
pub use transport::McpTransport;

/// Full-featured MCP client supporting pluggable transports.
pub struct McpClient<T: McpTransport> {
    transport: T,
}

impl McpClient<HttpMcpTransport> {
    /// Connect to an MCP server via localhost or remote HTTP endpoint.
    pub fn connect_http(url: &str) -> Self {
        Self { transport: HttpMcpTransport::new(url) }
    }

    /// Connect to an MCP server via HTTP with an explicit API key.
    pub fn connect_http_with_key(url: &str, api_key: impl Into<String>) -> Self {
        Self { transport: HttpMcpTransport::new(url).with_api_key(api_key) }
    }
}

impl McpClient<StdioMcpTransport> {
    /// Connect to an MCP server by spawning a child process over stdio.
    pub fn spawn_stdio(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<Self> {
        let transport = StdioMcpTransport::spawn(command, args, cwd)?;
        Ok(Self { transport })
    }
}

impl<T: McpTransport> McpClient<T> {
    /// Create a client instance with a custom transport.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Access the underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    // -----------------------------------------------------------------------
    // Core MCP Protocol Methods
    // -----------------------------------------------------------------------

    /// Initialize the MCP connection and perform protocol negotiation.
    pub async fn initialize(&self) -> Result<Value> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": { "listChanged": false }
            },
            "clientInfo": {
                "name": "groundcontrol-client",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let result = self.transport.send_request("initialize", Some(params)).await?;
        self.transport.send_notification("notifications/initialized", None).await?;
        Ok(result)
    }

    /// Ping the MCP server for liveness.
    pub async fn ping(&self) -> Result<()> {
        let _ = self.transport.send_request("ping", None).await?;
        Ok(())
    }

    /// List all tools exposed by the MCP server.
    pub async fn list_tools(&self) -> Result<Value> {
        self.transport.send_request("tools/list", None).await
    }

    /// Execute a tool by name with arguments.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });
        self.transport.send_request("tools/call", Some(params)).await
    }

    /// Cleanly close the client transport.
    pub async fn close(&self) -> Result<()> {
        self.transport.close().await
    }

    // -----------------------------------------------------------------------
    // High-Level Domain Helpers
    // -----------------------------------------------------------------------

    /// Execute a 4-modality hybrid search (BM25 + vector + graph expansion + RRF)
    /// via the consolidated `search` tool (`mode = "hybrid"`).
    pub async fn search_hybrid(
        &self,
        query: &str,
        limit: Option<usize>,
        depth: Option<&str>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({ "query": query, "mode": "hybrid" });
        if let Some(l) = limit {
            args["limit"] = l.into();
        }
        if let Some(d) = depth {
            args["depth"] = d.into();
        }
        self.call_tool("search", args).await
    }

    /// Execute a BM25 keyword full-text search via the `search` tool
    /// (`mode = "bm25"`).
    pub async fn search_bm25(&self, query: &str, limit: Option<usize>) -> Result<Value> {
        let mut args = serde_json::json!({ "query": query, "mode": "bm25" });
        if let Some(l) = limit {
            args["limit"] = l.into();
        }
        self.call_tool("search", args).await
    }

    /// Execute a dense vector similarity search via the `search` tool
    /// (`mode = "semantic"`).
    pub async fn search_semantic(
        &self,
        query: &str,
        limit: Option<usize>,
        depth: Option<&str>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({ "query": query, "mode": "semantic" });
        if let Some(l) = limit {
            args["limit"] = l.into();
        }
        if let Some(d) = depth {
            args["depth"] = d.into();
        }
        self.call_tool("search", args).await
    }

    /// Execute a typed graph expansion search via the `search` tool
    /// (`mode = "graph"`).
    pub async fn search_graph(&self, query: &str, graph_depth: Option<usize>) -> Result<Value> {
        let mut args = serde_json::json!({ "query": query, "mode": "graph" });
        if let Some(d) = graph_depth {
            args["graph_depth"] = d.into();
        }
        self.call_tool("search", args).await
    }

    /// Read the complete content of a note or source file via `read_file`.
    pub async fn read_file(&self, path: &str) -> Result<Value> {
        self.call_tool("read_file", serde_json::json!({ "path": path })).await
    }

    /// Read multiple files in batch via `read_file`.
    pub async fn read_files(&self, paths: &[&str]) -> Result<Value> {
        self.call_tool("read_file", serde_json::json!({ "paths": paths })).await
    }

    /// List notes in the corpus with their metadata.
    pub async fn list_notes(&self) -> Result<Value> {
        self.call_tool("list_notes", serde_json::json!({})).await
    }

    /// Create or update a note via `write_note`.
    pub async fn write_note(
        &self,
        path: &str,
        content: &str,
        mode: Option<&str>,
        template: Option<&str>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "path": path,
            "content": content
        });
        if let Some(m) = mode {
            args["mode"] = m.into();
        }
        if let Some(t) = template {
            args["template"] = t.into();
        }
        self.call_tool("write_note", args).await
    }

    /// Delete a note and remove it from disk and all indices.
    pub async fn delete_note(&self, path: &str) -> Result<Value> {
        self.call_tool("delete_note", serde_json::json!({ "path": path })).await
    }

    /// Retrieve corpus index status and document statistics via the consolidated
    /// `status` tool (multi-corpus overview when no corpus is targeted).
    pub async fn get_status(&self) -> Result<Value> {
        self.call_tool("status", serde_json::json!({})).await
    }

    /// Validate note schema conformance or corpus taxonomy via `validate`.
    pub async fn validate(
        &self,
        path: Option<&str>,
        check_taxonomy: Option<bool>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({});
        if let Some(p) = path {
            args["path"] = p.into();
        }
        if let Some(t) = check_taxonomy {
            args["check_taxonomy"] = t.into();
        }
        self.call_tool("validate", args).await
    }

    /// List all templates registered in the corpus.
    pub async fn list_templates(&self) -> Result<Value> {
        self.call_tool("list_templates", serde_json::json!({})).await
    }

    /// Execute a linear Cypher-Lite graph path query via `graph_match`.
    pub async fn graph_match(
        &self,
        pattern: &str,
        edge_class: Option<&str>,
        where_filter: Option<&str>,
        limit: Option<usize>,
        max_depth: Option<usize>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({ "pattern": pattern });
        if let Some(c) = edge_class {
            args["edge_class"] = c.into();
        }
        if let Some(w) = where_filter {
            args["where"] = w.into();
        }
        if let Some(l) = limit {
            args["limit"] = l.into();
        }
        if let Some(d) = max_depth {
            args["max_depth"] = d.into();
        }
        self.call_tool("graph_match", args).await
    }
}
