use groundcontrol_common::ports::RetrievalAlgorithm;
use groundcontrol_common::types::{Chunk, FileFormat, Modality, ParsedArtifact};

use super::projector::BinaryV2Projector;
use super::rri::RriEngine;
use super::BinaryV2Algorithm;

#[test]
fn test_binaryv2_all_channels_active() {
    let projector = BinaryV2Projector::new();
    let rri = RriEngine::new();
    let fp = projector.project_query("calculate shipping quote and parcel tracking ID", &rri);

    // Ensure all 4 channels are non-zero!
    assert_ne!(fp.0[0], 0, "Channel 0 (lexical) should be active");
    assert_ne!(fp.0[1], 0, "Channel 1 (rri) should be active");
    assert_ne!(fp.0[2], 0, "Channel 2 (graph/api) should be active");
    assert_ne!(fp.0[3], 0, "Channel 3 (path/structure) should be active");
}

#[test]
fn test_binaryv2_semantic_bridging() {
    let projector = BinaryV2Projector::new();
    let mut rri = RriEngine::new();

    // Train co-occurrence: "credit", "card", "charge", "payment" appear together
    for _ in 0..10 {
        rri.train_chunk(&[
            "credit".to_string(),
            "card".to_string(),
            "charge".to_string(),
            "payment".to_string(),
            "authorization".to_string(),
        ]);
    }

    let q_payment =
        projector.project_query("credit card charge and transaction authorization", &rri);

    let sym_payment = groundcontrol_common::types::CodeSymbol {
        name: "process_payment".to_string(),
        file_path: "src/paymentservice/charge.js".to_string(),
        language: "javascript".to_string(),
        symbol_type: groundcontrol_common::types::CodeSymbolType::Function,
        scope_path: "process_payment".to_string(),
        signature: "function process_payment(card_number, amount)".to_string(),
        docstring: Some("Processes credit card charge and billing authorization".to_string()),
        start_line: 1,
        end_line: 20,
    };
    let doc_payment =
        projector.project_symbol(&sym_payment, "src/paymentservice/charge.js", None, &rri);

    let dist_payment = q_payment.hamming_distance(&doc_payment);

    let sym_unrelated = groundcontrol_common::types::CodeSymbol {
        name: "calculate_matrix_determinant".to_string(),
        file_path: "src/math/matrix.rs".to_string(),
        language: "rust".to_string(),
        symbol_type: groundcontrol_common::types::CodeSymbolType::Function,
        scope_path: "calculate_matrix_determinant".to_string(),
        signature: "pub fn calculate_matrix_determinant(m: &Matrix) -> f64".to_string(),
        docstring: Some("Computes linear algebra determinant".to_string()),
        start_line: 1,
        end_line: 20,
    };
    let doc_unrelated = projector.project_symbol(&sym_unrelated, "src/math/matrix.rs", None, &rri);

    let dist_unrelated = q_payment.hamming_distance(&doc_unrelated);

    assert!(
        dist_payment < dist_unrelated,
        "Semantic matching should produce lower Hamming distance: payment {} vs unrelated {}",
        dist_payment,
        dist_unrelated
    );
}

#[test]
fn test_binaryv2_algorithm_retrieval() {
    let mut algo = BinaryV2Algorithm::new();

    let doc = ParsedArtifact {
        path: "src/shippingservice/src/shipping_service/quote.rs".to_string(),
        hash: "hash_quote".to_string(),
        is_code: true,
        format: FileFormat::Source,
        title: Some("Shipping Quote".to_string()),
        doc_metadata: None,
        symbols: vec![groundcontrol_common::types::CodeSymbol {
            name: "create_quote_from_count".to_string(),
            file_path: "src/shippingservice/src/shipping_service/quote.rs".to_string(),
            language: "rust".to_string(),
            symbol_type: groundcontrol_common::types::CodeSymbolType::Function,
            scope_path: "create_quote_from_count".to_string(),
            signature: "pub async fn create_quote_from_count(count: u32) -> Result<Quote, tonic::Status>".to_string(),
            docstring: Some("Check product catalog for price on each item and create shipping quote".to_string()),
            start_line: 19,
            end_line: 38,
        }],
        grammar_semantics: Vec::new(),
        chunks: vec![Chunk::new(
            "src/shippingservice/src/shipping_service/quote.rs".to_string(),
            0,
            "pub async fn create_quote_from_count(count: u32) -> Result<Quote, Status> { request_quote(count).await }".to_string(),
            0,
            100,
        )],
        graph_edges: Vec::new(),
        external_refs: Vec::new(),
        raw_content: Some("pub async fn create_quote_from_count(count: u32) -> Result<Quote, Status> { request_quote(count).await }".to_string()),
        projection_text: None,
    };

    algo.index_document(&doc).unwrap();

    let results =
        algo.search("calculate shipping quote and parcel tracking ID", 5, Modality::Code).unwrap();

    assert!(!results.is_empty(), "Should return indexed quote result");
    assert_eq!(results[0].path, "src/shippingservice/src/shipping_service/quote.rs");
}
