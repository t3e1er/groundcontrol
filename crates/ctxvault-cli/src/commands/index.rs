//! `index` and `sync` subcommand handlers.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ctxvault_core::corpus_manager::CorpusManager;

/// Prompt interactive user to extract compressed index bundle into central storage if present and unextracted.
pub fn prompt_bundle_extraction(
    corpus_root: &Path,
    name: &str,
    index_dir: &Path,
) -> anyhow::Result<()> {
    if let Some(bundle) = ctxvault_core::bundle::detect_bundle(corpus_root) {
        if !index_dir.join("meta.db").exists() {
            let should_extract = if std::io::stdin().is_terminal() {
                print!(
                    "[?] Found compressed index bundle for '{}' at '{}'. Extract into central storage? [Y/n]: ",
                    name,
                    bundle.display()
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                let trimmed = input.trim().to_lowercase();
                trimmed.is_empty() || trimmed == "y" || trimmed == "yes"
            } else {
                true
            };

            if should_extract {
                println!(
                    "[*] Extracting bundle '{}' into central storage ({})...",
                    bundle.display(),
                    index_dir.display()
                );
                ctxvault_core::bundle::import_bundle(&bundle, index_dir, None, None)?;
                println!("[+] Successfully extracted bundle into central storage.");
            }
        }
    }
    Ok(())
}

/// Execute the `index` subcommand: index a corpus directory into central storage.
pub fn handle_index(
    path: PathBuf,
    name: Option<String>,
    reindex: bool,
    fast: bool,
    batch_size: usize,
) -> anyhow::Result<()> {
    let canonical = path.canonicalize().unwrap_or(path);
    let dir_name = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("corpus").to_string();
    let active_name = name.clone().unwrap_or(dir_name);
    let index_dir = ctxvault_common::config::get_corpus_index_dir(&active_name);
    prompt_bundle_extraction(&canonical, &active_name, &index_dir)?;

    let mut manager = CorpusManager::new();
    let active_name = manager.ensure_corpus_with_name(&canonical, name.as_deref())?;
    let engine = manager.get_engine_mut(&active_name)?;
    if fast {
        engine.config_mut().index_mode = ctxvault_common::config::IndexMode::Fast;
    }

    if !canonical.join("ctxvault.toml").exists() {
        println!(
            "[i] No ctxvault.toml found. Indexed using defaults + local .gitignore. Run 'ctxvault init' to commit a local ctxvault.toml."
        );
    }

    println!(
        "[*] Indexing corpus '{}' ({}) into central storage...",
        active_name,
        canonical.display()
    );
    let start = Instant::now();
    let initial_vectors = engine.vector_count();
    let count = if reindex {
        engine.full_reindex_paginated(batch_size, false)?
    } else {
        let delta = engine.delta_scan_paginated(batch_size)?;
        println!(
            "[+] Delta scan: {} new, {} modified, {} deleted",
            delta.new_files.len(),
            delta.modified_files.len(),
            delta.deleted_files.len()
        );
        delta.new_files.len() + delta.modified_files.len()
    };

    let elapsed = start.elapsed();
    let final_vectors = engine.vector_count();
    let total_inserted =
        if reindex { final_vectors } else { final_vectors.saturating_sub(initial_vectors) };

    println!("[+] Successfully indexed '{}' ({} files processed)", active_name, count);

    if engine.has_vector_index() && total_inserted > 0 {
        let chunks_per_sec = if elapsed.as_secs_f64() > 0.0 {
            total_inserted as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        println!();
        println!("  Embedding complete");
        println!("  ├─ Chunks embedded : {}", total_inserted);
        println!("  ├─ Elapsed         : {:.1}s", elapsed.as_secs_f64());
        println!("  ├─ Throughput      : {:.1} chunks/sec", chunks_per_sec);
        if chunks_per_sec < 5.0 && total_inserted > 100 {
            println!("  └─ Tip: slow throughput detected. Consider --mode docs-embed for faster indexing.");
            println!("         (skeleton mode embeds ~1 chunk/file; throughput will improve after reindex)");
        } else {
            println!("  └─ Hardware       : {}", engine.hardware_acceleration());
        }
    }

    Ok(())
}

/// Execute the `sync` subcommand: synchronize cached corpora with their source directories.
pub fn handle_sync(corpus: Option<String>, batch_size: usize) -> anyhow::Result<()> {
    let mut manager = CorpusManager::new();
    let mounted = manager.mount_all_cached_corpora()?;
    if mounted.is_empty() {
        println!("[-] No cached corpora found in central storage to sync.");
        return Ok(());
    }

    let targets: Vec<String> = if let Some(target) = corpus {
        if !manager.has_corpus(&target) {
            anyhow::bail!("Corpus '{}' not found in central storage", target);
        }
        vec![target]
    } else {
        mounted
    };

    for target_name in targets {
        println!("[*] Syncing corpus '{}'...", target_name);
        let start = Instant::now();
        let engine = manager.get_engine_mut(&target_name)?;
        let initial_vectors = engine.vector_count();
        let delta = engine.delta_scan_paginated(batch_size)?;
        let elapsed = start.elapsed();
        let final_vectors = engine.vector_count();
        let total_inserted = final_vectors.saturating_sub(initial_vectors);

        println!(
            "[+] '{}': {} new, {} modified, {} deleted",
            target_name,
            delta.new_files.len(),
            delta.modified_files.len(),
            delta.deleted_files.len()
        );

        if engine.has_vector_index() && total_inserted > 0 {
            let chunks_per_sec = if elapsed.as_secs_f64() > 0.0 {
                total_inserted as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };

            println!();
            println!("  Embedding complete");
            println!("  ├─ Chunks embedded : {}", total_inserted);
            println!("  ├─ Elapsed         : {:.1}s", elapsed.as_secs_f64());
            println!("  ├─ Throughput      : {:.1} chunks/sec", chunks_per_sec);
            if chunks_per_sec < 5.0 && total_inserted > 100 {
                println!("  └─ Tip: slow throughput detected. Consider --mode docs-embed for faster indexing.");
                println!("         (skeleton mode embeds ~1 chunk/file; throughput will improve after reindex)");
            } else {
                println!("  └─ Hardware       : {}", engine.hardware_acceleration());
            }
        }
    }
    Ok(())
}
