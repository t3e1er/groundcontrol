use super::hit::{clean_path, deduplicate_hits, is_file_path, is_non_code_path};
use super::sanitizer::sanitize_lucene_query;
use groundcontrol_common::types::Modality;

#[test]
fn test_sanitize_lucene_query() {
    assert_eq!(sanitize_lucene_query("foo:bar"), "foo bar");
    assert_eq!(sanitize_lucene_query("foo AND bar"), "foo and bar");
    assert_eq!(sanitize_lucene_query("```rust\nlet x = 1;\n```"), "let x 1");
    assert_eq!(sanitize_lucene_query("foo+bar!baz"), "foo bar baz");
}

#[test]
fn test_clean_path() {
    assert_eq!(clean_path("src/lib.rs:chunk:1"), "src/lib.rs");
    assert_eq!(clean_path("src/lib.rs#MyStruct"), "src/lib.rs");
    assert_eq!(clean_path("TinyGPT-V-main/src/main.rs"), "src/main.rs");
    assert_eq!(clean_path("docs/readme.md"), "docs/readme.md");
}

#[test]
fn test_is_file_path() {
    assert!(is_file_path("src/lib.rs"));
    assert!(is_file_path("README.md"));
    assert!(is_file_path(r"src\engine.rs"));
    assert!(!is_file_path("SymbolOnly"));
}

#[test]
fn test_is_non_code_path() {
    assert!(is_non_code_path("docs/adr-001.md"));
    assert!(is_non_code_path("config.toml"));
    assert!(is_non_code_path("package.json"));
    assert!(!is_non_code_path("src/main.rs"));
    assert!(!is_non_code_path("app.py"));
}

#[test]
fn test_deduplicate_hits() {
    let raw = vec![
        ("src/lib.rs:chunk:0".to_string(), 0.9, Some("MyStruct".to_string())),
        ("src/lib.rs:chunk:1".to_string(), 0.8, None),
        ("docs/readme.md:chunk:0".to_string(), 0.95, None),
    ];

    let hits_code = deduplicate_hits(raw.clone(), 10, Modality::Code, "mystruct");
    assert_eq!(hits_code.len(), 1);
    assert_eq!(hits_code[0].path, "src/lib.rs");
    assert_eq!(hits_code[0].symbol.as_deref(), Some("MyStruct"));

    let hits_both = deduplicate_hits(raw, 10, Modality::Both, "test");
    assert_eq!(hits_both.len(), 2);
}
