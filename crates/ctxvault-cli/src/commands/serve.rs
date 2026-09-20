//! Server, proxy, daemon, and client lifecycle orchestration.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(not(windows))]
use ctxvault_common::config::get_logs_cache_dir;
use ctxvault_common::config::CorpusConfig;

/// Check if a ctxvault server /health endpoint is alive.
pub async fn is_server_healthy(server_url: &str) -> bool {
    let health_url = format!("{}/health", server_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(Duration::from_millis(150)).build();
    if let Ok(c) = client {
        if let Ok(resp) = c.get(&health_url).send().await {
            return resp.status().is_success();
        }
    }
    false
}

/// Spawn the background server daemon in a detached process.
pub fn spawn_daemon(
    bind_addr: &str,
    idle_timeout: u64,
    log_level: &str,
    watch: bool,
    require_auth: bool,
) -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(current_exe);
    cmd.args([
        "--mode",
        "server",
        "--bind",
        bind_addr,
        "--daemon",
        "--log-level",
        log_level,
        "--log-format",
        "json",
    ]);
    if watch {
        cmd.arg("--watch");
    }
    if idle_timeout > 0 {
        cmd.arg(format!("--idle-timeout={}", idle_timeout));
    }
    if require_auth {
        cmd.arg("--require-auth");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const DETACHED_PROCESS: u32 = 0x00000008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    #[cfg(not(windows))]
    {
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        let log_dir = get_logs_cache_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        if let Ok(log_file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("ctxvault-daemon.jsonl"))
        {
            cmd.stderr(log_file);
        } else {
            cmd.stderr(std::process::Stdio::null());
        }
    }

    cmd.spawn()?;
    Ok(())
}

/// Parse a `--corpus` spec of the form `name=path[,templates=rel_path]` or a bare `path`.
pub fn parse_corpus_spec(spec: &str) -> (Option<String>, PathBuf, Option<String>) {
    let mut parts = spec.split(',');
    let first = parts.next().unwrap_or(spec);
    let mut templates_override = None;

    for opt in parts {
        if let Some((k, v)) = opt.split_once('=') {
            let key = k.trim();
            if key == "templates" || key == "templates_dir" {
                templates_override = Some(v.trim().to_string());
            }
        }
    }

    let (name, path) = match first.split_once('=') {
        Some((name, path)) if !name.is_empty() => {
            (Some(name.trim().to_string()), PathBuf::from(path.trim()))
        }
        _ => (None, PathBuf::from(first.trim())),
    };

    (name, path, templates_override)
}

/// Load `ctxvault.toml` from the corpus directory, or create a default config.
pub fn load_or_default_config(corpus_path: &Path) -> anyhow::Result<CorpusConfig> {
    let config_path = corpus_path.join("ctxvault.toml");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let mut config: CorpusConfig = toml::from_str(&content)?;
        config.path = corpus_path.to_string_lossy().replace('\\', "/");
        Ok(config)
    } else {
        Ok(CorpusConfig {
            name: corpus_path.file_name().and_then(|n| n.to_str()).unwrap_or("default").to_string(),
            path: corpus_path.to_string_lossy().to_string(),
            mode: ctxvault_common::config::CorpusMode::ReadWrite,
            index_mode: ctxvault_common::config::IndexMode::Full,
            chunking: ctxvault_common::config::ChunkingConfig::default(),
            embedding: ctxvault_common::config::EmbeddingConfig::default(),
            graph: ctxvault_common::config::GraphConfig {
                edge_types: vec![
                    ctxvault_common::config::EdgeTypeConfig {
                        name: "Wikilink".to_string(),
                        source: ctxvault_common::config::EdgeSource::Wikilink,
                        weight: 1.0,
                        bidirectional: false,
                        field: None,
                        direction: None,
                        max_frequency: None,
                        class: None,
                        description: Some("Direct wikilink connection between notes".to_string()),
                        allowed_source_templates: None,
                        allowed_target_templates: None,
                    },
                    ctxvault_common::config::EdgeTypeConfig {
                        name: "SharedTag".to_string(),
                        source: ctxvault_common::config::EdgeSource::Tag,
                        weight: 0.5,
                        bidirectional: true,
                        field: None,
                        direction: None,
                        max_frequency: Some(15),
                        class: None,
                        description: Some("Shared thematic tag between notes".to_string()),
                        allowed_source_templates: None,
                        allowed_target_templates: None,
                    },
                ],
            },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        })
    }
}
