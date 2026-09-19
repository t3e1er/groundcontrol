//! Configuration CLI command handlers for `ctxvault config` (get/set/list).
//!
//! Backed by `${CTXV_CACHE_DIR}/config.toml`.

use ctxvault_common::config::{get_config_path, load_global_config, save_global_config, IndexMode};

/// Handle `ctxvault config list`.
pub fn handle_config_list() -> anyhow::Result<()> {
    let cfg = load_global_config();
    let path = get_config_path();
    println!("# Configuration: {}", path.display());
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

/// Handle `ctxvault config get <key>`.
pub fn handle_config_get(key: &str) -> anyhow::Result<()> {
    let cfg = load_global_config();
    match key {
        "auto_index" | "server.auto_index" => println!("{}", cfg.server.auto_index),
        "index_mode" | "server.index_mode" => println!("{:?}", cfg.server.index_mode),
        "idle_timeout_mins" | "server.idle_timeout_mins" => {
            println!("{}", cfg.server.idle_timeout_mins)
        }
        "log_level" | "server.log_level" => println!("{}", cfg.server.log_level),
        "bind" | "server.bind" => println!("{}", cfg.server.bind),
        "auth.require_auth" => println!("{}", cfg.auth.require_auth),
        "auth.daemon_key" => println!("{}", cfg.auth.daemon_key.as_deref().unwrap_or("")),
        "graphview.bind" => println!("{}", cfg.graphview.bind),
        "graphview.daemon" => println!("{}", cfg.graphview.daemon),
        "graphview.daemon_key" => println!("{}", cfg.graphview.daemon_key.as_deref().unwrap_or("")),
        "corpora.default" => println!("{}", cfg.corpora.default.as_deref().unwrap_or("")),
        "cache_dir" => println!("{}", cfg.cache_dir.as_deref().unwrap_or("")),
        _ => anyhow::bail!("unknown config key: '{}'", key),
    }
    Ok(())
}

/// Handle `ctxvault config set <key> <val>`.
pub fn handle_config_set(key: &str, val: &str) -> anyhow::Result<()> {
    let mut cfg = load_global_config();
    match key {
        "auto_index" | "server.auto_index" => {
            cfg.server.auto_index = val.parse::<bool>().map_err(|_| {
                anyhow::anyhow!(
                    "invalid boolean value for auto_index: '{}' (expected true/false)",
                    val
                )
            })?;
        }
        "index_mode" | "server.index_mode" => {
            cfg.server.index_mode = match val.to_lowercase().as_str() {
                "full" => IndexMode::Full,
                "fast" => IndexMode::Fast,
                _ => anyhow::bail!("invalid index_mode: '{}' (expected full or fast)", val),
            };
        }
        "idle_timeout_mins" | "server.idle_timeout_mins" => {
            cfg.server.idle_timeout_mins = val.parse::<u64>().map_err(|_| {
                anyhow::anyhow!("invalid integer value for idle_timeout_mins: '{}'", val)
            })?;
        }
        "log_level" | "server.log_level" => {
            cfg.server.log_level = val.to_string();
        }
        "bind" | "server.bind" => {
            cfg.server.bind = val.to_string();
        }
        "auth.require_auth" => {
            cfg.auth.require_auth = val.parse::<bool>().map_err(|_| {
                anyhow::anyhow!(
                    "invalid boolean value for auth.require_auth: '{}' (expected true/false)",
                    val
                )
            })?;
        }
        "auth.daemon_key" => {
            cfg.auth.daemon_key = if val.is_empty() { None } else { Some(val.to_string()) };
        }
        "graphview.bind" => {
            cfg.graphview.bind = val.to_string();
        }
        "graphview.daemon" => {
            cfg.graphview.daemon = val.to_string();
        }
        "graphview.daemon_key" => {
            cfg.graphview.daemon_key = if val.is_empty() { None } else { Some(val.to_string()) };
        }
        "corpora.default" => {
            cfg.corpora.default = if val.is_empty() { None } else { Some(val.to_string()) };
        }
        "cache_dir" => {
            cfg.cache_dir = if val.is_empty() { None } else { Some(val.to_string()) };
        }
        _ => anyhow::bail!("unknown config key: '{}'", key),
    }

    save_global_config(&cfg)?;
    let path = get_config_path();
    println!("[OK] Set {} = {} in {}", key, val, path.display());
    Ok(())
}
