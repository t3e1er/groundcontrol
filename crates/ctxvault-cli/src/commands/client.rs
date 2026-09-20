//! `client` subcommand handlers.

use std::path::PathBuf;

use crate::installer;

/// Initialize a fresh clients.json configuration with secure generated API keys.
pub fn handle_client_init(force: bool, path: Option<PathBuf>) -> anyhow::Result<()> {
    let dest = path.unwrap_or_else(|| PathBuf::from("clients.json"));
    if dest.exists() && !force {
        eprintln!("[-] '{}' already exists. Use --force to overwrite.", dest.display());
        return Ok(());
    }
    let config = ctxvault_common::client::generate_default_config();
    let json_str = serde_json::to_string_pretty(&config)?;
    std::fs::write(&dest, json_str)?;
    println!("[+] Generated fresh client configuration at '{}'", dest.display());
    println!("    Auth mode: require_auth = false (default zero-auth)");
    println!("    Generated API keys:");
    for c in &config.clients {
        if let Some(ref k) = c.key {
            println!("    * {:<18} (ID: {:<12}) -> x-api-key: {}", c.name, c.id, k);
        }
    }
    if let Some(ref dk) = config.daemon_key {
        println!("    * Dedicated Daemon Relay Key           -> x-api-key: {}", dk);
    }
    Ok(())
}

/// List configured client profiles and authentication status.
pub fn handle_client_list() -> anyhow::Result<()> {
    let config = ctxvault_common::client::load_clients_config(None);
    println!("\n=== ctxvault Client Authentication Registry ===");
    println!(
        "  Authentication Required: {}",
        if config.require_auth { "YES (strict)" } else { "NO (default zero-auth)" }
    );
    if let Some(ref dk) = config.daemon_key {
        println!("  Daemon Relay Key:        {}", dk);
    } else {
        println!("  Daemon Relay Key:        None (open)");
    }
    println!("\n  Registered Clients ({}):", config.clients.len());
    for c in &config.clients {
        let key_display = c.key.as_deref().unwrap_or("<none>");
        println!("  * {:<20} ID: {:<12} Color: {:<8} Key: {}", c.name, c.id, c.color, key_display);
    }
    Ok(())
}

/// Auto-populate detected MCP client config files with dedicated authentication credentials.
pub fn handle_client_autopopulate(
    dry_run: bool,
    agents: Option<String>,
    dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let agent_filters = agents.as_ref().map(|a| {
        a.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>()
    });
    let summary =
        installer::autopopulate_clients(dir.as_deref(), dry_run, agent_filters.as_deref())?;
    if dry_run {
        println!("\n=== ctxvault Client Credentials Autopopulate (DRY RUN) ===");
        for line in summary.dry_run_detected {
            println!("  [dry-run] {}", line);
        }
    } else {
        println!("\n=== ctxvault Client Credentials Autopopulate Complete ===");
        for line in summary.configured {
            println!("  [+] Autopopulated {}", line);
        }
    }
    for line in summary.skipped {
        println!("  [-] Skipped {}", line);
    }
    Ok(())
}
