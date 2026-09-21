//! MCP Stdio Transport Client.
//!
//! Spawns and manages a child process running an MCP server over stdio JSON-RPC.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use groundcontrol_common::{Error, Result};

use crate::client::transport::McpTransport;
use crate::transport::dispatch::{JsonRpcRequest, JsonRpcResponse};

/// Stdio transport client managing a child process MCP server.
pub struct StdioMcpTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    #[allow(dead_code)]
    child: Arc<Mutex<Child>>,
    next_id: AtomicI64,
}

impl StdioMcpTransport {
    /// Spawn a child process with stdin/stdout piped.
    pub fn spawn(command: &str, args: &[&str], cwd: Option<&Path>) -> Result<Self> {
        let mut cmd = Command::new(command);
        let _ =
            cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());

        if let Some(dir) = cwd {
            let _ = cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().map_err(Error::Io)?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Config("Failed to open child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Config("Failed to open child stdout".to_string()))?;

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicI64::new(1),
        })
    }
}

#[async_trait]
impl McpTransport for StdioMcpTransport {
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(id.into())),
            method: method.to_string(),
            params,
        };

        let request_str = serde_json::to_string(&request)
            .map_err(|e| Error::Config(format!("Failed to serialize request: {e}")))?;

        // Write request to child stdin
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(format!("{request_str}\n").as_bytes()).await.map_err(Error::Io)?;
            stdin.flush().await.map_err(Error::Io)?;
        }

        // Read response line from child stdout
        let mut line = String::new();
        {
            let mut stdout = self.stdout.lock().await;
            let _ = stdout.read_line(&mut line).await.map_err(Error::Io)?;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err(Error::Config("Empty response received from child stdio".to_string()));
        }

        let rpc_res: JsonRpcResponse = serde_json::from_str(trimmed).map_err(|e| {
            Error::Config(format!("Failed to parse JSON-RPC response from stdio: {e}"))
        })?;

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

        let request_str = serde_json::to_string(&request)
            .map_err(|e| Error::Config(format!("Failed to serialize notification: {e}")))?;

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(format!("{request_str}\n").as_bytes()).await.map_err(Error::Io)?;
        stdin.flush().await.map_err(Error::Io)?;

        Ok(())
    }

    async fn close(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        Ok(())
    }
}
