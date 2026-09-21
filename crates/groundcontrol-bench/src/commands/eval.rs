//! `eval` benchmark command handler.

use groundcontrol_bench::dataset::{DatasetLoader, DeterministicSampler};
use groundcontrol_bench::report::{CsvReporter, JsonReporter, LatexReporter, MarkdownReporter};
use groundcontrol_bench::runners::RetrievalMode;
use groundcontrol_bench::sweep::{BenchmarkSuite, BenchmarkSuiteReport};
use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::Modality;
use groundcontrol_core::engine::Engine;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Parse a comma-separated retrieval modes string.
pub fn parse_modes(modes_str: &str) -> Result<Vec<RetrievalMode>, String> {
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

/// Parse a modality filter string.
pub fn parse_modality(mod_str: &str) -> Modality {
    match mod_str.to_lowercase().trim() {
        "code" => Modality::Code,
        "docs" | "doc" => Modality::Docs,
        _ => Modality::Both,
    }
}

/// Helper to render and output a report to stdout or a file.
pub fn output_report(
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

/// Handle the `eval` subcommand.
#[allow(clippy::too_many_arguments)]
pub fn handle_eval(
    corpus: PathBuf,
    queries: PathBuf,
    modes: String,
    k: usize,
    modality: String,
    output: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    output_prefix: Option<String>,
    category: Option<String>,
    sample: Option<usize>,
    seed: u64,
    benchmark_name: Option<String>,
    repository: Option<String>,
    include_json: bool,
    include_tex: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let modes_list = parse_modes(&modes)?;
    let mut mod_enum = parse_modality(&modality);
    if modality == "both" {
        if let Some(ref bname) = benchmark_name {
            let b_lower = bname.to_lowercase();
            if b_lower.contains("code") || b_lower.contains("repo") || b_lower.contains("swe") {
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
    let config_path = if corpus.join("groundcontrol.toml").exists() {
        corpus.join("groundcontrol.toml")
    } else {
        corpus.join("ctxvault.toml")
    };
    let mut config: CorpusConfig = if config_path.exists() {
        let s = fs::read_to_string(&config_path)?;
        toml::from_str(&s)?
    } else {
        CorpusConfig { path: corpus.to_string_lossy().to_string(), ..Default::default() }
    };

    let needs_dense =
        modes_list.iter().any(|m| matches!(m, RetrievalMode::Semantic | RetrievalMode::Full));
    if !needs_dense {
        config.index_mode = groundcontrol_common::config::IndexMode::Fast;
    }

    let engine = Engine::open(config, &index_dir)?;
    let summaries = BenchmarkSuite::evaluate_modes(&engine, &dataset, &modes_list, k, mod_enum)?;

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

        println!("Generated publication reports (.md, .csv) for [{prefix}] in: {}", dir.display());
    } else {
        output_report(&report, output.as_deref())?;
    }
    Ok(())
}
