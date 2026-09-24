//! Template data model, schema types, and serialization structs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use groundcontrol_common::config::TemplateEdgeSchema;

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
    /// Must be fixed before note is considered valid.
    Error,
    /// Advisory / style recommendation.
    Warning,
    /// Informational notice.
    Info,
}

/// Overall result of validating a document against a template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationResult {
    /// Document path that was validated.
    pub path: String,
    /// Name of the template used (if any).
    pub template: Option<String>,
    /// Whether the document passed validation without errors.
    pub valid: bool,
    /// List of issues found during validation.
    pub issues: Vec<ValidationIssue>,
}

// ---------------------------------------------------------------------------
// Internal YAML Schema Deserialization Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct TemplateFrontmatterYaml {
    #[serde(default)]
    pub(super) template: Option<TemplateHeaderYaml>,
    #[serde(default)]
    pub(super) schema: Option<TemplateSchemaYaml>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct TemplateHeaderYaml {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) target_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct TemplateSchemaYaml {
    #[serde(default)]
    pub(super) fields: Option<FieldsConfig>,
    #[serde(default)]
    pub(super) edges: Option<Vec<TemplateEdgeSchema>>,
    #[serde(default)]
    pub(super) sections: Option<SectionsConfig>,
    #[serde(default)]
    pub(super) min_words: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum FieldsConfig {
    Map(BTreeMap<String, FieldDefYaml>),
    List(Vec<FieldDefListYaml>),
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FieldDefYaml {
    #[serde(rename = "type")]
    pub(super) field_type: FieldType,
    #[serde(default)]
    pub(super) required: bool,
    #[serde(default)]
    pub(super) values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FieldDefListYaml {
    pub(super) name: String,
    #[serde(rename = "type")]
    pub(super) field_type: FieldType,
    #[serde(default)]
    pub(super) required: bool,
    #[serde(default)]
    pub(super) values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum SectionsConfig {
    List(Vec<String>),
    Detailed(SectionRulesYaml),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct SectionRulesYaml {
    #[serde(default)]
    pub(super) required: Vec<String>,
}
