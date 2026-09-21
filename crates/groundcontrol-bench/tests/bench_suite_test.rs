//! Benchmark suite integration tests for indexing profiler, dataset loader, IR metrics, and retrieval ablation.

use std::fs;
use tempfile::TempDir;

use groundcontrol_bench::dataset::loader::DatasetLoader;
use groundcontrol_bench::dataset::schema::{BenchmarkDataset, BenchmarkQuery, RelevanceJudgment};
use groundcontrol_bench::metrics::ir::IrEvaluator;
use groundcontrol_bench::metrics::latency::LatencyTracker;
use groundcontrol_bench::profile::disk::DiskProfiler;
use groundcontrol_bench::profile::index_profiler::{IndexProfiler, IndexProfilerOptions};
use groundcontrol_bench::profile::memory::MemoryTracker;
use groundcontrol_bench::report::{CsvReporter, JsonReporter, MarkdownReporter};
use groundcontrol_bench::runners::RetrievalMode;
use groundcontrol_bench::sweep::{BenchmarkSuite, BenchmarkSuiteReport};
use groundcontrol_common::config::CorpusConfig;
use groundcontrol_common::types::{Modality, SearchResult};
use groundcontrol_core::engine::Engine;

#[test]
fn test_ir_metrics_calculation() {
    let results = vec![
        SearchResult::new("doc1.md", 0.95),
        SearchResult::new("doc2.md", 0.85),
        SearchResult::new("doc3.md", 0.75),
        SearchResult::new("doc4.md", 0.65),
        SearchResult::new("doc5.md", 0.55),
    ];

    let judgments = vec![
        RelevanceJudgment::new("doc1.md", 3),
        RelevanceJudgment::new("doc3.md", 2),
        RelevanceJudgment::new("doc9.md", 1),
    ];

    let metrics = IrEvaluator::evaluate(&results, &judgments, 5);
    // 2 hits out of 3 expected in top 5
    assert_eq!(metrics.hits_at_k, 2);
    assert!((metrics.recall_at_k - (2.0 / 3.0)).abs() < 1e-4);
    assert_eq!(metrics.mrr_at_k, 1.0); // first hit at rank 1
    assert!(metrics.ndcg_at_k > 0.7);
    assert!(metrics.score_separation > 1.0);
}

#[test]
fn test_latency_tracker_percentiles() {
    let mut tracker = LatencyTracker::new();
    for i in 1..=100 {
        tracker.record(i as f64);
    }
    let stats = tracker.compute();
    assert_eq!(stats.count, 100);
    assert!((stats.p50_ms - 50.5).abs() < 1.0);
    assert!((stats.p90_ms - 90.1).abs() < 1.0);
    assert!((stats.p99_ms - 99.01).abs() < 1.0);
    assert_eq!(stats.min_ms, 1.0);
    assert_eq!(stats.max_ms, 100.0);
    assert!(stats.qps > 0.0);
}

#[test]
fn test_dataset_loader_parsing() {
    let json_legacy = r#"[
        {
            "id": "q1",
            "query": "authentication token",
            "expected_relevant": ["src/auth.rs"],
            "category": "security"
        }
    ]"#;
    let ds1 = DatasetLoader::load_from_str(json_legacy).unwrap();
    assert_eq!(ds1.queries.len(), 1);
    assert_eq!(ds1.queries[0].expected.len(), 1);
    assert_eq!(ds1.queries[0].expected[0].path, "src/auth.rs");
    assert_eq!(ds1.queries[0].expected[0].grade, 1);

    let json_graded = r#"{
        "queries": [
            {
                "id": "q2",
                "query": "error handling",
                "expected": [
                    {"path": "src/error.rs", "grade": 3},
                    {"path": "src/lib.rs", "grade": 1}
                ]
            }
        ]
    }"#;
    let ds2 = DatasetLoader::load_from_str(json_graded).unwrap();
    assert_eq!(ds2.queries.len(), 1);
    assert_eq!(ds2.queries[0].expected.len(), 2);
    assert_eq!(ds2.queries[0].expected[0].grade, 3);
}

#[test]
fn test_memory_and_disk_profilers() {
    let mut mem = MemoryTracker::start();
    let current = mem.sample();
    assert!(current > 0);
    let summary = mem.finish();
    assert!(summary.peak_rss_bytes >= summary.start_rss_bytes);

    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path();
    fs::write(corpus_dir.join("test.md"), "# Test Document\nBody text").unwrap();
    let index_dir = corpus_dir.join(".index");
    fs::create_dir_all(&index_dir).unwrap();
    fs::write(index_dir.join("meta.db"), "dummy sqlite bytes").unwrap();

    let disk = DiskProfiler::profile(corpus_dir).unwrap();
    assert_eq!(disk.source_file_count, 1);
    assert!(disk.source_bytes > 0);
    assert_eq!(disk.meta_db_bytes, "dummy sqlite bytes".len() as u64);
    assert!(disk.total_index_bytes > 0);
    assert!(disk.expansion_ratio > 0.0);
}

#[test]
fn test_end_to_end_indexing_and_retrieval_bench() {
    let tmp = TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();

    // Create a markdown doc and a code file
    fs::write(
        corpus_dir.join("auth.md"),
        "# Authentication Service\nExplains JWT tokens and session validation.",
    )
    .unwrap();

    fs::write(
        corpus_dir.join("auth.rs"),
        "pub fn validate_jwt_token(token: &str) -> bool { true }\npub fn login_handler() {}",
    )
    .unwrap();

    // 1. Profile indexing pipeline
    let opts = IndexProfilerOptions { include_dense_embedding: false, clean_cold_start: true };
    let idx_report = IndexProfiler::profile(&corpus_dir, &opts).unwrap();
    assert_eq!(idx_report.documents_indexed, 2);
    assert!(idx_report.total_elapsed_ms > 0.0);
    assert!(idx_report.docs_per_second > 0.0);
    assert!(idx_report.memory.peak_rss_bytes > 0);
    assert!(idx_report.disk.source_file_count >= 2);

    // 2. Run retrieval ablation suite
    let config = CorpusConfig {
        path: corpus_dir.to_string_lossy().to_string(),
        index_mode: groundcontrol_common::config::IndexMode::Fast,
        ..Default::default()
    };
    let engine = Engine::open(config, &corpus_dir.join(".index")).unwrap();

    let dataset = BenchmarkDataset {
        name: Some("test_corpus".to_string()),
        queries: vec![
            BenchmarkQuery::simple(
                "q1",
                "validate_jwt_token",
                vec!["auth.rs".to_string()],
                Some("code".into()),
            ),
            BenchmarkQuery::simple(
                "q2",
                "JWT tokens and session",
                vec!["auth.md".to_string()],
                Some("doc".into()),
            ),
        ],
    };

    let modes = vec![RetrievalMode::Bm25, RetrievalMode::Binary, RetrievalMode::Fast];
    let summaries =
        BenchmarkSuite::evaluate_modes(&engine, &dataset, &modes, 5, Modality::Both).unwrap();
    assert_eq!(summaries.len(), 3);

    let suite_report = BenchmarkSuiteReport {
        timestamp_unix: 1700000000,
        query_count: dataset.queries.len(),
        k: 5,
        modes: summaries,
        indexing: Some(idx_report),
        benchmark: Some("unit_test".to_string()),
        repository: Some("test_repo".to_string()),
    };

    // 3. Verify formatters
    let md = MarkdownReporter::render(&suite_report);
    assert!(md.contains("# groundcontrol Retrieval & Indexing Benchmark Report"));
    assert!(md.contains("Indexing Performance"));
    assert!(md.contains("Retrieval Algorithm Quality"));

    let json_str = JsonReporter::to_string(&suite_report).unwrap();
    assert!(json_str.contains("\"query_count\": 2"));

    let csv_str = CsvReporter::render_retrieval_csv(&suite_report);
    assert!(csv_str.contains("benchmark,repository,mode,k,mean_recall"));
    assert!(csv_str.contains("unit_test,test_repo,bm25,5"));
    assert!(csv_str.contains("bm25,5"));
    assert!(csv_str.contains("binary,5"));
    assert!(csv_str.contains("fast,5"));
}

#[test]
fn test_retrieval_benchmark_no_zero_recalls() {
    use groundcontrol_bench::runners::QueryRunner;
    use groundcontrol_bench::runners::QueryRunnerOptions;
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let gotilla_dir = root_dir.join("benchmarks/workspace/repos/codesearchnet/grokify__gotilla");
    if gotilla_dir.join(".index").exists() {
        let config =
            CorpusConfig { path: gotilla_dir.to_string_lossy().to_string(), ..Default::default() };
        let engine = Engine::open(config, &gotilla_dir.join(".index")).unwrap();
        let queries = vec![
            BenchmarkQuery::simple(
                "csn_Go_00044",
                "convert a date string into yyyymmdd",
                vec!["time/timeutil/timeutil.go".into(), "time/timeutil/dt8.go".into()],
                None,
            ),
            BenchmarkQuery::simple(
                "csn_Go_00126",
                "how to randomly pick a number",
                vec!["strconv/phonenumber/fictitiousgenerator.go".into()],
                None,
            ),
        ];

        let opts = QueryRunnerOptions { limit: 10, modality: Modality::Code, decompose: false };
        for mode in
            &[RetrievalMode::Bm25, RetrievalMode::Binary, RetrievalMode::Ppr, RetrievalMode::Fast]
        {
            let mut total_recall = 0.0;
            for q in &queries {
                let (res, _) = QueryRunner::execute(&engine, q, *mode, &opts).unwrap();
                let ir = IrEvaluator::evaluate(&res, &q.expected, 10);
                total_recall += ir.recall_at_k;
            }
            let avg_recall = total_recall / queries.len() as f64;
            assert!(
                avg_recall > 0.0,
                "Gotilla average recall for {:?} must be > 0.000, got {}",
                mode,
                avg_recall
            );
        }
    }

    let tiny_dir = root_dir.join("benchmarks/workspace/repos/repobench/DLYuanGod__TinyGPT-V");
    if tiny_dir.join(".index").exists() {
        let config =
            CorpusConfig { path: tiny_dir.to_string_lossy().to_string(), ..Default::default() };
        let engine = Engine::open(config, &tiny_dir.join(".index")).unwrap();
        let q = BenchmarkQuery::simple(
            "rb_0",
            "import re\nfrom minigpt4.common.registry import registry\nfrom minigpt4.processors.base_processor import BaseProcessor\nfrom minigpt4.processors.randaugment import RandomAugment\nfrom omegaconf import OmegaConf\nfrom torchvision import transforms\nfrom torchvision.transforms.functional import InterpolationMode class BlipImageBaseProcessor(BaseProcessor):",
            vec!["minigpt4/processors/blip_processors.py".into()],
            None,
        );
        let opts = QueryRunnerOptions { limit: 10, modality: Modality::Code, decompose: false };
        for mode in
            &[RetrievalMode::Bm25, RetrievalMode::Binary, RetrievalMode::Ppr, RetrievalMode::Fast]
        {
            let (res, _) = QueryRunner::execute(&engine, &q, *mode, &opts).unwrap();
            let ir = IrEvaluator::evaluate(&res, &q.expected, 10);
            assert!(
                ir.recall_at_k > 0.0,
                "TinyGPT-V recall for {:?} must be > 0.000, got {}",
                mode,
                ir.recall_at_k
            );
        }
    }
}
