//! Template file discovery, directory traversal, and markdown/YAML parsing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use groundcontrol_common::{Error, Result};

use super::model::{FieldSchema, FieldsConfig, SectionsConfig, Template, TemplateFrontmatterYaml};

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
    /// 3. `.groundcontrol/templates`
    /// 4. `docs/templates`
    ///
    /// Returns a tuple of `(Option<PathBuf>, HashMap<String, Template>)` where the first
    /// element is the resolved relative directory (relative to `corpus_path`) if found,
    /// and the second is the map of loaded templates.
    pub fn discover_and_load(
        corpus_path: &Path,
        configured_rel_dir: Option<&str>,
    ) -> Result<(Option<PathBuf>, HashMap<String, Template>)> {
        // 1. Try explicitly configured relative path if provided
        if let Some(rel_dir) = configured_rel_dir {
            let trimmed = rel_dir.trim();
            if !trimmed.is_empty() {
                let candidate = corpus_path.join(trimmed);
                if candidate.is_dir() {
                    let map = Self::load_from_dir(&candidate)?;
                    return Ok((Some(PathBuf::from(trimmed)), map));
                }
            }
        }

        // 2. Probe candidate directories in priority order
        for candidate_rel in Self::CANDIDATE_TEMPLATE_DIRS {
            let candidate_full = corpus_path.join(candidate_rel);
            if candidate_full.is_dir() {
                let map = Self::load_from_dir(&candidate_full)?;
                return Ok((Some(PathBuf::from(*candidate_rel)), map));
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
}
