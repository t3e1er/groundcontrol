//! `import` benchmark dataset command handler.

use std::fs;
use std::path::PathBuf;

use groundcontrol_bench::dataset::{PublicBenchmarkAdapter, PublicBenchmarkFormat};

/// Handle the `import` subcommand: import and convert public benchmarks (CSN, RepoBench, SWE-bench).
pub fn handle_import(
    input: PathBuf,
    format: String,
    output: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let fmt = match format.to_lowercase().trim() {
        "codesearchnet" | "csn" | "advtest" => PublicBenchmarkFormat::CodeSearchNet,
        "repobench" | "repobench-r" | "rb" => PublicBenchmarkFormat::RepoBench,
        "swebench" | "swe-bench" | "swe" => PublicBenchmarkFormat::SweBench,
        other => {
            return Err(format!(
                "Unknown benchmark format '{other}'. Valid formats: codesearchnet, repobench, swebench"
            )
            .into());
        }
    };

    println!("Converting {} using format {:?}...", input.display(), fmt);
    let dataset = PublicBenchmarkAdapter::convert_file(&input, fmt)?;
    let json_str = serde_json::to_string_pretty(&dataset)?;

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, json_str)?;
    println!("Successfully converted {} queries to: {}", dataset.queries.len(), output.display());
    Ok(())
}
