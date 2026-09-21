//! MCP JSON-RPC 2.0 protocol dispatch and message handling.
//!
//! Provides the core request parsing, routing, and response construction
//! shared across stdio, HTTP, and SSE server transports.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};

use groundcontrol_common::{Error, Result};
use groundcontrol_core::corpus_manager::CorpusManager;

use crate::tools::MultiCorpusToolRegistry;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// MCP server information reported during initialization.
pub const SERVER_NAME: &str = "groundcontrol";
/// MCP server version string.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// MCP protocol version string.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

/// Inbound JSON-RPC 2.0 request (or notification).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version string (must be "2.0").
    #[allow(dead_code)]
    pub jsonrpc: String,
    /// Request identifier (omitted for notifications).
    pub id: Option<Value>,
    /// Remote method name.
    pub method: String,
    /// Optional parameter object or array.
    pub params: Option<Value>,
}

/// Outbound JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version string ("2.0").
    pub jsonrpc: String,
    /// Request identifier matching the request.
    pub id: Value,
    /// Successful result payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error payload if execution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Additional error details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Check if a multi-corpus JSON-RPC request is read-only.
pub fn is_read_only_request_multi(
    req: &JsonRpcRequest,
    registry: &MultiCorpusToolRegistry,
) -> bool {
    match req.method.as_str() {
        "initialize" | "server/discover" | "tools/list" | "ping" | "roots/list" => true,
        "tools/call" => {
            if let Some(params) = &req.params {
                if let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) {
                    registry.is_read_only(tool_name)
                } else {
                    false
                }
            } else {
                false
            }
        }
        method if method.starts_with("notifications/") || method.starts_with("$/") => true,
        _ => false,
    }
}

/// Route a JSON-RPC request for multi-corpus reading concurrently.
pub fn dispatch_multi_read(
    request: &JsonRpcRequest,
    manager: &CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    debug!(method = %request.method, "dispatching MCP multi read request");
    match request.method.as_str() {
        "initialize" | "server/discover" => handle_initialize(),
        "tools/list" => handle_tools_list_multi(registry),
        "tools/call" => handle_tools_call_multi_read(request, manager, registry),
        "ping" => Ok(serde_json::json!({})),
        "roots/list" => Ok(serde_json::json!({ "roots": [] })),
        method if method.starts_with("notifications/") || method.starts_with("$/") => {
            debug!(method, "handling MCP notification");
            Ok(Value::Null)
        }
        other => {
            warn!(method = other, "unknown method");
            Err(Error::NotFound(format!("method not found: {other}")))
        }
    }
}

/// Route a JSON-RPC request for multi-corpus mutating with exclusive lock.
pub fn dispatch_multi_write(
    request: &JsonRpcRequest,
    manager: &mut CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    debug!(method = %request.method, "dispatching MCP multi write request");
    match request.method.as_str() {
        "initialize" | "server/discover" => handle_initialize(),
        "tools/list" => handle_tools_list_multi(registry),
        "tools/call" => handle_tools_call_multi_write(request, manager, registry),
        "ping" => Ok(serde_json::json!({})),
        "roots/list" => Ok(serde_json::json!({ "roots": [] })),
        method if method.starts_with("notifications/") || method.starts_with("$/") => {
            debug!(method, "handling MCP notification");
            Ok(Value::Null)
        }
        other => {
            warn!(method = other, "unknown method");
            Err(Error::NotFound(format!("method not found: {other}")))
        }
    }
}

/// Route a JSON-RPC request with multi-corpus routing support.
pub fn dispatch_multi(
    request: &JsonRpcRequest,
    manager: &mut CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    dispatch_multi_write(request, manager, registry)
}

// ---------------------------------------------------------------------------
// MCP Handlers
// ---------------------------------------------------------------------------

/// Respond to `initialize` with server capabilities.
pub fn handle_initialize() -> Result<Value> {
    Ok(serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION
        }
    }))
}

/// Respond to `tools/list` with all registered tools (multi-corpus).
pub fn handle_tools_list_multi(registry: &MultiCorpusToolRegistry) -> Result<Value> {
    let tools: Vec<Value> = registry
        .list()
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema
            })
        })
        .collect();

    Ok(serde_json::json!({ "tools": tools }))
}

/// Dispatch a `tools/call` request with multi-corpus routing (read-only).
pub fn handle_tools_call_multi_read(
    request: &JsonRpcRequest,
    manager: &CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    let params = request.params.as_ref().ok_or_else(|| Error::Config("missing params".into()))?;

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Config("missing tool name".into()))?;

    let mut arguments =
        params.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));

    if let Value::Object(ref mut map) = arguments {
        map.entry("format").or_insert_with(|| Value::String("lean".to_string()));
    }

    let result = registry.execute_read(tool_name, manager, arguments)?;

    let text = match result {
        Value::String(s) => s,
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    };

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    }))
}

/// Dispatch a `tools/call` request with multi-corpus routing (mutating).
pub fn handle_tools_call_multi_write(
    request: &JsonRpcRequest,
    manager: &mut CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    let params = request.params.as_ref().ok_or_else(|| Error::Config("missing params".into()))?;

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Config("missing tool name".into()))?;

    let arguments =
        params.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));

    let result = registry.execute_write(tool_name, manager, arguments)?;

    let text = match result {
        Value::String(s) => s,
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    };

    Ok(serde_json::json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    }))
}

/// Dispatch a `tools/call` request with multi-corpus routing.
pub fn handle_tools_call_multi(
    request: &JsonRpcRequest,
    manager: &mut CorpusManager,
    registry: &MultiCorpusToolRegistry,
) -> Result<Value> {
    handle_tools_call_multi_write(request, manager, registry)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a JSON-RPC error response.
pub fn make_error_response(id: Value, code: i64, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError { code, message: message.to_string(), data: None }),
    }
}

/// Format the result of a `dispatch()` call into a `JsonRpcResponse`.
pub fn format_rpc_response(id: Value, result: Result<Value>) -> JsonRpcResponse {
    match result {
        Ok(val) => {
            JsonRpcResponse { jsonrpc: "2.0".to_string(), id, result: Some(val), error: None }
        }
        Err(e) => {
            let code = match &e {
                Error::NotFound(_) => -32601, // method not found
                _ => -32603,                  // internal error
            };
            make_error_response(id, code, &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_initialize() {
        let result = handle_initialize().unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(result["serverInfo"]["version"], SERVER_VERSION);
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn test_dispatch_tools_list() {
        let registry = MultiCorpusToolRegistry::new();

        let result = handle_tools_list_multi(&registry).unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());

        for tool in tools {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[test]
    fn test_dispatch_unknown_method() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            method: "nonexistent/method".to_string(),
            params: None,
        };

        let result = dispatch_method_only(&request.method);
        assert!(result.is_err());

        if let Err(Error::NotFound(msg)) = result {
            assert!(msg.contains("nonexistent/method"));
        } else {
            panic!("Expected NotFound error");
        }
    }

    #[test]
    fn test_make_error_response() {
        let resp = make_error_response(Value::Number(42.into()), -32600, "invalid request");
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, Value::Number(42.into()));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "invalid request");
    }

    #[test]
    fn test_parse_json_rpc_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn test_parse_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.id.is_none());
    }

    fn dispatch_method_only(method: &str) -> Result<Value> {
        match method {
            "initialize" => handle_initialize(),
            "ping" => Ok(serde_json::json!({})),
            other => Err(Error::NotFound(format!("method not found: {other}"))),
        }
    }
}
