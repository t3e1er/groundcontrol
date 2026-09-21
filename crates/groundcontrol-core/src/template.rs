//! Template system: loading, validation, and schema enforcement.
//!
//! Templates are Markdown files (`.templates/*.md`) in the corpus templates directory.
//! Each template defines required/optional frontmatter fields, declarative graph edge
//! relationships, and content structure rules in its YAML frontmatter, with the
//! markdown body serving as the starter scaffolding.
//!
//! Notes declare which template they follow via a `template:` frontmatter field.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use groundcontrol_common::config::TemplateEdgeSchema;
use groundcontrol_common::{Error, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A parsed template definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Template name (used in frontmatter `template:` field).
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Optional default target directory for notes created with this template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<String>,
    /// Fields that must be present in frontmatter.
    pub required_fields: Vec<FieldSchema>,
    /// Fields that may be present in frontmatter.
    pub optional_fields: Vec<FieldSchema>,
    /// Declarative edge rules defined for this template.
    #[serde(default)]
    pub edges: Vec<TemplateEdgeSchema>,
    /// Headings that must exist in the content body.
    pub required_sections: Vec<String>,
    /// Minimum word count for the content body.
    pub min_word_count: Option<usize>,
    /// The unparsed markdown body below the frontmatter used as a starter scaffold.
    #[serde(default)]
    pub scaffold: String,
    /// Path to the template file on disk (relative or absolute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// Schema for a frontmatter field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field name (key in frontmatter).
    pub name: String,
    /// Expected type of the field value.
    pub field_type: FieldType,
    /// Allowed values (only for `Enum` type).
    pub values: Option<Vec<String>>,
}

/// Supported field types for validation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// Free-form text.
    String,
    /// Date in YYYY-MM-DD format.
    Date,
    /// One of a fixed set of allowed values.
    Enum,
    /// An array of values.
    List,
    /// A file path reference.
    Path,
    /// An array of file path references.
    #[serde(rename = "listofpaths")]
    ListOfPaths,
    /// A numeric value.
    Number,
    /// A true/false value.
    Boolean,
}

/// A validation issue found during note checking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationIssue {
    /// How severe this issue is.
    pub severity: Severity,
    /// Human-readable description of the problem.
    pub message: String,
    /// Which frontmatter field is involved (if applicable).
    pub field: Option<String>,
}

/// Issue severity level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The note violates a hard requirement (field missing, wrong type, etc.).
    Error,
    /// The note has a soft issue (word count too low, etc.).
    Warning,
}

/// Result of validating a note against its template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Path of the validated note.
    pub path: String,
    /// Template name it was validated against (if any).
    pub template: Option<String>,
    /// Whether the note passed validation with no errors.
    pub valid: bool,
    /// All issues found.
    pub issues: Vec<ValidationIssue>,
}

// ---------------------------------------------------------------------------
// YAML deserialization helpers (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct TemplateFrontmatterYaml {
    #[serde(default)]
    template: Option<TemplateHeaderYaml>,
    #[serde(default)]
    schema: Option<TemplateSchemaYaml>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TemplateHeaderYaml {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    target_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TemplateSchemaYaml {
    #[serde(default)]
    fields: Option<FieldsConfig>,
    #[serde(default)]
    edges: Option<Vec<TemplateEdgeSchema>>,
    #[serde(default)]
    sections: Option<SectionsConfig>,
    #[serde(default)]
    min_words: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum FieldsConfig {
    Map(BTreeMap<String, FieldDefYaml>),
    List(Vec<FieldDefListYaml>),
}

#[derive(Debug, Clone, Deserialize)]
struct FieldDefYaml {
    #[serde(rename = "type")]
    field_type: FieldType,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct FieldDefListYaml {
    name: String,
    #[serde(rename = "type")]
    field_type: FieldType,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SectionsConfig {
    List(Vec<String>),
    Detailed(SectionRulesYaml),
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SectionRulesYaml {
    #[serde(default)]
    required: Vec<String>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl Template {
    /// Standard candidate relative template directories probed in priority order.
    pub const CANDIDATE_TEMPLATE_DIRS: &'static [&'static str] =
        &["docs/.templates", ".templates", ".groundcontrol/templates", "docs/templates"];

    /// Discover and load templates for a corpus.
    ///
    /// If `configured_rel_dir` is provided and non-empty, checks that directory first
    /// relative to `corpus_path`. If it exists, templates are loaded from it.
    ///
    /// If `configured_rel_dir` is None (or if the configured path does not exist),
    /// probes standard candidate relative directories in order:
    /// 1. `docs/.templates`
    /// 2. `.templates`
    /// 3. `.ctxvault/templates`
    /// 4. `docs/templates`
    ///
    /// Returns a tuple of `(Option<std::path::PathBuf>, HashMap<String, Template>)` where the first
    /// element is the resolved relative directory (relative to `corpus_path`) if found,
    /// and the second is the map of loaded templates.
    pub fn discover_and_load(
        corpus_path: &Path,
        configured_rel_dir: Option<&str>,
    ) -> Result<(Option<std::path::PathBuf>, HashMap<String, Template>)> {
        // 1. Try explicitly configured relative path if provided
        if let Some(rel_dir) = configured_rel_dir {
            let trimmed = rel_dir.trim();
            if !trimmed.is_empty() {
                let candidate = corpus_path.join(trimmed);
                if candidate.is_dir() {
                    let map = Self::load_from_dir(&candidate)?;
                    return Ok((Some(std::path::PathBuf::from(trimmed)), map));
                }
            }
        }

        // 2. Probe candidate directories in priority order
        for candidate_rel in Self::CANDIDATE_TEMPLATE_DIRS {
            let candidate_full = corpus_path.join(candidate_rel);
            if candidate_full.is_dir() {
                let map = Self::load_from_dir(&candidate_full)?;
                return Ok((Some(std::path::PathBuf::from(*candidate_rel)), map));
            }
        }

        Ok((None, HashMap::new()))
    }

    /// Load all templates from a directory (reads `.md` files).
    ///
    /// If the directory does not exist, returns an empty map rather than an error.
    pub fn load_from_dir(dir: &Path) -> Result<HashMap<String, Template>> {
        let mut templates = HashMap::new();

        if !dir.exists() {
            return Ok(templates);
        }

        let entries = std::fs::read_dir(dir).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("cannot read templates directory {}: {}", dir.display(), e),
            ))
        })?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let content = std::fs::read_to_string(&path).map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("cannot read template file {}: {}", path.display(), e),
                ))
            })?;

            let template = Self::parse_markdown_template(&path, &content)?;
            let _ = templates.insert(template.name.clone(), template);
        }

        Ok(templates)
    }

    /// Parse a single `.templates/<name>.md` file.
    pub fn parse_markdown_template(path: &Path, content: &str) -> Result<Self> {
        let content_clean = content.trim_start_matches('\u{feff}');
        if !content_clean.starts_with("---") {
            return Err(Error::Config(format!(
                "template file {} is missing YAML frontmatter ('---')",
                path.display()
            )));
        }

        let after_opening = &content_clean[3..];
        let end_pos = after_opening.find("\n---").ok_or_else(|| {
            Error::Config(format!("template file {} has unclosed YAML frontmatter", path.display()))
        })?;

        let yaml_str = after_opening[..end_pos].trim();
        let scaffold =
            after_opening[end_pos + 4..].trim_start_matches(|c| c == '\r' || c == '\n').to_string();

        let parsed: TemplateFrontmatterYaml = serde_yaml::from_str(yaml_str).map_err(|e| {
            Error::Config(format!("invalid YAML frontmatter in template {}: {}", path.display(), e))
        })?;

        let default_name =
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("unnamed").to_string();

        let (name, description, target_dir) = match parsed.template {
            Some(header) => (
                if header.name.trim().is_empty() { default_name } else { header.name },
                header.description,
                header.target_dir,
            ),
            None => (default_name, None, None),
        };

        let mut required_fields = Vec::new();
        let mut optional_fields = Vec::new();

        if let Some(schema) = parsed.schema {
            if let Some(fields) = schema.fields {
                match fields {
                    FieldsConfig::Map(map) => {
                        for (field_name, def) in map {
                            let schema_entry = FieldSchema {
                                name: field_name,
                                field_type: def.field_type,
                                values: def.values,
                            };
                            if def.required {
                                required_fields.push(schema_entry);
                            } else {
                                optional_fields.push(schema_entry);
                            }
                        }
                    }
                    FieldsConfig::List(list) => {
                        for def in list {
                            let schema_entry = FieldSchema {
                                name: def.name,
                                field_type: def.field_type,
                                values: def.values,
                            };
                            if def.required {
                                required_fields.push(schema_entry);
                            } else {
                                optional_fields.push(schema_entry);
                            }
                        }
                    }
                }
            }

            let required_sections = match schema.sections {
                Some(SectionsConfig::List(list)) => list,
                Some(SectionsConfig::Detailed(d)) => d.required,
                None => Vec::new(),
            };

            let edges = schema.edges.unwrap_or_default();
            let min_word_count = schema.min_words;

            let source_path = Some(path.to_string_lossy().replace('\\', "/"));

            Ok(Template {
                name,
                description,
                target_dir,
                required_fields,
                optional_fields,
                edges,
                required_sections,
                min_word_count,
                scaffold,
                source_path,
            })
        } else {
            let source_path = Some(path.to_string_lossy().replace('\\', "/"));

            Ok(Template {
                name,
                description,
                target_dir,
                required_fields: Vec::new(),
                optional_fields: Vec::new(),
                edges: Vec::new(),
                required_sections: Vec::new(),
                min_word_count: None,
                scaffold,
                source_path,
            })
        }
    }

    /// Validate a document's frontmatter and content against this template.
    ///
    /// Returns a list of issues found (empty list means valid).
    pub fn validate(&self, frontmatter: &Option<Value>, content: &str) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        // Extract frontmatter object (or treat as empty).
        let fm_obj = frontmatter.as_ref().and_then(|v| v.as_object());

        // 1. Check required fields exist and validate types.
        for field in &self.required_fields {
            let value = fm_obj.and_then(|obj| obj.get(&field.name));

            match value {
                None => {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("missing required field '{}'", field.name),
                        field: Some(field.name.clone()),
                    });
                }
                Some(val) => {
                    self.validate_field_type(field, val, &mut issues);
                }
            }
        }

        // 2. Validate optional fields that are present.
        for field in &self.optional_fields {
            if let Some(val) = fm_obj.and_then(|obj| obj.get(&field.name)) {
                self.validate_field_type(field, val, &mut issues);
            }
        }

        // 3. Check declared edges (presence if required + type safety).
        for edge in &self.edges {
            let value = fm_obj.and_then(|obj| obj.get(&edge.field));
            match value {
                None => {
                    if edge.required {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            message: format!(
                                "missing required edge field '{}' ({})",
                                edge.field, edge.edge_type
                            ),
                            field: Some(edge.field.clone()),
                        });
                    }
                }
                Some(val) => {
                    if !val.is_string() && !val.is_array() {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            message: format!(
                                "edge field '{}' must be a string or array of strings, got {}",
                                edge.field, val
                            ),
                            field: Some(edge.field.clone()),
                        });
                    } else if let Some(arr) = val.as_array() {
                        for item in arr {
                            if !item.is_string() {
                                issues.push(ValidationIssue {
                                    severity: Severity::Error,
                                    message: format!(
                                        "edge field '{}' array items must be strings",
                                        edge.field
                                    ),
                                    field: Some(edge.field.clone()),
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 4. Check required sections exist as headings.
        for section in &self.required_sections {
            if !has_section(content, section) {
                issues.push(ValidationIssue {
                    severity: Severity::Error,
                    message: format!("missing required section '{}'", section),
                    field: None,
                });
            }
        }

        // 5. Check minimum word count.
        if let Some(min_words) = self.min_word_count {
            let word_count = content.split_whitespace().count();
            if word_count < min_words {
                issues.push(ValidationIssue {
                    severity: Severity::Warning,
                    message: format!("content has {} words, minimum is {}", word_count, min_words),
                    field: None,
                });
            }
        }

        issues
    }

    /// Validate topological graph relationships for edges declared in this template.
    ///
    /// Checks that:
    /// - Document target note exists and matches `target_template` (if configured).
    /// - Code target symbol exists in the symbol catalog (if `target_kind == "code_symbol"`).
    pub fn validate_edge_targets(
        &self,
        frontmatter: &Option<Value>,
        note_templates: &HashMap<String, Option<String>>,
        symbol_catalog_has: impl Fn(&str) -> bool,
    ) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let Some(fm_obj) = frontmatter.as_ref().and_then(|v| v.as_object()) else {
            return issues;
        };

        for edge in &self.edges {
            let Some(val) = fm_obj.get(&edge.field) else {
                continue;
            };

            let mut targets = Vec::new();
            if let Some(s) = val.as_str() {
                targets.push(s.to_string());
            } else if let Some(arr) = val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        targets.push(s.to_string());
                    }
                }
            }

            for target in targets {
                let target = target.trim();
                if target.is_empty() {
                    continue;
                }

                if edge.target_kind.as_deref() == Some("code_symbol") {
                    if !symbol_catalog_has(target) {
                        issues.push(ValidationIssue {
                            severity: Severity::Warning,
                            message: format!(
                                "edge '{}' targets code symbol '{}', but symbol was not found in catalog",
                                edge.field, target
                            ),
                            field: Some(edge.field.clone()),
                        });
                    }
                    continue;
                }

                if !note_templates.is_empty() {
                    let clean_target = target
                        .trim_start_matches('/')
                        .trim_start_matches('\\')
                        .trim_end_matches(".md");

                    let found = note_templates.iter().find(|(path, _)| {
                        let clean_p = path
                            .trim_start_matches('/')
                            .trim_start_matches('\\')
                            .trim_end_matches(".md")
                            .replace('\\', "/");
                        clean_p == clean_target.replace('\\', "/")
                    });

                    match found {
                        None => {
                            issues.push(ValidationIssue {
                                severity: Severity::Warning,
                                message: format!(
                                    "edge '{}' targets '{}', but target note does not exist",
                                    edge.field, target
                                ),
                                field: Some(edge.field.clone()),
                            });
                        }
                        Some((_, actual_template)) => {
                            if let Some(ref expected_template) = edge.target_template {
                                match actual_template {
                                    Some(actual) if actual == expected_template => {}
                                    Some(actual) => {
                                        issues.push(ValidationIssue {
                                            severity: Severity::Error,
                                            message: format!(
                                                "edge '{}' targets note '{}' with template '{}', expected template '{}'",
                                                edge.field, target, actual, expected_template
                                            ),
                                            field: Some(edge.field.clone()),
                                        });
                                    }
                                    None => {
                                        issues.push(ValidationIssue {
                                            severity: Severity::Error,
                                            message: format!(
                                                "edge '{}' targets note '{}' with no template, expected template '{}'",
                                                edge.field, target, expected_template
                                            ),
                                            field: Some(edge.field.clone()),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        issues
    }

    /// Validate a single field value against its schema.
    fn validate_field_type(
        &self,
        field: &FieldSchema,
        value: &Value,
        issues: &mut Vec<ValidationIssue>,
    ) {
        match field.field_type {
            FieldType::String | FieldType::Path => {
                if !value.is_string() {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be a string", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
            FieldType::Date => {
                if let Some(s) = value.as_str() {
                    if !is_date_like(s) {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            message: format!(
                                "field '{}' must be a date (YYYY-MM-DD), got '{}'",
                                field.name, s
                            ),
                            field: Some(field.name.clone()),
                        });
                    }
                } else {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be a date string", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
            FieldType::Enum => {
                if let Some(s) = value.as_str() {
                    if let Some(allowed) = &field.values {
                        if !allowed.contains(&s.to_string()) {
                            issues.push(ValidationIssue {
                                severity: Severity::Error,
                                message: format!(
                                    "field '{}' has invalid value '{}', allowed: {:?}",
                                    field.name, s, allowed
                                ),
                                field: Some(field.name.clone()),
                            });
                        }
                    }
                } else {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be a string (enum)", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
            FieldType::List | FieldType::ListOfPaths => {
                if !value.is_array() {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be an array", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
            FieldType::Number => {
                if !value.is_number() {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be a number", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
            FieldType::Boolean => {
                if !value.is_boolean() {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        message: format!("field '{}' must be a boolean", field.name),
                        field: Some(field.name.clone()),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a string looks like a YYYY-MM-DD date.
fn is_date_like(s: &str) -> bool {
    if s.len() < 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

/// Check if content contains a heading matching the given section name.
/// Matches `# Section Name`, `## Section Name`, etc.
fn has_section(content: &str, section: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim();
            if heading.eq_ignore_ascii_case(section) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::config::{EdgeClass, EdgeDirection};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

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
        let missing = issues
            .iter()
            .find(|i| i.field.as_deref() == Some("date") && i.severity == Severity::Error);
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
        let _ = note_templates
            .insert("docs/architecture/adr/001.md".to_string(), Some("adr".to_string()));

        let issues = tmpl.validate_edge_targets(&Some(frontmatter), &note_templates, |sym| {
            sym == "SearchService"
        });
        assert!(
            issues.is_empty(),
            "Should have no issues when target note and symbol exist: {:?}",
            issues
        );

        // Test mismatched template
        let mut mismatched = HashMap::new();
        let _ =
            mismatched.insert("docs/architecture/adr/001.md".to_string(), Some("rfc".to_string()));
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
}
