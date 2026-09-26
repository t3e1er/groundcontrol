//! Central corpus configuration and index lifecycle command handlers (`groundcontrol corpus list|add|remove|default|sync|export|import`).

use std::path::PathBuf;

use anyhow::Context;

use groundcontrol_common::config::{
    calculate_corpus_disk_usage, format_bytes, get_corpus_index_dir, load_global_config,
    save_global_config, IndexMode, RegisteredCorpus,
};

use super::index::{handle_index, handle_sync};

/// List all registered and cached corpora with disk footprints and indexing modes.
pub fn handle_corpus_list() -> anyhow::Result<()> {
    let global = load_global_config();
    let default_name = global.corpora.default.as_deref().unwrap_or("");

    println!("Central Multi-Corpus Registry (${{GROUNDCONTROL_CACHE_DIR}}/config.toml)");
    println!();

    if global.corpora.registered.is_empty() {
        println!("[-] No corpora currently registered in central configuration.");
        println!("    Register a repository with 'groundcontrol corpus add <path>'.");
        return Ok(());
    }

    println!(
        "  {:<24} {:<8} {:<8} {:<12} {}",
        "NAME", "DEFAULT", "MODE", "DISK USAGE", "SOURCE PATH"
    );
    println!("  {}", "-".repeat(80));

    for (name, reg) in &global.corpora.registered {
        let is_default = if name == default_name { "*" } else { "" };
        let mode_str = match reg.index_mode {
            Some(IndexMode::Fast) => "fast",
            _ => "full",
        };
        let footprint = calculate_corpus_disk_usage(name);
        let usage_str = if footprint.total_bytes > 0 {
            format_bytes(footprint.total_bytes)
        } else {
            "unindexed".to_string()
        };

        println!("  {:<24} {:<8} {:<8} {:<12} {}", name, is_default, mode_str, usage_str, reg.path);
    }

    println!();
    Ok(())
}

/// Register a repository in central configuration and optionally index it immediately.
pub fn handle_corpus_add(
    path: PathBuf,
    name: Option<String>,
    mode: Option<IndexMode>,
    no_index: bool,
    fast: bool,
) -> anyhow::Result<()> {
    let canonical = path.canonicalize().unwrap_or(path);
    if !canonical.exists() {
        anyhow::bail!("corpus path does not exist: {}", canonical.display());
    }

    let derived_name =
        canonical.file_name().and_then(|n| n.to_str()).unwrap_or("corpus").to_string();
    let active_name = name.unwrap_or(derived_name);
    let index_mode = mode.unwrap_or(if fast { IndexMode::Fast } else { IndexMode::Full });

    let mut global = load_global_config();
    let is_first = global.corpora.registered.is_empty();

    global.corpora.registered.insert(
        active_name.clone(),
        RegisteredCorpus {
            path: canonical.to_string_lossy().replace('\\', "/"),
            index_mode: Some(index_mode),
        },
    );

    if is_first || global.corpora.default.is_none() {
        global.corpora.default = Some(active_name.clone());
    }

    save_global_config(&global)?;
    println!("[+] Registered corpus '{}' in central configuration.", active_name);

    if !no_index {
        println!();
        handle_index(canonical, Some(active_name), false, index_mode == IndexMode::Fast, 50)?;
    } else {
        println!(
            "    [i] Skipped indexing (--no-index). Run 'groundcontrol corpus sync {}' when ready.",
            active_name
        );
    }

    Ok(())
}

/// Deregister a corpus from central configuration with optional cache purging.
pub fn handle_corpus_remove(name: &str, purge: bool) -> anyhow::Result<()> {
    let mut global = load_global_config();

    if global.corpora.registered.remove(name).is_none() {
        anyhow::bail!("corpus '{}' is not registered in central configuration", name);
    }

    if global.corpora.default.as_deref() == Some(name) {
        global.corpora.default = global.corpora.registered.keys().next().cloned();
    }

    save_global_config(&global)?;
    println!("[-] Deregistered corpus '{}' from central configuration.", name);

    if purge {
        let index_dir = get_corpus_index_dir(name);
        if index_dir.exists() {
            std::fs::remove_dir_all(&index_dir).with_context(|| {
                format!("failed to remove index directory {}", index_dir.display())
            })?;
            println!("[-] Purged index cache directory: {}", index_dir.display());
        }
    }

    Ok(())
}

/// View or update the active default corpus in central configuration.
pub fn handle_corpus_default(name: Option<String>) -> anyhow::Result<()> {
    let mut global = load_global_config();

    if let Some(target) = name {
        if !global.corpora.registered.contains_key(&target) {
            anyhow::bail!("corpus '{}' is not registered in central configuration", target);
        }
        global.corpora.default = Some(target.clone());
        save_global_config(&global)?;
        println!("[+] Set default corpus to '{}'.", target);
    } else {
        match &global.corpora.default {
            Some(def) => println!("Default corpus: {}", def),
            None => println!("No default corpus currently set."),
        }
    }

    Ok(())
}

/// Synchronize a specific corpus or all registered corpora.
pub fn handle_corpus_sync(corpus: Option<String>, batch_size: usize) -> anyhow::Result<()> {
    handle_sync(corpus, batch_size)
}

/// Export a corpus index into a compressed portable archive (.groundcontrol/vault.tar.zst).
pub fn handle_corpus_export(corpus: Option<String>, output: Option<PathBuf>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let name = corpus
        .as_deref()
        .unwrap_or_else(|| cwd.file_name().and_then(|n| n.to_str()).unwrap_or("default"));
    let index_dir = get_corpus_index_dir(name);
    if !index_dir.exists() {
        anyhow::bail!(
            "No central index found for corpus '{}' at '{}'. Run indexing first.",
            name,
            index_dir.display()
        );
    }
    let exported = crate::artifacts::export_artifact(&index_dir, &cwd, output.as_deref())?;
    println!("[+] Exported corpus artifact to: {}", exported.display());
    Ok(())
}

/// Import a compressed portable archive (.groundcontrol/vault.tar.zst) into central corpus storage.
pub fn handle_corpus_import(input: Option<PathBuf>, corpus: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let name = corpus
        .as_deref()
        .unwrap_or_else(|| cwd.file_name().and_then(|n| n.to_str()).unwrap_or("default"));
    let src_path = input.unwrap_or_else(|| cwd.join(".groundcontrol").join("vault.tar.zst"));
    let dest_dir = get_corpus_index_dir(name);
    let imported = crate::artifacts::import_artifact(&src_path, &dest_dir)?;
    println!("[+] Imported corpus artifact into central storage: {}", imported.display());
    Ok(())
}
