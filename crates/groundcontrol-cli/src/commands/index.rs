use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use groundcontrol_common::config::{calculate_corpus_disk_usage, format_bytes};
use groundcontrol_core::corpus_manager::CorpusManager;
use groundcontrol_core::engine::{IndexingProgress, IndexingStage, ProgressCallback};

/// Build a progress callback that renders a dynamic terminal ticker with progress bar.
pub fn make_terminal_progress_reporter(is_terminal: bool) -> ProgressCallback {
    Arc::new(move |p: &IndexingProgress| {
        if is_terminal {
            let percent = if p.total_files > 0 {
                (p.processed_files as f64 / p.total_files as f64 * 100.0).min(100.0)
            } else {
                0.0
            };
            let bar_len: usize = 20;
            let filled = (percent / 100.0 * bar_len as f64) as usize;
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_len.saturating_sub(filled));
            let stage_name = match p.stage {
                IndexingStage::Discovery => "discovery",
                IndexingStage::ParsingAndIndexing => "indexing",
                IndexingStage::GeneratingEmbeddings => "embeddings",
                IndexingStage::ResolvingGraphEdges => "graph edges",
                IndexingStage::Committing => "committing",
                IndexingStage::Completed => "done",
            };
            let path_hint = p.current_path.as_deref().unwrap_or("");
            let truncated_path = if path_hint.len() > 30 {
                format!("...{}", &path_hint[path_hint.len() - 27..])
            } else {
                path_hint.to_string()
            };

            print!(
                "\r  [{}] {:>3.0}% ({}/{} files) | {:.1} f/s | {:<11} {:<30}\x1b[K",
                bar,
                percent,
                p.processed_files,
                p.total_files,
                p.files_per_second,
                stage_name,
                truncated_path
            );
            let _ = std::io::stdout().flush();
        }
    })
}

/// Print formatted index completion summary card with storage footprint breakdown.
pub fn print_index_completion_card(
    name: &str,
    elapsed: Duration,
    engine: &groundcontrol_core::engine::Engine,
) {
    use groundcontrol_common::ports::MetadataCatalog;
    let file_count = engine.store().list_files().map(|f| f.len()).unwrap_or(0);
    let node_count = engine.knowledge_graph().node_count();
    let edge_count = engine.knowledge_graph().edge_count();
    let vector_count = engine.vector_count();
    let footprint = calculate_corpus_disk_usage(name);

    println!();
    println!("+------------------------------------------------------------------------+");
    println!("|  Index Complete: {:<38} ({:>4.1}s elapsed) |", name, elapsed.as_secs_f64());
    println!("+------------------------------------------------------------------------+");
    println!("|  Entities & Graph:                                                     |");
    println!("|  ├─ Files Indexed     : {:<47}|", format!("{} files", file_count));
    println!(
        "|  ├─ Graph Topology    : {:<47}|",
        format!("{} nodes, {} edges", node_count, edge_count)
    );
    if engine.has_vector_index() {
        println!(
            "|  └─ Vector Chunks     : {:<47}|",
            format!("{} vectors ({})", vector_count, engine.hardware_acceleration())
        );
    } else {
        println!("|  └─ Vector Chunks     : 0 vectors (fast mode, skipped ONNX)            |");
    }
    println!("|                                                                        |");
    println!("|  Central Cache Footprint:                                              |");
    println!("|  ├─ SQLite Metadata   : {:<47}|", format_bytes(footprint.meta_db_bytes));
    println!("|  ├─ Tantivy Index     : {:<47}|", format_bytes(footprint.tantivy_bytes));
    if footprint.vectors_bytes > 0 {
        println!("|  ├─ Vector Store      : {:<47}|", format_bytes(footprint.vectors_bytes));
    }
    println!("|  └─ Total Footprint   : {:<47}|", format_bytes(footprint.total_bytes));
    println!("+------------------------------------------------------------------------+");
    println!();
}

/// Prompt interactive user to extract compressed index bundle into central storage if present and unextracted.
pub fn prompt_bundle_extraction(
    corpus_root: &Path,
    name: &str,
    index_dir: &Path,
) -> anyhow::Result<()> {
    if let Some(bundle) = groundcontrol_core::bundle::detect_bundle(corpus_root) {
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
                groundcontrol_core::bundle::import_bundle(&bundle, index_dir, None, None)?;
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
    let index_dir = groundcontrol_common::config::get_corpus_index_dir(&active_name);
    prompt_bundle_extraction(&canonical, &active_name, &index_dir)?;

    let mut manager = CorpusManager::new();
    let active_name = manager.ensure_corpus_with_name(&canonical, name.as_deref())?;
    let engine = manager.get_engine_mut(&active_name)?;
    if fast {
        engine.config_mut().index_mode = groundcontrol_common::config::IndexMode::Fast;
    }

    if !canonical.join("groundcontrol.toml").exists() && !canonical.join("ctxvault.toml").exists() {
        println!(
            "[i] No groundcontrol.toml found. Indexed using defaults + local .gitignore. Run 'groundcontrol init' to commit a local groundcontrol.toml."
        );
    }

    println!(
        "[*] Indexing corpus '{}' ({}) into central storage...",
        active_name,
        canonical.display()
    );

    let is_terminal = std::io::stdout().is_terminal();
    let progress_cb = make_terminal_progress_reporter(is_terminal);
    let start = Instant::now();

    if reindex {
        engine.full_reindex_with_progress(batch_size, false, Some(progress_cb))?;
    } else {
        let delta = engine.delta_scan_with_progress(batch_size, Some(progress_cb))?;
        if is_terminal {
            println!();
        }
        println!(
            "[+] Delta scan: {} new, {} modified, {} deleted",
            delta.new_files.len(),
            delta.modified_files.len(),
            delta.deleted_files.len()
        );
    };

    if is_terminal {
        println!();
    }

    let elapsed = start.elapsed();
    print_index_completion_card(&active_name, elapsed, engine);

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

    let is_terminal = std::io::stdout().is_terminal();

    for target_name in targets {
        println!("[*] Syncing corpus '{}'...", target_name);
        let progress_cb = make_terminal_progress_reporter(is_terminal);
        let start = Instant::now();
        let engine = manager.get_engine_mut(&target_name)?;
        let delta = engine.delta_scan_with_progress(batch_size, Some(progress_cb))?;
        if is_terminal {
            println!();
        }
        println!(
            "[+] '{}': {} new, {} modified, {} deleted",
            target_name,
            delta.new_files.len(),
            delta.modified_files.len(),
            delta.deleted_files.len()
        );

        let elapsed = start.elapsed();
        print_index_completion_card(&target_name, elapsed, engine);
    }

    Ok(())
}
