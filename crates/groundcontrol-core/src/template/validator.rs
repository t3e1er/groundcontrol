//! Template validation engine checking document frontmatter and content against schema rules.

use std::collections::HashMap;

use serde_json::Value;

use super::model::{FieldSchema, FieldType, Severity, Template, ValidationIssue};

impl Template {
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
