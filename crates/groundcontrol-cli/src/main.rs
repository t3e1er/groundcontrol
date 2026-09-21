//! CLI entry point: argument parsing, mode selection, startup orchestration.

mod artifacts;
mod commands;
mod config_cmd;
mod installer;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::Value;

use groundcontrol_common::config::get_logs_cache_dir;
use groundcontrol_core::corpus_manager::CorpusManager;
use groundcontrol_mcp::client::McpClient;
use groundcontrol_mcp::tools::MultiCorpusToolRegistry;
use groundcontrol_mcp::transport;

use commands::client::{handle_client_autopopulate, handle_client_init, handle_client_list};
use commands::index::{handle_index, handle_sync, prompt_bundle_extraction};
use commands::init::handle_init;
use commands::serve::{is_server_healthy, load_or_default_config, parse_corpus_spec, spawn_daemon};

/// Enterprise semantic MCP server for markdown knowledge bases and codebases.
#[derive(Parser, Debug)]
#[command(name = "groundcontrol", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Corpus root(s) to serve, repeatable. Each value is either `name=path` or a
    /// bare `path` (the name is derived from the directory's file name).
    #[arg(long = "corpus", value_name = "NAME=PATH|PATH")]
    corpora: Vec<String>,

    /// Name of the corpus to treat as the default. Defaults to the first `--corpus` added.
    #[arg(long = "default-corpus", value_name = "NAME")]
    default_corpus: Option<String>,

    /// Operating mode. Auto probes port 9090, auto-spawns daemon if needed, and proxies stdio.
    #[arg(long, default_value = "auto")]
    mode: Mode,

    /// Tool exposure profile controlling which tools `tools/list` advertises:
    /// scout (minimal retrieve/navigate), analysis (+ read-only graph/analysis/code
    /// intel), or all (every tool, including writes). Hidden tools still execute if
    /// called directly.
    #[arg(long, default_value = "all")]
    profile: Profile,

    /// Bind address for server mode.
    #[arg(long, default_value = "127.0.0.1:9090")]
    bind: String,

    /// Server endpoint URL when running in client or proxy mode.
    #[arg(long, visible_alias = "remote", default_value = "http://127.0.0.1:9090")]
    server: String,

    /// Tool name to execute when in client mode (e.g. search_hybrid, list_notes).
    #[arg(long)]
    call: Option<String>,

    /// Query shorthand string for search tool execution in client mode.
    #[arg(long)]
    query: Option<String>,

    /// JSON arguments string for tool execution in client mode.
    #[arg(long)]
    args: Option<String>,

    /// Force full re-index on startup.
    #[arg(long)]
    reindex: bool,

    /// Run delta sync (index new/modified files) on startup.
    /// Without --sync or --reindex, the server starts without indexing.
    #[arg(long)]
    sync: bool,

    /// Batch size for paginated indexing and delta scanning.
    #[arg(long, default_value = "50")]
    batch_size: usize,

    /// Do not resume indexing from previous checkpoint; restart from scratch.
    #[arg(long)]
    no_resume: bool,

    /// Fast Mode: skip dense embedding and vector indexing for instant BM25+Graph indexing.
    #[arg(long)]
    fast: bool,

    /// Indexing mode: full or fast. Overrides --fast if set.
    #[arg(long = "index-mode", value_enum)]
    index_mode: Option<CliIndexMode>,

    /// Ingest a SCIP protobuf index file into the knowledge graph on startup.
    #[arg(long = "scip", value_name = "PATH")]
    scip: Option<PathBuf>,

    /// Run server as a detached background daemon with idle auto-shutdown.
    #[arg(long)]
    daemon: bool,

    /// Continuously watch corpus directories for file changes and incrementally reindex.
    #[arg(long)]
    watch: bool,

    /// Idle timeout in minutes before background daemon auto-shuts down (0 = disabled).
    #[arg(long, default_value = "30")]
    idle_timeout: u64,

    /// Log level.
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Log format: text (human-readable) or json (structured JSON Lines).
    /// Defaults to json in daemon mode, text otherwise.
    #[arg(long = "log-format", value_enum)]
    log_format: Option<LogFormat>,

    /// Require valid x-api-key authentication for incoming MCP requests.
    #[arg(long = "require-auth")]
    require_auth: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize a new repository configuration (groundcontrol.toml) with migrated gitignore exclusions.
    Init {
        /// Target repository directory to initialize (defaults to current working directory).
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        /// Force overwrite if groundcontrol.toml already exists.
        #[arg(long)]
        force: bool,
    },
    /// Auto-detect and configure installed coding agents with zero-arg groundcontrol entries.
    Install {
        /// Target installation directory containing groundcontrol binary.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Automatically confirm modifications.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Dry-run mode: show what would change without modifying files.
        #[arg(long)]
        dry_run: bool,
        /// Auto-populate agent steering rules (.cursorrules, GEMINI.md, .windsurfrules, CLAUDE.md).
        #[arg(long)]
        rules: bool,
        /// Optional workspace directory to write local repository rules into (defaults to current directory if --rules is set).
        #[arg(long)]
        rules_dir: Option<PathBuf>,
        /// Fast installation: skip remote checksum download and skip redundant scans.
        #[arg(long)]
        fast: bool,
        /// Target specific coding agents only (comma-separated, e.g. "antigravity,cursor,claude").
        #[arg(long)]
        agents: Option<String>,
        /// Auto-populate agent configuration with dedicated authentication API keys (GROUNDCONTROL_API_KEY).
        #[arg(long)]
        auth: bool,
    },
    /// View and edit groundcontrol configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Export repository index into a compressed team sharing artifact (.groundcontrol/vault.tar.zst).
    ExportArtifact {
        /// Target corpus name (optional, defaults to current repo or default corpus).
        #[arg(long)]
        corpus: Option<String>,
        /// Destination archive file path (default: .groundcontrol/vault.tar.zst).
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },
    /// Import a compressed team sharing artifact (.groundcontrol/vault.tar.zst) into local or central cache.
    ImportArtifact {
        /// Source archive file path to import (.groundcontrol/vault.tar.zst).
        #[arg(long, short = 'i')]
        input: Option<PathBuf>,
        /// Target corpus name override (optional, defaults to archive manifest name).
        #[arg(long)]
        corpus: Option<String>,
    },
    /// Index a corpus directory into central storage (`~/.cache/groundcontrol/indices/<name>/`).
    Index {
        /// Path to the corpus directory to index.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Target corpus name (defaults to directory name if omitted).
        #[arg(long)]
        name: Option<String>,
        /// Force full reindex instead of delta scan.
        #[arg(long)]
        reindex: bool,
        /// Skip dense embeddings and vector indexing for instant BM25+Graph indexing.
        #[arg(long)]
        fast: bool,
        /// Batch size for delta scanning (default 50).
        #[arg(long, default_value = "50")]
        batch_size: usize,
    },
    /// Synchronize all cached corpora with their source directories.
    Sync {
        /// Specific corpus to sync (syncs all if omitted).
        #[arg(long)]
        corpus: Option<String>,
        /// Batch size for delta scanning (default 50).
        #[arg(long, default_value = "50")]
        batch_size: usize,
    },
    /// Launch the standalone 3D knowledge graph visualizer and agent telemetry dashboard.
    Graphview {
        /// Socket address to bind the web dashboard server to.
        #[arg(long, default_value = "127.0.0.1:9091")]
        bind: String,
        /// Path override for corpora storage directory.
        #[arg(long, value_name = "DIR")]
        corpora_dir: Option<PathBuf>,
        /// Upstream groundcontrol MCP daemon HTTP URL for live agent telemetry.
        #[arg(long, default_value = "http://127.0.0.1:9090")]
        daemon: String,
        /// Dedicated authentication key for daemon-to-graphview relay.
        #[arg(long)]
        daemon_key: Option<String>,
    },
    /// Manage AI agent client profiles and authentication keys.
    Client {
        #[command(subcommand)]
        action: ClientAction,
    },
}

#[derive(Subcommand, Debug)]
enum ClientAction {
    /// Initialize a fresh clients.json configuration with secure generated API keys.
    Init {
        /// Force overwrite if clients.json already exists.
        #[arg(long)]
        force: bool,
        /// Destination file path (defaults to ./clients.json).
        #[arg(long, short = 'o')]
        path: Option<PathBuf>,
    },
    /// List configured client profiles and authentication status.
    List,
    /// Auto-populate detected MCP client config files with dedicated authentication credentials.
    Autopopulate {
        /// Dry-run mode: display modifications without writing to disk.
        #[arg(long)]
        dry_run: bool,
        /// Target specific coding agents only (comma-separated, e.g. "antigravity,cursor,claude").
        #[arg(long)]
        agents: Option<String>,
        /// Target installation directory containing groundcontrol binary.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// List active configuration values.
    List,
    /// Get the value of a configuration key.
    Get { key: String },
    /// Set a configuration key to a value.
    Set { key: String, value: String },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CliIndexMode {
    /// Full indexing: Dense Embeddings (Jina ONNX) for Docs; Binary Hamming for Code; BM25 + Graph for both.
    Full,
    /// Fast mode: Algorithmic Binary Hamming + BM25 + Graph across both Docs and Code (Zero ONNX inference).
    Fast,
}

impl From<CliIndexMode> for groundcontrol_common::config::IndexMode {
    fn from(m: CliIndexMode) -> Self {
        match m {
            CliIndexMode::Full => groundcontrol_common::config::IndexMode::Full,
            CliIndexMode::Fast => groundcontrol_common::config::IndexMode::Fast,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Profile {
    /// Minimal retrieve/navigate tool set.
    Scout,
    /// Scout plus read-only graph/validation/analysis/code-intel tools.
    Analysis,
    /// Every registered tool, including mutating/admin tools.
    All,
}

impl From<Profile> for groundcontrol_mcp::tools::ToolProfile {
    fn from(p: Profile) -> Self {
        match p {
            Profile::Scout => groundcontrol_mcp::tools::ToolProfile::Scout,
            Profile::Analysis => groundcontrol_mcp::tools::ToolProfile::Analysis,
            Profile::All => groundcontrol_mcp::tools::ToolProfile::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, clap::ValueEnum)]
enum Mode {
    /// Auto-daemonizing launcher: probes server/health, spawns daemon if needed, proxies stdio.
    Auto,
    /// Stdio MCP transport (single agent, local).
    Local,
    /// Streamable HTTP server (multi-agent, remote).
    Server,
    /// Connect to an existing MCP server as a client.
    Client,
    /// Stdio locally, forwarding to a remote server.
    Proxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LogFormat {
    /// Human-readable plain text format
    Text,
    /// Structured JSON Lines format (one JSON object per line)
    Json,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable full backtraces on panic (writes to stderr, not stdout/JSON-RPC channel).
    std::env::set_var("RUST_BACKTRACE", "1");
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        eprintln!("=== PANIC ===");
        eprintln!("{info}");
        eprintln!("{bt}");
    }));

    let cli = Cli::parse();

    if cli.require_auth {
        std::env::set_var("GROUNDCONTROL_REQUIRE_AUTH", "true");
    }

    // -----------------------------------------------------------------------
    // Subcommand Execution
    // -----------------------------------------------------------------------
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Init { path, force } => {
                return handle_init(path, force);
            }
            Commands::Install { dir, yes, dry_run, rules, rules_dir, fast: _, agents, auth } => {
                let _ = groundcontrol_common::config::ensure_global_config();
                let current_dir = std::env::current_dir().ok();
                let ws_dir = if rules {
                    rules_dir.as_deref().or(current_dir.as_deref())
                } else {
                    rules_dir.as_deref()
                };
                let agent_filters = agents.as_ref().map(|a| {
                    a.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                });
                let summary = installer::run_install(
                    dir.as_deref(),
                    dry_run,
                    yes,
                    rules || rules_dir.is_some(),
                    ws_dir,
                    agent_filters.as_deref(),
                    auth,
                )?;
                if dry_run {
                    println!("\n=== groundcontrol Agent Configuration (DRY RUN) ===");
                    for line in summary.dry_run_detected {
                        println!("  [dry-run] {}", line);
                    }
                } else {
                    println!("\n=== groundcontrol Agent Configuration Complete ===");
                    for line in summary.configured {
                        println!("  [+] Configured {}", line);
                    }
                    for line in summary.rules_configured {
                        println!("  [+] Installed rule {}", line);
                    }
                }
                for line in summary.skipped {
                    println!("  [-] Skipped {}", line);
                }
                return Ok(());
            }
            Commands::Config { action } => match action {
                ConfigAction::List => {
                    config_cmd::handle_config_list()?;
                    return Ok(());
                }
                ConfigAction::Get { key } => {
                    config_cmd::handle_config_get(&key)?;
                    return Ok(());
                }
                ConfigAction::Set { key, value } => {
                    config_cmd::handle_config_set(&key, &value)?;
                    return Ok(());
                }
            },
            Commands::ExportArtifact { corpus, output } => {
                let cwd = std::env::current_dir()?;
                let name = corpus.as_deref().unwrap_or_else(|| {
                    cwd.file_name().and_then(|n| n.to_str()).unwrap_or("default")
                });
                let index_dir = groundcontrol_common::config::get_corpus_index_dir(name);
                if !index_dir.exists() {
                    anyhow::bail!(
                        "No central index found for corpus '{}' at '{}'. Run indexing first.",
                        name,
                        index_dir.display()
                    );
                }
                let exported = artifacts::export_artifact(&index_dir, &cwd, output.as_deref())?;
                println!("[+] Exported artifact to: {}", exported.display());
                return Ok(());
            }
            Commands::ImportArtifact { input, corpus } => {
                let cwd = std::env::current_dir()?;
                let name = corpus.as_deref().unwrap_or_else(|| {
                    cwd.file_name().and_then(|n| n.to_str()).unwrap_or("default")
                });
                let src_path =
                    input.unwrap_or_else(|| cwd.join(".groundcontrol").join("vault.tar.zst"));
                let dest_dir = groundcontrol_common::config::get_corpus_index_dir(name);
                let imported = artifacts::import_artifact(&src_path, &dest_dir)?;
                println!("[+] Imported artifact into central storage: {}", imported.display());
                return Ok(());
            }
            Commands::Index { path, name, reindex, fast, batch_size } => {
                return handle_index(path, name, reindex, fast, batch_size);
            }
            Commands::Sync { corpus, batch_size } => {
                return handle_sync(corpus, batch_size);
            }
            Commands::Graphview { bind, corpora_dir, daemon, daemon_key } => {
                let global = groundcontrol_common::config::load_global_config();
                let effective_bind =
                    if bind == "127.0.0.1:9091" && !global.graphview.bind.is_empty() {
                        global.graphview.bind.as_str()
                    } else {
                        bind.as_str()
                    };
                let effective_daemon =
                    if daemon == "http://127.0.0.1:9090" && !global.graphview.daemon.is_empty() {
                        global.graphview.daemon.as_str()
                    } else {
                        daemon.as_str()
                    };
                let effective_key =
                    daemon_key.or(global.graphview.daemon_key).or(global.auth.daemon_key);

                if let Some(ref k) = effective_key {
                    std::env::set_var("GROUNDCONTROL_INTERNAL_API_KEY", k);
                }
                groundcontrol_graphview::run_graphview_server(
                    effective_bind,
                    corpora_dir,
                    Some(effective_daemon.to_string()),
                )
                .await?;
                return Ok(());
            }
            Commands::Client { action } => match action {
                ClientAction::Init { force, path } => {
                    return handle_client_init(force, path);
                }
                ClientAction::List => {
                    return handle_client_list();
                }
                ClientAction::Autopopulate { dry_run, agents, dir } => {
                    return handle_client_autopopulate(dry_run, agents, dir);
                }
            },
        }
    }

    // -----------------------------------------------------------------------
    // Tracing Configuration
    // -----------------------------------------------------------------------
    let log_format =
        cli.log_format.unwrap_or(if cli.daemon { LogFormat::Json } else { LogFormat::Text });

    if cli.daemon {
        let log_dir = get_logs_cache_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let log_filename = match log_format {
            LogFormat::Json => "groundcontrol-daemon.jsonl",
            LogFormat::Text => "groundcontrol-daemon.log",
        };
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join(log_filename))?;

        match log_format {
            LogFormat::Json => {
                tracing_subscriber::fmt()
                    .json()
                    .flatten_event(true)
                    .with_current_span(false)
                    .with_span_list(false)
                    .with_env_filter(&cli.log_level)
                    .with_writer(log_file)
                    .init();
            }
            LogFormat::Text => {
                tracing_subscriber::fmt()
                    .with_env_filter(&cli.log_level)
                    .with_writer(log_file)
                    .init();
            }
        }
    } else {
        match log_format {
            LogFormat::Json => {
                tracing_subscriber::fmt()
                    .json()
                    .flatten_event(true)
                    .with_current_span(false)
                    .with_span_list(false)
                    .with_env_filter(&cli.log_level)
                    .with_writer(std::io::stderr)
                    .init();
            }
            LogFormat::Text => {
                tracing_subscriber::fmt()
                    .with_env_filter(&cli.log_level)
                    .with_writer(std::io::stderr)
                    .init();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Auto Mode Execution (Zero-Arg Launcher with Daemon Autostart)
    // -----------------------------------------------------------------------
    if matches!(cli.mode, Mode::Auto) {
        let server_url = &cli.server;
        if !is_server_healthy(server_url).await {
            tracing::info!(server = %server_url, "central daemon is down; spawning background server");
            spawn_daemon(&cli.bind, cli.idle_timeout, &cli.log_level, cli.watch, cli.require_auth)?;

            // Poll /health until server is responsive (up to 5 seconds deadline)
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut up = false;
            while Instant::now() < deadline {
                if is_server_healthy(server_url).await {
                    up = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            if !up {
                anyhow::bail!("failed to start groundcontrol background daemon on {}", cli.bind);
            }
        }

        tracing::info!(server = %server_url, "bridging stdio JSON-RPC to central daemon");
        let api_key = std::env::var("GROUNDCONTROL_API_KEY").ok();
        transport::run_stdio_proxy(server_url, api_key.as_deref()).await?;
        return Ok(());
    }

    // -----------------------------------------------------------------------
    // Proxy Mode Execution
    // -----------------------------------------------------------------------
    if matches!(cli.mode, Mode::Proxy) {
        tracing::info!(server = %cli.server, "starting stdio MCP proxy -> remote server");
        let api_key = std::env::var("GROUNDCONTROL_API_KEY").ok();
        transport::run_stdio_proxy(&cli.server, api_key.as_deref()).await?;
        return Ok(());
    }

    // -----------------------------------------------------------------------
    // Client Mode Execution
    // -----------------------------------------------------------------------
    if matches!(cli.mode, Mode::Client) {
        tracing::info!(server = %cli.server, "connecting MCP client");
        let client = McpClient::connect_http(&cli.server);

        // Initialize handshake
        let init_result = client.initialize().await?;
        tracing::debug!(?init_result, "MCP client initialized");

        if let Some(tool_name) = &cli.call {
            let mut arguments: Value = if let Some(args_str) = &cli.args {
                serde_json::from_str(args_str)
                    .map_err(|e| anyhow::anyhow!("invalid JSON in --args: {e}"))?
            } else {
                serde_json::json!({})
            };

            if let Some(q) = &cli.query {
                if let Some(obj) = arguments.as_object_mut() {
                    let _ = obj.insert("query".to_string(), Value::String(q.clone()));
                }
            }

            tracing::info!(tool = %tool_name, ?arguments, "executing tool call");
            let result = client.call_tool(tool_name, arguments).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            // Default action: list available tools
            let tools_res = client.list_tools().await?;
            println!("{}", serde_json::to_string_pretty(&tools_res)?);
        }

        return Ok(());
    }

    // -----------------------------------------------------------------------
    // Local / Server Modes
    // -----------------------------------------------------------------------
    tracing::info!(mode = ?cli.mode, daemon = cli.daemon, "starting groundcontrol engine");

    if cli.require_auth {
        let (config, path) =
            groundcontrol_common::client::ensure_central_clients_config(true, true)?;
        tracing::info!(path = %path.display(), "authenticated mode active; loaded central clients registry");
        if let Some(ref dk) = config.daemon_key {
            tracing::info!(daemon_key = %dk, "daemon-to-graphview relay key active");
        }
    }

    // Build the multi-corpus manager.
    let mut manager = CorpusManager::new();
    let mut corpus_names: Vec<String> = Vec::new();

    if !cli.corpora.is_empty() {
        for spec in &cli.corpora {
            let (name_override, corpus_path, templates_override) = parse_corpus_spec(spec);
            let mut config = load_or_default_config(&corpus_path)?;
            if let Some(name) = name_override {
                config.name = name;
            }
            if let Some(tmpl) = templates_override {
                config.templates_dir = Some(tmpl);
            }
            if let Some(mode) = cli.index_mode {
                config.index_mode = mode.into();
            } else if cli.fast {
                config.index_mode = groundcontrol_common::config::IndexMode::Fast;
            }
            corpus_names.push(config.name.clone());
            let canonical = Path::new(&config.path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&config.path));
            let index_dir = groundcontrol_common::config::get_corpus_index_dir(&config.name);
            prompt_bundle_extraction(&canonical, &config.name, &index_dir)?;
            manager.add_corpus_with_index_dir(config, &index_dir)?;
        }
    } else {
        // When no explicit --corpus arguments are provided, auto-mount all cached corpora
        // from central storage. No arbitrary fallback to current_dir() — if no central corpora
        // exist, the manager starts cleanly with 0 corpora.
        let mounted = manager.mount_all_cached_corpora()?;
        for name in mounted {
            corpus_names.push(name);
        }
    }

    if let Some(default_name) = &cli.default_corpus {
        manager.set_default(default_name)?;
    }

    // Startup indexing applies to every configured corpus.
    if cli.reindex {
        for name in &corpus_names {
            tracing::info!(
                corpus = %name,
                batch_size = cli.batch_size,
                resume = !cli.no_resume,
                "performing full reindex (paginated)"
            );
            let engine = manager.get_engine_mut(name)?;
            let count = engine.full_reindex_paginated(cli.batch_size, !cli.no_resume)?;
            tracing::info!(corpus = %name, count, "reindex complete");
        }
    } else if cli.sync {
        for name in &corpus_names {
            tracing::info!(corpus = %name, batch_size = cli.batch_size, "running delta scan (paginated)");
            let engine = manager.get_engine_mut(name)?;
            let result = engine.delta_scan_paginated(cli.batch_size)?;
            tracing::info!(
                corpus = %name,
                new = result.new_files.len(),
                modified = result.modified_files.len(),
                deleted = result.deleted_files.len(),
                "delta scan complete"
            );
        }
    } else {
        tracing::info!(
            corpora = corpus_names.len(),
            "skipping indexing on startup (use --sync or --reindex, or call sync_corpus/reindex_corpus tools)"
        );
    }

    // Ingest SCIP index if specified
    if let Some(ref scip_path) = cli.scip {
        for name in &corpus_names {
            tracing::info!(corpus = %name, scip = %scip_path.display(), "ingesting SCIP index");
            let engine = manager.get_engine_mut(name)?;
            let stats = engine.ingest_scip(scip_path)?;
            tracing::info!(
                corpus = %name,
                documents = stats.documents_processed,
                definitions = stats.definitions_extracted,
                calls = stats.calls_extracted,
                edges = stats.edges_added,
                "SCIP index ingestion complete"
            );
        }
    }

    // Cross-corpus symbol linking
    if manager.corpus_count() > 1 {
        // Doc-frontmatter side: resolve `implements`/`documents` targets to a
        // unique symbol in a sibling corpus.
        match manager.link_cross_corpus_symbols() {
            Ok(count) => {
                tracing::info!(cross_corpus_edges = count, "cross-corpus symbol linking complete");
            }
            Err(e) => {
                tracing::warn!(error = %e, "cross-corpus symbol linking failed");
            }
        }
        // Code side: resolve captured call/import ExternalRefs to a unique symbol
        // in a sibling corpus, emitting bidirectional cross-corpus edges. Like the
        // doc pass this mutates the in-memory graphs only; the daemon serves those
        // graphs for the session, so no extra persistence is performed here.
        match manager.resolve_external_refs() {
            Ok(count) => {
                tracing::info!(
                    cross_corpus_ref_edges = count,
                    "cross-corpus external-ref resolution complete"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "cross-corpus external-ref resolution failed");
            }
        }
    }

    let registry = MultiCorpusToolRegistry::with_profile(cli.profile.into());

    match cli.mode {
        Mode::Local => {
            tracing::info!(watch = cli.watch, "starting stdio MCP transport");
            let manager_arc = std::sync::Arc::new(tokio::sync::RwLock::new(manager));
            transport::run_stdio_multi(manager_arc, std::sync::Arc::new(registry), cli.watch)
                .await?;
        }
        Mode::Server => {
            tracing::info!(bind = %cli.bind, daemon = cli.daemon, watch = cli.watch, "starting localhost HTTP MCP server");
            let idle_dur = if cli.idle_timeout > 0 {
                Some(Duration::from_secs(cli.idle_timeout * 60))
            } else {
                None
            };
            let options = transport::ServerOptions {
                daemon: cli.daemon,
                idle_timeout: idle_dur,
                watch: cli.watch,
            };
            transport::run_http_server_multi_with_options(&cli.bind, manager, registry, options)
                .await?;
        }
        Mode::Auto | Mode::Client | Mode::Proxy => unreachable!(),
    }

    Ok(())
}
