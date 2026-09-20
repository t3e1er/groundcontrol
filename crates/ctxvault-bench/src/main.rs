//! `ctxv-bench`: Dedicated data science benchmarking CLI for ctxvault.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Parser, Subcommand};
use ctxvault_bench::dataset::{
    DatasetLoader, DeterministicSampler, PublicBenchmarkAdapter, PublicBenchmarkFormat,
};
use ctxvault_bench::profile::{IndexProfiler, IndexProfilerOptions};
use ctxvault_bench::report::{
    CsvReporter, JsonReporter, LatexReporter, MarkdownReporter, ReportAggregator,
};
use ctxvault_bench::runners::RetrievalMode;
use ctxvault_bench::sweep::{BenchmarkSuite, BenchmarkSuiteReport};
use ctxvault_common::config::CorpusConfig;
use ctxvault_common::types::Modality;
use ctxvault_core::engine::Engine;

#[derive(Parser)]
#[command(
    name = "ctxv-bench",
    about = "Data science benchmark harness: Indexing resource profiling & retrieval algorithm ablation",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Profile the indexing pipeline (wall-clock stage timings, throughput, peak RSS, disk breakdown)
    Index {
        /// Path to the corpus directory
        #[arg(short, long)]
        corpus: PathBuf,

        /// Perform a clean cold-start build by removing existing .index directory
        #[arg(long, default_value_t = false)]
        clean: bool,

        /// Include dense ONNX neural re-embedding stage
        #[arg(long, default_value_t = false)]
        reembed: bool,

        /// Path to output JSON profiling report
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run retrieval quality evaluation and latency benchmarks across specified modes
    Eval {
        /// Path to the indexed corpus directory
        #[arg(short, long)]
        corpus: PathBuf,

        /// Path to the queries and ground-truth judgments JSON file
        #[arg(short, long)]
        queries: PathBuf,

        /// Comma-separated list of retrieval modes to ablate (bm25, binary, ppr, fast, semantic, full, or all)
        #[arg(short, long, default_value = "all")]
        modes: String,

        /// Cutoff rank K for Recall@K, MRR@K, NDCG@K
        #[arg(short, long, default_value_t = 5)]
        k: usize,

        /// Modality filter: both, code, or docs
        #[arg(long, default_value = "both")]
        modality: String,

        /// Optional output file path (.md, .json, or .csv)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Optional directory to write publication reports (.md and .csv by default)
        #[arg(long)]
        output_dir: Option<PathBuf>,

        /// Optional file prefix when used with --output-dir (e.g. "codesearchnet")
        #[arg(long)]
        output_prefix: Option<String>,

        /// Optional category/repository filter to only evaluate queries for a specific repo
        #[arg(long)]
        category: Option<String>,

        /// Optional sample count to deterministically subsample queries
        #[arg(long)]
        sample: Option<usize>,

        /// Seed for deterministic query sampling
        #[arg(long, default_value_t = 42)]
        seed: u64,

        /// Optional benchmark name (e.g. "swe_bench", "codesearchnet", "repobench")
        #[arg(long)]
        benchmark_name: Option<String>,

        /// Optional target repository name (e.g. "pallets__flask")
        #[arg(long)]
        repository: Option<String>,

        /// Whether to also export raw JSON report in addition to .md and .csv
        #[arg(long, default_value_t = false)]
        include_json: bool,

        /// Whether to also export LaTeX table in addition to .md and .csv
        #[arg(long, default_value_t = false)]
        include_tex: bool,
    },

    /// Run full benchmark: clean reindex with resource profiling followed by retrieval evaluation
    All {
        /// Path to the corpus directory
        #[arg(short, long)]
        corpus: PathBuf,

        /// Path to the queries and ground-truth judgments JSON file
        #[arg(short, long)]
        queries: PathBuf,

        /// Comma-separated list of retrieval modes (default: all)
        #[arg(short, long, default_value = "all")]
        modes: String,

        /// Cutoff rank K for Recall@K, MRR@K, NDCG@K
        #[arg(short, long, default_value_t = 5)]
        k: usize,

        /// Perform a clean cold-start build
        #[arg(long, default_value_t = false)]
        clean: bool,

        /// Include dense ONNX neural re-embedding stage
        #[arg(long, default_value_t = false)]
        reembed: bool,

        /// Output directory to write report.md, report.json, and report.csv
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
    },

    /// Import and convert a public benchmark dataset (CodeSearchNet, RepoBench-R, or SWE-bench)
    Import {
        /// Path to the external dataset file (JSON or JSONL)
        #[arg(short, long)]
        input: PathBuf,

        /// Format: codesearchnet (or csn), repobench (or rb), swebench (or swe)
        #[arg(short, long)]
        format: String,

        /// Output path for the converted benchmark dataset JSON
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Aggregate all sub-report CSVs into a unified master leaderboard (summary_report.md & summary_report.csv)
    Aggregate {
        /// Results directory containing sub-reports
        #[arg(short, long)]
        results_dir: PathBuf,

        /// Optional output directory for summary reports (defaults to results_dir)
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
    },

    /// Check index health, document count, and validity of a corpus
    Status {
        /// Path to the corpus directory
        #[arg(short, long)]
        corpus: PathBuf,
    },
}

fn parse_modes(modes_str: &str) -> Result<Vec<RetrievalMode>, String> {
    if modes_str.trim().eq_ignore_ascii_case("all") {
        return Ok(RetrievalMode::all().to_vec());
    }
    let mut modes = Vec::new();
    for part in modes_str.split(',') {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            modes.push(RetrievalMode::from_str(trimmed)?);
        }
    }
    if modes.is_empty() {
        return Err("No retrieval modes specified".to_string());
    }
    Ok(modes)
}

fn parse_modality(mod_str: &str) -> Modality {
    match mod_str.to_lowercase().trim() {
        "code" => Modality::Code,
        "docs" | "doc" => Modality::Docs,
        _ => Modality::Both,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { corpus, clean, reembed, output } => {
            println!("Profiling indexing pipeline for: {}", corpus.display());
            let opts =
                IndexProfilerOptions { include_dense_embedding: reembed, clean_cold_start: clean };
            let report = IndexProfiler::profile(&corpus, &opts)?;
            let json_str = serde_json::to_string_pretty(&report)?;

            if let Some(out_path) = output {
                fs::write(&out_path, &json_str)?;
                println!("Wrote indexing profile report to: {}", out_path.display());
            } else {
                println!("{json_str}");
            }
        }

        Commands::Eval {
            corpus,
            queries,
            modes,
            k,
            modality,
            output,
            output_dir,
            output_prefix,
            category,
            sample,
            seed,
            benchmark_name,
            repository,
            include_json,
            include_tex,
        } => {
            let modes_list = parse_modes(&modes)?;
            let mut mod_enum = parse_modality(&modality);
            if modality == "both" {
                if let Some(ref bname) = benchmark_name {
                    let b_lower = bname.to_lowercase();
                    if b_lower.contains("code")
                        || b_lower.contains("repo")
                        || b_lower.contains("swe")
                    {
                        mod_enum = Modality::Code;
                    }
                }
            }
            let mut dataset = DatasetLoader::load_from_file(&queries)?;

            let target_repo = repository.as_ref().or(category.as_ref());
            if let Some(repo_filter) = target_repo {
                let rf_lower = repo_filter.to_lowercase();
                dataset.queries.retain(|q| {
                    let matches_repo = q
                        .repository
                        .as_deref()
                        .map(|r| {
                            let r_lower = r.to_lowercase();
                            r_lower.contains(&rf_lower) || rf_lower.contains(&r_lower)
                        })
                        .unwrap_or(false);

                    let matches_cat = q
                        .category
                        .as_deref()
                        .map(|c| {
                            let c_lower = c.to_lowercase();
                            c_lower.contains(&rf_lower) || rf_lower.contains(&c_lower)
                        })
                        .unwrap_or(false);

                    matches_repo || matches_cat
                });
                println!(
                    "Filtered to {} queries matching repository/category '{}'",
                    dataset.queries.len(),
                    repo_filter
                );
            }

            if let Some(n) = sample {
                if n < dataset.queries.len() {
                    let mut sampler = DeterministicSampler::new(seed);
                    let sampled_indices = sampler.sample_indices(dataset.queries.len(), n);
                    let mut sampled_queries = Vec::with_capacity(n);
                    for idx in sampled_indices {
                        sampled_queries.push(dataset.queries[idx].clone());
                    }
                    println!(
                        "Deterministically sampled {} / {} queries using seed {}",
                        sampled_queries.len(),
                        dataset.queries.len(),
                        seed
                    );
                    dataset.queries = sampled_queries;
                }
            }

            println!(
                "Evaluating {} queries on corpus '{}'. Modes: {:?} at K={}",
                dataset.queries.len(),
                corpus.display(),
                modes_list,
                k
            );

            let index_dir = corpus.join(".index");
            let config_path = corpus.join("ctxvault.toml");
            let mut config: CorpusConfig = if config_path.exists() {
                let s = fs::read_to_string(&config_path)?;
                toml::from_str(&s)?
            } else {
                CorpusConfig { path: corpus.to_string_lossy().to_string(), ..Default::default() }
            };

            let needs_dense = modes_list
                .iter()
                .any(|m| matches!(m, RetrievalMode::Semantic | RetrievalMode::Full));
            if !needs_dense {
                config.index_mode = ctxvault_common::config::IndexMode::Fast;
            }

            let engine = Engine::open(config, &index_dir)?;
            let summaries =
                BenchmarkSuite::evaluate_modes(&engine, &dataset, &modes_list, k, mod_enum)?;

            let report = BenchmarkSuiteReport {
                timestamp_unix: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                query_count: dataset.queries.len(),
                k,
                modes: summaries,
                indexing: None,
                benchmark: benchmark_name,
                repository: repository.or(category),
            };

            if let Some(dir) = output_dir {
                fs::create_dir_all(&dir)?;
                let prefix = output_prefix.unwrap_or_else(|| "report".to_string());
                let md_str = MarkdownReporter::render(&report);
                let csv_str = CsvReporter::render_retrieval_csv(&report);

                let md_path = dir.join(format!("{prefix}_report.md"));
                let csv_path = dir.join(format!("{prefix}_report.csv"));

                fs::write(&md_path, md_str)?;
                fs::write(&csv_path, csv_str)?;

                if include_json {
                    let json_str = JsonReporter::to_string(&report)?;
                    let json_path = dir.join(format!("{prefix}_report.json"));
                    fs::write(&json_path, json_str)?;
                }

                if include_tex {
                    let tex_str = LatexReporter::render(&report);
                    let tex_path = dir.join(format!("{prefix}_table.tex"));
                    fs::write(&tex_path, tex_str)?;
                }

                println!(
                    "Generated publication reports (.md, .csv) for [{prefix}] in: {}",
                    dir.display()
                );
            } else {
                output_report(&report, output.as_deref())?;
            }
        }

        Commands::All { corpus, queries, modes, k, clean, reembed, output_dir } => {
            let modes_list = parse_modes(&modes)?;
            let dataset = DatasetLoader::load_from_file(&queries)?;

            println!("=== 1. Profiling Indexing Pipeline ===");
            let opts =
                IndexProfilerOptions { include_dense_embedding: reembed, clean_cold_start: clean };
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
            let config_path = corpus.join("ctxvault.toml");
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
                println!(
                    "Saved reports (report.md, report.json, report.csv) to: {}",
                    dir.display()
                );
            } else {
                let md = MarkdownReporter::render(&suite_report);
                println!("\n{md}");
            }
        }

        Commands::Import { input, format, output } => {
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
            println!(
                "Successfully converted {} queries to: {}",
                dataset.queries.len(),
                output.display()
            );
        }

        Commands::Aggregate { results_dir, output_dir } => {
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
        }
        Commands::Status { corpus } => {
            let index_dir = corpus.join(".index");
            let db_path = index_dir.join("meta.db");
            if !db_path.exists() {
                eprintln!("Index database does not exist: {}", db_path.display());
                std::process::exit(1);
            }
            let store = match ctxvault_core::persistence::Store::open(&db_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to open meta.db: {e}");
                    std::process::exit(1);
                }
            };
            let files = match store.list_files() {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Failed to read files from meta.db: {e}");
                    std::process::exit(1);
                }
            };
            if files.is_empty() {
                eprintln!("Index contains 0 documents");
                std::process::exit(1);
            }
            println!("Index healthy: {} documents indexed in {}", files.len(), corpus.display());
        }
    }

    Ok(())
}

fn output_report(
    report: &BenchmarkSuiteReport,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let md = MarkdownReporter::render(report);
    if let Some(path) = output {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("md");
        match ext {
            "json" => {
                let json_str = JsonReporter::to_string(report)?;
                fs::write(path, json_str)?;
            }
            "csv" => {
                let csv_str = CsvReporter::render_retrieval_csv(report);
                fs::write(path, csv_str)?;
            }
            "tex" | "latex" => {
                let tex_str = LatexReporter::render(report);
                fs::write(path, tex_str)?;
            }
            _ => {
                fs::write(path, &md)?;
            }
        }
        println!("Report saved to: {}", path.display());
    } else {
        println!("{md}");
    }
    Ok(())
}
