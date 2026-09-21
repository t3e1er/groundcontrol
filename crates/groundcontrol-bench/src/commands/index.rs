//! `index` benchmark command handler.

use std::fs;
use std::path::PathBuf;

use groundcontrol_bench::profile::{IndexProfiler, IndexProfilerOptions};

/// Handle the `index` subcommand to profile the indexing pipeline.
pub fn handle_index(
    corpus: PathBuf,
    clean: bool,
    reembed: bool,
    output: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Profiling indexing pipeline for: {}", corpus.display());
    let opts = IndexProfilerOptions { include_dense_embedding: reembed, clean_cold_start: clean };
    let report = IndexProfiler::profile(&corpus, &opts)?;
    let json_str = serde_json::to_string_pretty(&report)?;

    if let Some(out_path) = output {
        fs::write(&out_path, &json_str)?;
        println!("Wrote indexing profile report to: {}", out_path.display());
    } else {
        println!("{json_str}");
    }
    Ok(())
}
