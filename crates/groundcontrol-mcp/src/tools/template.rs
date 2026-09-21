//! Template and validation tools: `validate`, `list_templates`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use groundcontrol_common::ports::{GraphStore, MetadataCatalog};
use groundcontrol_common::{Error, Result};
use groundcontrol_core::engine::Engine;
use groundcontrol_core::template::Template;

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub path: Option<String>,
    pub check_taxonomy: Option<bool>,
    pub limit: Option<usize>,
}

/// Load templates for the corpus.
fn load_corpus_templates(engine: &Engine) -> Result<HashMap<String, Template>> {
    engine.load_templates()
}

/// Validate a single note against its declared template.
fn validate_single_note(
    engine: &Engine,
    path: &str,
) -> Result<groundcontrol_core::template::ValidationResult> {
    let corpus_path = PathBuf::from(&engine.config().path);
    let full_path = corpus_path.join(path);

    let content = fs::read_to_string(&full_path)
        .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?;

    let doc = groundcontrol_core::parser::parse_document(Path::new(path), &content)?;

    let template_name = doc.template.clone();

    let (valid, issues, tmpl_name) = if let Some(ref name) = template_name {
        let templates = load_corpus_templates(engine)?;
        if let Some(tmpl) = templates.get(name) {
            let mut issues = tmpl.validate(&doc.frontmatter, &doc.content);

            let files = engine.store().list_files().unwrap_or_default();
            let mut note_templates: HashMap<String, Option<String>> = HashMap::new();
            for f in files {
                let _ = note_templates.insert(f.path, f.template);
            }
            let edge_issues =
                tmpl.validate_edge_targets(&doc.frontmatter, &note_templates, |sym| {
                    engine.store().find_symbols_by_name(sym).map(|v| !v.is_empty()).unwrap_or(false)
                        || engine
                            .store()
                            .find_symbols_by_qualified_name(sym)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false)
                });
            issues.extend(edge_issues);

            let valid =
                !issues.iter().any(|i| i.severity == groundcontrol_core::template::Severity::Error);
            (valid, issues, Some(name.clone()))
        } else {
            let issues = vec![groundcontrol_core::template::ValidationIssue {
                severity: groundcontrol_core::template::Severity::Warning,
                message: format!("template '{}' not found in templates directory", name),
                field: Some("template".to_string()),
            }];
            (true, issues, Some(name.clone()))
        }
    } else {
        (true, Vec::new(), None)
    };

    Ok(groundcontrol_core::template::ValidationResult {
        path: path.to_string(),
        template: tmpl_name,
        valid,
        issues,
    })
}

/// Validate all templated notes in the corpus.
fn validate_corpus_notes(
    engine: &Engine,
    limit: Option<usize>,
) -> Result<Vec<groundcontrol_core::template::ValidationResult>> {
    let templates = load_corpus_templates(engine)?;
    let files = engine.store().list_files()?;
    let corpus_path = PathBuf::from(&engine.config().path);

    let mut note_templates: HashMap<String, Option<String>> = HashMap::new();
    for f in &files {
        let _ = note_templates.insert(f.path.clone(), f.template.clone());
    }

    let mut results: Vec<groundcontrol_core::template::ValidationResult> = Vec::new();

    for file in &files {
        let tmpl_name = match &file.template {
            Some(name) => name.clone(),
            None => continue,
        };

        let full_path = corpus_path.join(&file.path);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let doc = match groundcontrol_core::parser::parse_document(Path::new(&file.path), &content)
        {
            Ok(d) => d,
            Err(_) => continue,
        };

        let issues = if let Some(tmpl) = templates.get(&tmpl_name) {
            let mut issues = tmpl.validate(&doc.frontmatter, &doc.content);
            let edge_issues =
                tmpl.validate_edge_targets(&doc.frontmatter, &note_templates, |sym| {
                    engine.store().find_symbols_by_name(sym).map(|v| !v.is_empty()).unwrap_or(false)
                        || engine
                            .store()
                            .find_symbols_by_qualified_name(sym)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false)
                });
            issues.extend(edge_issues);
            issues
        } else {
            vec![groundcontrol_core::template::ValidationIssue {
                severity: groundcontrol_core::template::Severity::Warning,
                message: format!("template '{}' not found in templates directory", tmpl_name),
                field: Some("template".to_string()),
            }]
        };

        if !issues.is_empty() {
            let valid =
                !issues.iter().any(|i| i.severity == groundcontrol_core::template::Severity::Error);
            results.push(groundcontrol_core::template::ValidationResult {
                path: file.path.clone(),
                template: Some(tmpl_name),
                valid,
                issues,
            });
        }

        if let Some(limit_val) = limit {
            if results.len() >= limit_val {
                break;
            }
        }
    }

    Ok(results)
}

/// Validate structural ontology and graph integrity (broken links, cycle detection, orphan ADRs).
fn run_taxonomy_validation(engine: &Engine) -> Result<Value> {
    let files = engine.store().list_files()?;
    let existing_paths: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();

    let broken_links = engine.graph().detect_broken_links(&existing_paths);
    let circular_dependencies = engine.graph().detect_circular_dependencies(&[
        "supersedes",
        "depends_on",
        "implements",
        "parent_of",
    ]);

    let adr_paths: Vec<String> = files
        .iter()
        .filter(|f| {
            f.template.as_deref() == Some("adr")
                || f.template.as_deref() == Some("decision-record")
                || f.path.starts_with("docs/adrs/")
                || f.path.starts_with("adrs/")
        })
        .map(|f| f.path.clone())
        .collect();
    let orphan_adrs = engine.graph().detect_orphan_adrs(&adr_paths);

    let valid =
        broken_links.is_empty() && circular_dependencies.is_empty() && orphan_adrs.is_empty();

    Ok(serde_json::json!({
        "valid": valid,
        "broken_links_count": broken_links.len(),
        "broken_links": broken_links,
        "circular_dependencies_count": circular_dependencies.len(),
        "circular_dependencies": circular_dependencies,
        "orphan_adrs_count": orphan_adrs.len(),
        "orphan_adrs": orphan_adrs,
    }))
}

/// Unified validation tool: validates a single note, entire corpus notes against templates, and/or graph taxonomy.
pub fn handle_validate(engine: &Engine, args: Value) -> Result<Value> {
    let params: ValidateParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if let Some(path) = &params.path {
        let note_res = validate_single_note(engine, path)?;
        if params.check_taxonomy == Some(true) {
            let tax_res = run_taxonomy_validation(engine)?;
            let valid = note_res.valid && tax_res["valid"].as_bool().unwrap_or(true);
            Ok(serde_json::json!({
                "valid": valid,
                "note": note_res,
                "taxonomy": tax_res,
            }))
        } else {
            serde_json::to_value(note_res)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))
        }
    } else {
        let check_taxonomy = params.check_taxonomy.unwrap_or(true);
        let note_issues = validate_corpus_notes(engine, params.limit)?;
        let notes_valid = note_issues.is_empty() || note_issues.iter().all(|r| r.valid);

        if check_taxonomy {
            let tax_res = run_taxonomy_validation(engine)?;
            let tax_valid = tax_res["valid"].as_bool().unwrap_or(true);
            Ok(serde_json::json!({
                "valid": notes_valid && tax_valid,
                "notes_with_issues": note_issues,
                "taxonomy": tax_res,
            }))
        } else {
            Ok(serde_json::json!({
                "valid": notes_valid,
                "notes_with_issues": note_issues,
            }))
        }
    }
}

/// List all available templates.
pub fn handle_list_templates(engine: &Engine, _args: Value) -> Result<Value> {
    let templates = load_corpus_templates(engine)?;

    let mut list: Vec<&Template> = templates.values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));

    serde_json::to_value(list).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}
