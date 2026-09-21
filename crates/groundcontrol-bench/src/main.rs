//! `gc-bench`: Dedicated data science benchmarking CLI for groundcontrol.

mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use commands::aggregate::handle_aggregate;
use commands::all::handle_all;
use commands::eval::handle_eval;
use commands::import::handle_import;
use commands::index::handle_index;
use commands::status::handle_status;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { corpus, clean, reembed, output } => {
            handle_index(corpus, clean, reembed, output)?;
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
            handle_eval(
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
            )?;
        }
        Commands::All { corpus, queries, modes, k, clean, reembed, output_dir } => {
            handle_all(corpus, queries, modes, k, clean, reembed, output_dir)?;
        }
        Commands::Import { input, format, output } => {
            handle_import(input, format, output)?;
        }
        Commands::Aggregate { results_dir, output_dir } => {
            handle_aggregate(results_dir, output_dir)?;
        }
        Commands::Status { corpus } => {
            handle_status(corpus)?;
        }
    }

    Ok(())
}
