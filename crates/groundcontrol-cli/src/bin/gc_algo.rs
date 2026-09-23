//! `gc-algo` — CLI binary wrapper for direct algorithmic retrieval.
//!
//! Provides direct command-line execution and JSON output for isolated retrieval algorithms.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use groundcontrol_common::types::Modality;
use groundcontrol_core::algorithm::{AlgoConfig, AlgorithmicIndex, BinaryProjectionKind};

#[derive(Parser, Debug)]
#[command(name = "gc-algo", about = "Direct algorithmic retrieval CLI for groundcontrol")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Build an index for a corpus
    Index {
        /// Path to corpus directory
        #[arg(short, long)]
        corpus: PathBuf,

        /// Binary projection method to configure
        #[arg(long, default_value = "partitioned-hyperplane")]
        projection: CliProjection,
    },

    /// Run an algorithmic query against an indexed corpus
    Query {
        /// Path to corpus directory
        #[arg(short, long)]
        corpus: PathBuf,

        /// Retrieval algorithm to execute
        #[arg(short, long)]
        method: CliMethod,

        /// Query string
        #[arg(short, long)]
        query: String,

        /// Number of results to return
        #[arg(short, long, default_value_t = 10)]
        k: usize,

        /// Modality filter: both, code, or docs
        #[arg(long, default_value = "both")]
        modality: CliModality,

        /// Binary projection variant: partitioned-hyperplane or flat-sif
        #[arg(long, default_value = "partitioned-hyperplane")]
        projection: CliProjection,

        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliMethod {
    Binary,
    Bm25,
    Ppr,
    Fast,
    Semantic,
    Hybrid,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliProjection {
    FlatSif,
    PartitionedHyperplane,
}

impl From<CliProjection> for BinaryProjectionKind {
    fn from(p: CliProjection) -> Self {
        match p {
            CliProjection::FlatSif => BinaryProjectionKind::FlatSif,
            CliProjection::PartitionedHyperplane => BinaryProjectionKind::PartitionedHyperplane,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliModality {
    Both,
    Code,
    Docs,
}

impl From<CliModality> for Modality {
    fn from(m: CliModality) -> Self {
        match m {
            CliModality::Both => Modality::Both,
            CliModality::Code => Modality::Code,
            CliModality::Docs => Modality::Docs,
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { corpus, projection } => {
            let config = AlgoConfig { binary_projection: projection.into(), ..Default::default() };
            println!("Building algorithmic index for {}...", corpus.display());
            let (_index, stats) =
                AlgorithmicIndex::build(&corpus, config).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "Indexed {} documents ({} nodes, {} edges) in {:.2}ms",
                stats.documents, stats.graph_nodes, stats.graph_edges, stats.time_ms
            );
        }
        Commands::Query { corpus, method, query, k, modality, projection, json } => {
            let config = AlgoConfig { binary_projection: projection.into(), ..Default::default() };
            let index =
                AlgorithmicIndex::load(&corpus, config).map_err(|e| anyhow::anyhow!("{e}"))?;
            let mod_val = modality.into();

            let hits = match method {
                CliMethod::Binary => index.query_binary(&query, k, mod_val),
                CliMethod::Bm25 => index.query_bm25(&query, k, mod_val),
                CliMethod::Ppr => index.query_ppr(&query, k, mod_val),
                CliMethod::Fast => index.query_fast(&query, k, mod_val),
                CliMethod::Semantic => index.query_semantic(&query, k, mod_val),
                CliMethod::Hybrid => index.query_hybrid(&query, k, mod_val),
            }
            .map_err(|e| anyhow::anyhow!("{e}"))?;

            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                println!("Results for '{}' ({:?}):", query, method);
                for hit in hits {
                    println!(
                        "  #{:<2} {:<60} score: {:.4}{}",
                        hit.rank,
                        hit.path,
                        hit.score,
                        hit.symbol.map(|s| format!(" ({s})")).unwrap_or_default()
                    );
                }
            }
        }
    }

    Ok(())
}
