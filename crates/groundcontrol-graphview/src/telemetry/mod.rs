//! Multi-agent telemetry relay and activation hub.

pub mod relay;

pub use relay::{spawn_telemetry_relay, AgentActivation, TelemetryHub};
