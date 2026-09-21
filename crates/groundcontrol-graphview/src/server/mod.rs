//! Axum HTTP and SSE server implementation for GraphView.

pub mod routes;
pub mod state;

pub use routes::create_router;
pub use state::ServerState;
