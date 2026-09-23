//! Tests for composite multi-algorithm retrieval.

use super::*;
use groundcontrol_common::types::SearchResult;

#[test]
fn test_rrf_merge_ranks() {
    let list1 = vec![
        SearchResult::new("src/main.rs".to_string(), 10.0),
        SearchResult::new("src/lib.rs".to_string(), 8.0),
    ];
    let list2 = vec![
        SearchResult::new("src/lib.rs".to_string(), 0.9),
        SearchResult::new("src/util.rs".to_string(), 0.7),
    ];

    let merged = rrf_merge(&[&list1, &list2], 5, 60.0);
    assert_eq!(merged.len(), 3);
    // src/lib.rs appears in both lists, so its combined RRF score is highest:
    // (1/62) + (1/61) > 1/61
    assert_eq!(merged[0].path, "src/lib.rs");
}
