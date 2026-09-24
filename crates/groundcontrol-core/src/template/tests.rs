use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use groundcontrol_common::config::{EdgeClass, EdgeDirection};

use super::*;

fn sample_adr_markdown() -> &'static str {
    r#"---
template:
  name: adr
  description: "Architecture Decision Record"
  target_dir: "docs/architecture/adr"

schema:
  fields:
    status:
      type: enum
      required: true
      values: [proposed, accepted, deprecated, superseded]
    date:
      type: date
      required: true
    deciders:
      type: list
      required: false

  edges:
    - field: supersedes
      type: Supersedes
      class: structural
      direction: outbound
      bidirectional: false
      target_template: adr
      required: false
      description: "Previous decision superseded by this one"
    - field: implements
      type: ImplementsSpec
      class: crossmodal
      target_kind: code_symbol
      required: false

  sections:
    required: ["Context", "Decision", "Consequences"]
  min_words: 50
---
# ADR-{id}: {Title}

<!-- Guidance comment -->

## Context
Background context here.

## Decision
Decision details here.

## Consequences
Consequences details here.
"#
}

#[test]
fn test_load_markdown_template() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("adr.md");
    fs::write(&path, sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    assert_eq!(templates.len(), 1);

    let tmpl = templates.get("adr").unwrap();
    assert_eq!(tmpl.name, "adr");
    assert_eq!(tmpl.description, Some("Architecture Decision Record".to_string()));
    assert_eq!(tmpl.target_dir, Some("docs/architecture/adr".to_string()));
    assert_eq!(tmpl.required_fields.len(), 2);
    assert_eq!(tmpl.optional_fields.len(), 1);
    assert_eq!(tmpl.edges.len(), 2);
    assert_eq!(tmpl.edges[0].field, "supersedes");
    assert_eq!(tmpl.edges[0].edge_type, "Supersedes");
    assert_eq!(tmpl.edges[0].class, EdgeClass::Structural);
    assert_eq!(tmpl.edges[0].direction, EdgeDirection::Outbound);
    assert_eq!(tmpl.edges[0].target_template, Some("adr".to_string()));

    assert_eq!(tmpl.required_sections, vec!["Context", "Decision", "Consequences"]);
    assert_eq!(tmpl.min_word_count, Some(50));
    assert!(tmpl.scaffold.contains("# ADR-{id}: {Title}"));
    assert!(tmpl.scaffold.contains("<!-- Guidance comment -->"));
}

#[test]
fn test_load_from_nonexistent_dir() {
    let templates = Template::load_from_dir(Path::new("/nonexistent/dir")).unwrap();
    assert!(templates.is_empty());
}

#[test]
fn test_validate_valid_note() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("adr.md"), sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    let tmpl = templates.get("adr").unwrap();

    let frontmatter = serde_json::json!({
        "template": "adr",
        "status": "accepted",
        "date": "2026-09-11",
        "supersedes": "docs/architecture/adr/001.md"
    });

    let content = r#"# ADR-002: Choose Database

## Context

We need to choose a database for our new service. The current system uses PostgreSQL
but we are evaluating alternatives for better scalability.

## Decision

We will use PostgreSQL with read replicas for horizontal read scaling. This leverages
our existing expertise and tooling while addressing the scalability concern.

## Consequences

This means we need to set up replication infrastructure and handle eventual consistency
in read paths. The team is familiar with PostgreSQL so onboarding cost is low.
"#;

    let issues = tmpl.validate(&Some(frontmatter), content);
    assert!(issues.is_empty(), "Valid note should have no issues: {:?}", issues);
}

#[test]
fn test_validate_missing_required_field() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("adr.md"), sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    let tmpl = templates.get("adr").unwrap();

    // Missing 'date'
    let frontmatter = serde_json::json!({
        "template": "adr",
        "status": "accepted"
    });

    let content = "## Context\n\n## Decision\n\n## Consequences\n\nEnough words here to pass the minimum word count requirement for the template validation check.\nExtra words to pad out.Extra words to pad out.Extra words to pad out.Extra words to pad out.\n";

    let issues = tmpl.validate(&Some(frontmatter), content);
    assert!(!issues.is_empty());
    let missing =
        issues.iter().find(|i| i.field.as_deref() == Some("date") && i.severity == Severity::Error);
    assert!(missing.is_some(), "Should report missing 'date' field");
}

#[test]
fn test_validate_invalid_enum() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("adr.md"), sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    let tmpl = templates.get("adr").unwrap();

    let frontmatter = serde_json::json!({
        "template": "adr",
        "status": "invalid-status",
        "date": "2026-09-11"
    });

    let content = "## Context\n\n## Decision\n\n## Consequences\n\nEnough words here to pass the minimum word count requirement for the template validation check.\nExtra words to pad out.Extra words to pad out.Extra words to pad out.Extra words to pad out.\n";

    let issues = tmpl.validate(&Some(frontmatter), content);
    let enum_issue = issues
        .iter()
        .find(|i| i.field.as_deref() == Some("status") && i.severity == Severity::Error);
    assert!(enum_issue.is_some(), "Should report invalid enum value: {:?}", issues);
}

#[test]
fn test_validate_missing_section() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("adr.md"), sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    let tmpl = templates.get("adr").unwrap();

    let frontmatter = serde_json::json!({
        "template": "adr",
        "status": "accepted",
        "date": "2026-09-11"
    });

    // Missing "Decision" section
    let content = "## Context\n\nSome context.\n\n## Consequences\n\nThe consequences are significant and broad enough to exceed word count minimums easily.\nExtra words to pad out.Extra words to pad out.Extra words to pad out.Extra words to pad out.\n";

    let issues = tmpl.validate(&Some(frontmatter), content);
    let section_issue =
        issues.iter().find(|i| i.message.contains("Decision") && i.severity == Severity::Error);
    assert!(section_issue.is_some(), "Should report missing 'Decision' section: {:?}", issues);
}

#[test]
fn test_validate_edge_targets() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("adr.md"), sample_adr_markdown()).unwrap();

    let templates = Template::load_from_dir(tmp.path()).unwrap();
    let tmpl = templates.get("adr").unwrap();

    let frontmatter = serde_json::json!({
        "supersedes": "docs/architecture/adr/001.md",
        "implements": "SearchService"
    });

    let mut note_templates = HashMap::new();
    let _ =
        note_templates.insert("docs/architecture/adr/001.md".to_string(), Some("adr".to_string()));

    let issues = tmpl
        .validate_edge_targets(&Some(frontmatter), &note_templates, |sym| sym == "SearchService");
    assert!(
        issues.is_empty(),
        "Should have no issues when target note and symbol exist: {:?}",
        issues
    );

    // Test mismatched template
    let mut mismatched = HashMap::new();
    let _ = mismatched.insert("docs/architecture/adr/001.md".to_string(), Some("rfc".to_string()));
    let issues = tmpl.validate_edge_targets(
        &Some(serde_json::json!({ "supersedes": "docs/architecture/adr/001.md" })),
        &mismatched,
        |_| true,
    );
    assert!(
        issues
            .iter()
            .any(|i| i.severity == Severity::Error
                && i.message.contains("expected template 'adr'")),
        "Expected template mismatch error: {:?}",
        issues
    );
}

#[test]
fn test_load_repo_templates_coverage() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (resolved, templates) = Template::discover_and_load(&repo_root, None).unwrap();
    if let Some(dir) = resolved {
        assert_eq!(dir, PathBuf::from("docs/.templates"));
        let expected = ["adr", "architecture", "concept", "guide", "hub", "rfc", "roadmap"];
        for name in &expected {
            assert!(
                templates.contains_key(*name),
                "Expected template '{}' in docs/.templates dir",
                name
            );
            let tmpl = templates.get(*name).unwrap();
            assert!(!tmpl.scaffold.is_empty());
            assert!(!tmpl.required_sections.is_empty());
            assert!(tmpl.source_path.is_some());
        }
    }
}

#[test]
fn test_discover_and_load_precedence_and_fallback() {
    let tmp = TempDir::new().unwrap();
    let corpus_path = tmp.path();

    // 1. When empty, returns None and empty map
    let (resolved, templates) = Template::discover_and_load(corpus_path, None).unwrap();
    assert_eq!(resolved, None);
    assert!(templates.is_empty());

    // 2. Create .templates and docs/.templates with distinct templates
    let dot_templates = corpus_path.join(".templates");
    fs::create_dir_all(&dot_templates).unwrap();
    fs::write(dot_templates.join("adr.md"), sample_adr_markdown()).unwrap();

    let docs_templates = corpus_path.join("docs").join(".templates");
    fs::create_dir_all(&docs_templates).unwrap();
    let custom_guide = r#"---
template:
  name: guide
  description: "Test Guide"
schema:
  fields:
    title: { type: string, required: true }
  sections: ["Overview"]
---
# Guide
## Overview
Scaffold
"#;
    fs::write(docs_templates.join("guide.md"), custom_guide).unwrap();

    // 3. Auto-discovery should prioritize docs/.templates over .templates
    let (resolved, templates) = Template::discover_and_load(corpus_path, None).unwrap();
    assert_eq!(resolved, Some(PathBuf::from("docs/.templates")));
    assert!(templates.contains_key("guide"));
    assert!(!templates.contains_key("adr"));

    // 4. Explicit configuration overrides auto-discovery priority
    let (resolved, templates) =
        Template::discover_and_load(corpus_path, Some(".templates")).unwrap();
    assert_eq!(resolved, Some(PathBuf::from(".templates")));
    assert!(templates.contains_key("adr"));
    assert!(!templates.contains_key("guide"));

    // 5. Explicit non-existent directory falls back to auto-discovery
    let (resolved, templates) =
        Template::discover_and_load(corpus_path, Some("nonexistent/templates")).unwrap();
    assert_eq!(resolved, Some(PathBuf::from("docs/.templates")));
    assert!(templates.contains_key("guide"));
}
