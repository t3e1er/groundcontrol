//! `all` benchmark command handler: clean reindex with resource profiling followed by retrieval evaluation.

use std::fs;
use std::path::PathBuf;

use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::Modality;
use groundcontrol_core::engine::Engine;

use groundcontrol_bench::dataset::DatasetLoader;
use groundcontrol_bench::profile::{IndexProfiler, IndexProfilerOptions};
use groundcontrol_bench::report::{CsvReporter, JsonReporter, MarkdownReporter};
use groundcontrol_bench::sweep::{BenchmarkSuite, BenchmarkSuiteReport};

use super::eval::parse_modes;

/// Handle the `all` subcommand.
pub fn handle_all(
    corpus: PathBuf,
    queries: PathBuf,
    modes: String,
    k: usize,
    clean: bool,
    reembed: bool,
    output_dir: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let modes_list = parse_modes(&modes)?;
    let dataset = DatasetLoader::load_from_file(&queries)?;

    println!("=== 1. Profiling Indexing Pipeline ===");
    let opts = IndexProfilerOptions { include_dense_embedding: reembed, clean_cold_start: clean };
    let idx_report = IndexProfiler::profile(&corpus, &opts)?;
    println!(
        "Indexed {} documents in {:.2}ms ({:.1} files/s, peak RSS: {:.1}MB)",
        idx_report.documents_indexed,
        idx_report.total_elapsed_ms,
        idx_report.docs_per_second,
        idx_report.memory.peak_mb()
    );

    println!("\n=== 2. Running Retrieval Ablation ===");
    let index_dir = corpus.join(".index");
    let config_path = if corpus.join("groundcontrol.toml").exists() {
        corpus.join("groundcontrol.toml")
    } else {
        corpus.join("ctxvault.toml")
    };
    let config: CorpusConfig = if config_path.exists() {
        let s = fs::read_to_string(&config_path)?;
        toml::from_str(&s)?
    } else {
        CorpusConfig { path: corpus.to_string_lossy().to_string(), ..Default::default() }
    };

    let engine = Engine::open(config, &index_dir)?;
    let summaries =
        BenchmarkSuite::evaluate_modes(&engine, &dataset, &modes_list, k, Modality::Both)?;

    let suite_report = BenchmarkSuiteReport {
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        query_count: dataset.queries.len(),
        k,
        modes: summaries,
        indexing: Some(idx_report),
        benchmark: None,
        repository: None,
    };

    if let Some(dir) = output_dir {
        fs::create_dir_all(&dir)?;
        let md_str = MarkdownReporter::render(&suite_report);
        let json_str = JsonReporter::to_string(&suite_report)?;
        let csv_str = CsvReporter::render_retrieval_csv(&suite_report);

        fs::write(dir.join("report.md"), md_str)?;
        fs::write(dir.join("report.json"), json_str)?;
        fs::write(dir.join("report.csv"), csv_str)?;
        println!("Saved reports (report.md, report.json, report.csv) to: {}", dir.display());
    } else {
        let md = MarkdownReporter::render(&suite_report);
        println!("\n{md}");
    }
    Ok(())
}
