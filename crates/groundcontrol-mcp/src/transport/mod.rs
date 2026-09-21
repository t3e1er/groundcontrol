//! MCP transport: stdio, HTTP, and SSE server implementations.

pub mod dispatch;
pub mod http;
pub mod stdio;

pub use dispatch::{
    format_rpc_response, make_error_response, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    PROTOCOL_VERSION, SERVER_NAME, SERVER_VERSION,
};
pub use http::{
    run_http_server_multi, run_http_server_multi_with_options, MultiCorpusServerState,
    ServerOptions,
};
pub use stdio::{run_stdio_multi, run_stdio_proxy};
