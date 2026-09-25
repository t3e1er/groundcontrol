//! Unit tests for binaryv3 retrieval algorithm, projector, and Bayesian prior flags.

use groundcontrol_common::types::{CodeSymbol, CodeSymbolType, Modality};

use super::index::BinaryV3SearchIndex;
use super::projector::BinaryV3Projector;
use super::tokenizer::{expand_tokens_morphology, stem_suffix, tokenize_code_text};
use super::types::{BinaryV3Config, EntityPriorFlags, FingerprintV3Record};

#[test]
fn test_prior_flags_multiplier() {
    let base = EntityPriorFlags::default();
    assert_eq!(base.compute_multiplier(), 1.0);

    // Exported function
    let exported_fn = EntityPriorFlags(
        EntityPriorFlags::IS_PUBLIC_EXPORT
            | EntityPriorFlags::KIND_FUNCTION
            | EntityPriorFlags::IS_ROOT_SCOPE,
    );
    let m = exported_fn.compute_multiplier();
    // 1.15 * 1.10 * 1.05 = 1.32825
    assert!((m - 1.32825).abs() < 1e-4);

    // Exported API route
    let exported_route =
        EntityPriorFlags(EntityPriorFlags::IS_PUBLIC_EXPORT | EntityPriorFlags::KIND_ROUTE_API);
    let m_route = exported_route.compute_multiplier();
    // 1.15 * 1.20 = 1.38
    assert!((m_route - 1.38).abs() < 1e-4);

    // Test function penalty
    let test_fn =
        EntityPriorFlags(EntityPriorFlags::IS_TEST_OR_MOCK | EntityPriorFlags::KIND_FUNCTION);
    let m_test = test_fn.compute_multiplier();
    // 1.10 * 0.75 = 0.825
    assert!((m_test - 0.825).abs() < 1e-4);
}

#[test]
fn test_prior_flags_inference() {
    let sym = CodeSymbol {
        file_path: "src/shippingservice/src/quote.rs".to_string(),
        name: "get_quote".to_string(),
        symbol_type: CodeSymbolType::Function,
        scope_path: "shipping_service::get_quote".to_string(),
        language: "rust".to_string(),
        signature: "pub fn get_quote(req: QuoteRequest) -> Result<QuoteResponse>".to_string(),
        docstring: Some("Calculate shipping quote".to_string()),
        start_line: 10,
        end_line: 25,
    };

    let flags = EntityPriorFlags::from_symbol(&sym, "src/shippingservice/src/quote.rs");
    assert_ne!(flags.0 & EntityPriorFlags::IS_PUBLIC_EXPORT, 0);
    assert_ne!(flags.0 & EntityPriorFlags::KIND_ROUTE_API, 0); // Contains "quote" and "request"
    assert_eq!(flags.0 & EntityPriorFlags::IS_TEST_OR_MOCK, 0);

    let test_sym = CodeSymbol {
        file_path: "tests/quote_test.rs".to_string(),
        name: "test_quote_calculation".to_string(),
        symbol_type: CodeSymbolType::Function,
        scope_path: "test::test_quote_calculation".to_string(),
        language: "rust".to_string(),
        signature: "fn test_quote_calculation()".to_string(),
        docstring: None,
        start_line: 1,
        end_line: 10,
    };

    let test_flags = EntityPriorFlags::from_symbol(&test_sym, "tests/quote_test.rs");
    assert_ne!(test_flags.0 & EntityPriorFlags::IS_TEST_OR_MOCK, 0);
}

#[test]
fn test_tokenizer_and_abbreviations() {
    let tokens = tokenize_code_text("ShippingServiceController");
    assert_eq!(tokens, vec!["shipping", "service", "controller"]);

    let expanded = expand_tokens_morphology(&tokens);
    let texts: Vec<&str> = expanded.iter().map(|e| e.text.as_str()).collect();
    assert!(texts.contains(&"ship")); // Stemmed
    assert!(texts.contains(&"shipping"));

    assert_eq!(stem_suffix("calculation"), Some("calculat".to_string()));
    assert_eq!(stem_suffix("currencies"), Some("currency".to_string()));
}

#[test]
fn test_projector_semantic_affinity() {
    let projector = BinaryV3Projector::default();

    let fp_query = projector.project_query("calculate shipping quote");
    let fp_target = projector.project_query("shipping quote calculation service");
    let fp_unrelated = projector.project_query("kubernetes volume claim pod controller");

    let dist_target = fp_query.hamming_distance(&fp_target);
    let dist_unrelated = fp_query.hamming_distance(&fp_unrelated);

    assert!(
        dist_target < dist_unrelated,
        "Target dist {dist_target} should be less than unrelated dist {dist_unrelated}"
    );
}

#[test]
fn test_index_bayesian_prior_rescoring() {
    let mut index = BinaryV3SearchIndex::new();
    let projector = index.projector().clone();

    let fp_prod = projector.project_query("charge credit card transaction");
    let fp_test = projector.project_query("charge credit card transaction test mock");

    // Production symbol
    let prod_flags = EntityPriorFlags(
        EntityPriorFlags::IS_PUBLIC_EXPORT
            | EntityPriorFlags::KIND_ROUTE_API
            | EntityPriorFlags::IS_ROOT_SCOPE,
    );
    // Test symbol
    let test_flags =
        EntityPriorFlags(EntityPriorFlags::IS_TEST_OR_MOCK | EntityPriorFlags::KIND_FUNCTION);

    index
        .index_fingerprints(&[
            FingerprintV3Record {
                id: "src/paymentservice/charge.js".into(),
                fingerprint: fp_prod,
                modality: Modality::Code,
                flags: prod_flags,
            },
            FingerprintV3Record {
                id: "src/paymentservice/test/charge.test.js".into(),
                fingerprint: fp_test,
                modality: Modality::Code,
                flags: test_flags,
            },
        ])
        .unwrap();

    let query_bits = projector.project_query("authorize credit card charge");
    let results = index.search_candidates(&query_bits, 5, Modality::Code).unwrap();

    assert!(!results.is_empty());
    // Production file should score significantly higher due to prior multiplier
    assert_eq!(results[0].0, "src/paymentservice/charge.js");
    assert!(results[0].2 > results[1].2);
}

#[test]
fn test_matryoshka_early_exit() {
    let mut index = BinaryV3SearchIndex::new();
    let projector = index.projector().clone();

    let fp_auth = projector.project_query("authentication jwt login validator");
    let fp_unrelated = projector.project_query("kubernetes cluster storage persistent volume");

    index
        .index_fingerprints(&[
            FingerprintV3Record {
                id: "src/auth.rs".into(),
                fingerprint: fp_auth,
                modality: Modality::Code,
                flags: EntityPriorFlags::default(),
            },
            FingerprintV3Record {
                id: "src/storage.rs".into(),
                fingerprint: fp_unrelated,
                modality: Modality::Code,
                flags: EntityPriorFlags::default(),
            },
        ])
        .unwrap();

    let mut cfg = BinaryV3Config::default();
    cfg.early_exit = true;
    cfg.early_exit_threshold = 30; // Strict threshold on Word 0
    index.set_config(cfg);

    let query_bits = projector.project_query("jwt token auth");
    let results = index.search_candidates(&query_bits, 5, Modality::Code).unwrap();

    assert!(!results.is_empty());
    assert_eq!(results[0].0, "src/auth.rs");
}
