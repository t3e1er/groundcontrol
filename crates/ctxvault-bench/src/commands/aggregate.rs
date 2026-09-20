//! `aggregate` benchmark command handler.

use std::fs;
use std::path::PathBuf;

use ctxvault_bench::report::ReportAggregator;

/// Handle the `aggregate` subcommand: merge all sub-report CSVs into a master summary leaderboard.
pub fn handle_aggregate(
    results_dir: PathBuf,
    output_dir: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = output_dir.unwrap_or_else(|| results_dir.clone());
    fs::create_dir_all(&out_dir)?;

    println!("Aggregating sub-reports from: {}", results_dir.display());
    let (md_str, csv_str) = ReportAggregator::aggregate(&results_dir);

    let md_path = out_dir.join("summary_report.md");
    let csv_path = out_dir.join("summary_report.csv");

    fs::write(&md_path, &md_str)?;
    fs::write(&csv_path, &csv_str)?;

    println!("Master aggregate reports successfully written to:");
    println!("  - {}", md_path.display());
    println!("  - {}", csv_path.display());
    Ok(())
}
