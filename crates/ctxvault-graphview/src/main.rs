//! CLI entrypoint for the standalone `ctxvault-graphview` sidecar.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "ctxvault-graphview",
    about = "Standalone high-performance 3D knowledge graph visualizer and multi-agent activation substrate"
)]
struct Args {
    /// Socket address to bind the web dashboard server to.
    #[arg(long, default_value = "127.0.0.1:9091")]
    bind: String,

    /// Optional override path for the corpora cache storage directory.
    #[arg(long, value_name = "DIR")]
    corpora_dir: Option<PathBuf>,

    /// Upstream ctxvault daemon HTTP URL for live agent telemetry subscription.
    #[arg(long, default_value = "http://127.0.0.1:9090")]
    daemon: String,

    /// Dedicated authentication key for daemon-to-graphview relay.
    #[arg(long)]
    daemon_key: Option<String>,

    /// Log level filter (trace, debug, info, warn, error).
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let global = ctxvault_common::config::load_global_config();
    let effective_log_level = if args.log_level == "info" && !global.server.log_level.is_empty() {
        global.server.log_level.as_str()
    } else {
        args.log_level.as_str()
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(effective_log_level)),
        )
        .init();

    let effective_bind = if args.bind == "127.0.0.1:9091" && !global.graphview.bind.is_empty() {
        global.graphview.bind.as_str()
    } else {
        args.bind.as_str()
    };
    let effective_daemon =
        if args.daemon == "http://127.0.0.1:9090" && !global.graphview.daemon.is_empty() {
            global.graphview.daemon.as_str()
        } else {
            args.daemon.as_str()
        };
    let effective_key = args
        .daemon_key
        .clone()
        .or_else(|| global.graphview.daemon_key.clone())
        .or_else(|| global.auth.daemon_key.clone());

    if let Some(ref k) = effective_key {
        std::env::set_var("CTXV_INTERNAL_API_KEY", k);
    }

    ctxvault_graphview::run_graphview_server(
        effective_bind,
        args.corpora_dir,
        Some(effective_daemon.to_string()),
    )
    .await?;

    Ok(())
}
