//! Document edge builder: constructs wikilink, tag, and frontmatter edges for notes.

use std::collections::{HashMap, HashSet};

use groundcontrol_common::config::{EdgeClass, EdgeDirection, EdgeSource, EdgeTypeConfig};
use groundcontrol_common::types::{Document, EdgeProvenance};

use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// Build edges from a parsed Document based on the given edge type configurations.
    pub fn build_edges_for_document(
        &mut self,
        doc: &Document,
        edge_configs: &[EdgeTypeConfig],
        all_docs: &[Document],
    ) {
        let _ = self.add_node(&doc.path, doc.title.as_deref());

        for config in edge_configs {
            match config.source {
                EdgeSource::Wikilink => {
                    self.build_wikilink_edges(doc, config);
                }
                EdgeSource::Tag => {
                    self.build_tag_edges(doc, config, all_docs);
                }
                EdgeSource::Frontmatter => {
                    self.build_frontmatter_edges(doc, config);
                }
                EdgeSource::Reference => {}
                EdgeSource::Code => {}
            }
        }
    }

    fn build_wikilink_edges(&mut self, doc: &Document, config: &EdgeTypeConfig) {
        let class = config.class.unwrap_or_else(|| EdgeClass::infer_from_source(&config.source));
        for wikilink in &doc.links {
            self.add_edge(
                &doc.path,
                &wikilink.target,
                &config.name,
                config.weight,
                EdgeProvenance::Wikilink,
                class,
            );
            if config.bidirectional {
                self.add_edge(
                    &wikilink.target,
                    &doc.path,
                    &config.name,
                    config.weight,
                    EdgeProvenance::Wikilink,
                    class,
                );
            }
        }
    }

    /// High-performance inverted-index tag edge builder across all documents.
    pub fn build_all_tag_edges(&mut self, configs: &[EdgeTypeConfig], all_docs: &[Document]) {
        if configs.is_empty() || all_docs.is_empty() {
            return;
        }

        let n = all_docs.len() as f32;

        let mut tag_postings: HashMap<&str, Vec<&str>> = HashMap::new();
        for d in all_docs {
            for tag in &d.tags {
                tag_postings.entry(tag.as_str()).or_default().push(&d.path);
            }
        }

        for config in configs {
            let class =
                config.class.unwrap_or_else(|| EdgeClass::infer_from_source(&config.source));
            for (&_tag, paths) in &tag_postings {
                let doc_freq = paths.len();
                if let Some(max_freq) = config.max_frequency {
                    if doc_freq > max_freq {
                        continue;
                    }
                }
                let idf = (n / doc_freq as f32).ln();
                let edge_weight = config.weight * idf;
                if edge_weight <= 0.0 {
                    continue;
                }

                for (i, p1) in paths.iter().enumerate() {
                    for p2 in &paths[i + 1..] {
                        self.add_edge(
                            p1,
                            p2,
                            &config.name,
                            edge_weight,
                            EdgeProvenance::SharedTag,
                            class,
                        );
                        if config.bidirectional {
                            self.add_edge(
                                p2,
                                p1,
                                &config.name,
                                edge_weight,
                                EdgeProvenance::SharedTag,
                                class,
                            );
                        }
                    }
                }
            }
        }
    }

    fn build_tag_edges(&mut self, doc: &Document, config: &EdgeTypeConfig, all_docs: &[Document]) {
        if doc.tags.is_empty() || all_docs.is_empty() {
            return;
        }

        let class = config.class.unwrap_or_else(|| EdgeClass::infer_from_source(&config.source));
        let n = all_docs.len() as f32;

        let mut tag_frequencies: HashMap<&str, usize> = HashMap::new();
        for d in all_docs {
            for tag in &d.tags {
                *tag_frequencies.entry(tag.as_str()).or_insert(0) += 1;
            }
        }

        let doc_tags: HashSet<&str> = doc.tags.iter().map(|t| t.as_str()).collect();

        for other in all_docs {
            if other.path == doc.path {
                continue;
            }

            let mut weight_sum: f32 = 0.0;
            for tag in &other.tags {
                if !doc_tags.contains(tag.as_str()) {
                    continue;
                }
                if let Some(max_freq) = config.max_frequency {
                    if let Some(&count) = tag_frequencies.get(tag.as_str()) {
                        if count > max_freq {
                            continue;
                        }
                    }
                }
                if let Some(&doc_freq) = tag_frequencies.get(tag.as_str()) {
                    let idf = (n / doc_freq as f32).ln();
                    weight_sum += config.weight * idf;
                }
            }

            if weight_sum > 0.0 {
                self.add_edge(
                    &doc.path,
                    &other.path,
                    &config.name,
                    weight_sum,
                    EdgeProvenance::SharedTag,
                    class,
                );
            }
        }
    }

    fn build_frontmatter_edges(&mut self, doc: &Document, config: &EdgeTypeConfig) {
        let Some(ref frontmatter) = doc.frontmatter else {
            return;
        };
        let Some(ref field_name) = config.field else {
            return;
        };

        let targets = extract_frontmatter_targets(frontmatter, field_name);

        let direction = config.direction.as_ref();
        let class = config.class.unwrap_or_else(|| EdgeClass::infer_from_source(&config.source));

        for target in targets {
            match direction {
                Some(EdgeDirection::Inbound) => {
                    self.add_edge(
                        &target,
                        &doc.path,
                        &config.name,
                        config.weight,
                        EdgeProvenance::Frontmatter,
                        class,
                    );
                }
                _ => {
                    self.add_edge(
                        &doc.path,
                        &target,
                        &config.name,
                        config.weight,
                        EdgeProvenance::Frontmatter,
                        class,
                    );
                }
            }
            if config.bidirectional {
                match direction {
                    Some(EdgeDirection::Inbound) => {
                        self.add_edge(
                            &doc.path,
                            &target,
                            &config.name,
                            config.weight,
                            EdgeProvenance::Frontmatter,
                            class,
                        );
                    }
                    _ => {
                        self.add_edge(
                            &target,
                            &doc.path,
                            &config.name,
                            config.weight,
                            EdgeProvenance::Frontmatter,
                            class,
                        );
                    }
                }
            }
        }
    }
}

/// Extract target paths from a frontmatter field value.
fn extract_frontmatter_targets(frontmatter: &serde_json::Value, field: &str) -> Vec<String> {
    match frontmatter.get(field) {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }
        _ => Vec::new(),
    }
}
