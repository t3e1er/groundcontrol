//! Integration tests for the document parsing pipeline.

use std::path::PathBuf;

use groundcontrol_core::parser;

#[test]
fn parses_complete_document_with_frontmatter_and_wikilinks() {
    let content = r#"---
title: Architecture Decision
tags: [architecture, auth]
template: decision-record
status: accepted
parent: overview.md
---

# Architecture Decision: Use JWT

We decided to use [[jwt-tokens]] for authentication.
See also [[oauth-flow|OAuth implementation]] and [[session-management]].

## Context

The system needs stateless authentication. #security #performance

## Decision

Use JWT with short-lived access tokens.

## Consequences

- Stateless: no session store needed
- Links to [[refresh-tokens]] pattern
"#;

    let doc = parser::parse_document(&PathBuf::from("decisions/use-jwt.md"), content).unwrap();

    // Metadata
    assert_eq!(doc.title, Some("Architecture Decision".to_string()));
    assert_eq!(doc.template, Some("decision-record".to_string()));

    // Tags (from frontmatter + inline)
    assert!(doc.tags.contains(&"architecture".to_string()));
    assert!(doc.tags.contains(&"auth".to_string()));
    assert!(doc.tags.contains(&"security".to_string()));
    assert!(doc.tags.contains(&"performance".to_string()));

    // Wikilinks
    assert_eq!(doc.wikilinks.len(), 4);
    assert_eq!(doc.wikilinks[0].target, "jwt-tokens");
    assert_eq!(doc.wikilinks[1].target, "oauth-flow");
    assert_eq!(
        doc.wikilinks[1].alias,
        Some("OAuth implementation".to_string())
    );
    assert_eq!(doc.wikilinks[2].target, "session-management");
    assert_eq!(doc.wikilinks[3].target, "refresh-tokens");

    // Content hash is deterministic
    assert!(!doc.content_hash.is_empty());
    let doc2 = parser::parse_document(&PathBuf::from("decisions/use-jwt.md"), content).unwrap();
    assert_eq!(doc.content_hash, doc2.content_hash);
}

#[test]
fn parses_document_without_frontmatter() {
    let content = "# Simple Note\n\nJust some text with a [[link]].\n";
    let doc = parser::parse_document(&PathBuf::from("simple.md"), content).unwrap();

    assert_eq!(doc.title, Some("Simple Note".to_string()));
    assert_eq!(doc.frontmatter, None);
    assert_eq!(doc.template, None);
    assert_eq!(doc.wikilinks.len(), 1);
}
