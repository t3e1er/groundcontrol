//! Matryoshka-aligned SIF projector and PCA first principal component removal.
//!
//! Implements Pillar 1: Pure Interface-Semantic 256-bit Vector Projection
//! and Pillar 3: True Matryoshka Representation Learning (MRL) Prefix Slicing
//! where V_64 ⊂ V_128 ⊂ V_256.

use std::collections::HashMap;

use groundcontrol_common::types::{BinaryFingerprint, CodeSymbol};

use super::tokenizer::UNIVERSAL_ABBREVIATIONS;

/// Total embedding dimensions for Matryoshka binary fingerprints (4 x 64-bit words).
pub const V3_DIMENSIONS: usize = 256;

/// Default SIF smoothing parameter `a` (1e-4).
pub const SIF_SMOOTHING_PARAM: f32 = 1e-4;

/// A deterministic Matryoshka SIF projector with seeded background frequency dictionary
/// and power-iteration first principal component removal.
#[derive(Debug, Clone)]
pub struct BinaryV3Projector {
    token_frequencies: HashMap<String, f32>,
    total_token_count: f32,
    smoothing_a: f32,
    first_principal_component: Option<[f32; V3_DIMENSIONS]>,
}

impl Default for BinaryV3Projector {
    fn default() -> Self {
        Self::new(SIF_SMOOTHING_PARAM)
    }
}

impl BinaryV3Projector {
    /// Create a new projector with the given SIF smoothing parameter `a`.
    pub fn new(smoothing_a: f32) -> Self {
        let mut projector = Self {
            token_frequencies: HashMap::new(),
            total_token_count: 0.0,
            smoothing_a,
            first_principal_component: None,
        };
        projector.seed_background_vocabulary();
        projector
    }

    /// Seed the dictionary with downweighted stop words and polyglot syntax keywords.
    fn seed_background_vocabulary(&mut self) {
        let stop_words = [
            // English prose stop words
            "in",
            "to",
            "for",
            "of",
            "and",
            "the",
            "a",
            "an",
            "is",
            "at",
            "by",
            "from",
            "with",
            "on",
            "as",
            "it",
            "this",
            "that",
            "be",
            "or",
            "if",
            "let",
            "mut",
            // Polyglot programming keywords & syntax boilerplate
            "const",
            "var",
            "val",
            "def",
            "fn",
            "func",
            "function",
            "pub",
            "private",
            "public",
            "protected",
            "class",
            "struct",
            "interface",
            "type",
            "enum",
            "import",
            "export",
            "require",
            "package",
            "namespace",
            "use",
            "using",
            "return",
            "returns",
            "async",
            "await",
            "yield",
            "new",
            "throw",
            "throws",
            "try",
            "catch",
            "finally",
            "except",
            "true",
            "false",
            "null",
            "nil",
            "undefined",
            "none",
            "void",
            "self",
            "super",
            "string",
            "int",
            "bool",
            "boolean",
            "float",
            "double",
            "bytes",
            "byte",
            "any",
            "include",
            "header",
            // Common path boilerplate and file extension stop words
            "src",
            "lib",
            "internal",
            "pkg",
            "index",
            "js",
            "ts",
            "rs",
            "go",
            "py",
            "java",
            "cpp",
            "c",
            "h",
            "cs",
            "rb",
            "kt",
            "php",
        ];
        for &sw in &stop_words {
            self.token_frequencies.insert(sw.to_string(), 5000.0);
            self.total_token_count += 5000.0;
        }
    }

    /// Retrieve SIF weighting coefficient for a given token string: `a / (a + p(t))`.
    #[inline]
    fn token_sif_weight(&self, token: &str) -> f32 {
        let count = self.token_frequencies.get(token).copied().unwrap_or(1.0);
        let prob = count / self.total_token_count.max(1.0);
        self.smoothing_a / (self.smoothing_a + prob)
    }

    /// Retrieve deterministically synthesized 256-dimensional unit vector for any token.
    #[inline]
    fn token_vector(&self, token: &str) -> [f32; V3_DIMENSIONS] {
        deterministic_hash_vector(token)
    }

    /// Accumulate token SIF vector into running sum with salience multiplier.
    fn accumulate_token_sif(
        &self,
        sum: &mut [f32; V3_DIMENSIONS],
        total_weight: &mut f32,
        token: &str,
        multiplier: f32,
    ) {
        let lower = token.to_lowercase();
        let base_weight = self.token_sif_weight(&lower);
        let weight = base_weight * multiplier;
        let vec = self.token_vector(&lower);

        for d in 0..V3_DIMENSIONS {
            sum[d] += weight * vec[d];
        }
        *total_weight += weight;

        // Universal abbreviation expansion (mu_abbrev = 0.75 * multiplier)
        for &(abbrev, expanded) in UNIVERSAL_ABBREVIATIONS {
            if lower == abbrev {
                let exp_weight = self.token_sif_weight(expanded) * multiplier * 0.75;
                let exp_vec = self.token_vector(expanded);
                for d in 0..V3_DIMENSIONS {
                    sum[d] += exp_weight * exp_vec[d];
                }
                *total_weight += exp_weight;
                break;
            } else if lower == expanded {
                let abb_weight = self.token_sif_weight(abbrev) * multiplier * 0.75;
                let abb_vec = self.token_vector(abbrev);
                for d in 0..V3_DIMENSIONS {
                    sum[d] += abb_weight * abb_vec[d];
                }
                *total_weight += abb_weight;
                break;
            }
        }

        // Morphological stem (inherits multiplier)
        if let Some(stem) = super::tokenizer::stem_suffix(&lower) {
            if stem != lower && stem.len() >= 2 {
                let stem_weight = self.token_sif_weight(&stem) * multiplier;
                let stem_vec = self.token_vector(&stem);
                for d in 0..V3_DIMENSIONS {
                    sum[d] += stem_weight * stem_vec[d];
                }
                *total_weight += stem_weight;
            }
        }

        // Sub-tokens from camelCase and snake_case identifiers (mu_subword = 1.0)
        let sub_tokens = crate::parser::code::patterns::split_identifier(token);
        if sub_tokens.len() > 1 {
            for sub in sub_tokens {
                let sub_lower = sub.to_lowercase();
                if sub_lower != lower {
                    let sub_weight = self.token_sif_weight(&sub_lower) * 1.0;
                    let sub_vec = self.token_vector(&sub_lower);
                    for d in 0..V3_DIMENSIONS {
                        sum[d] += sub_weight * sub_vec[d];
                    }
                    *total_weight += sub_weight;

                    if let Some(sub_stem) = super::tokenizer::stem_suffix(&sub_lower) {
                        if sub_stem != sub_lower && sub_stem.len() >= 2 {
                            let stem_weight = self.token_sif_weight(&sub_stem) * 1.0;
                            let stem_vec = self.token_vector(&sub_stem);
                            for d in 0..V3_DIMENSIONS {
                                sum[d] += stem_weight * stem_vec[d];
                            }
                            *total_weight += stem_weight;
                        }
                    }

                    for &(abbrev, expanded) in UNIVERSAL_ABBREVIATIONS {
                        if sub_lower == abbrev {
                            let exp_weight = self.token_sif_weight(expanded) * 0.75;
                            let exp_vec = self.token_vector(expanded);
                            for d in 0..V3_DIMENSIONS {
                                sum[d] += exp_weight * exp_vec[d];
                            }
                            *total_weight += exp_weight;
                            break;
                        } else if sub_lower == expanded {
                            let abb_weight = self.token_sif_weight(abbrev) * 0.75;
                            let abb_vec = self.token_vector(abbrev);
                            for d in 0..V3_DIMENSIONS {
                                sum[d] += abb_weight * abb_vec[d];
                            }
                            *total_weight += abb_weight;
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Accumulate a text block into the running sum with given salience multiplier.
    pub fn accumulate_text(
        &self,
        sum: &mut [f32; V3_DIMENSIONS],
        total_weight: &mut f32,
        text: &str,
        multiplier: f32,
    ) {
        let words = text
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .filter(|w| !w.is_empty());

        for word in words {
            self.accumulate_token_sif(sum, total_weight, word, multiplier);
        }
    }

    /// Finalize continuous sum into a 256-bit Matryoshka fingerprint with optional PCA.
    pub fn finalize_projection(
        &self,
        mut sum: [f32; V3_DIMENSIONS],
        total_weight: f32,
    ) -> BinaryFingerprint {
        if total_weight > 0.0 {
            let inv_w = 1.0 / total_weight;
            for d in 0..V3_DIMENSIONS {
                sum[d] *= inv_w;
            }
        }

        // Subtract 1st principal component if computed
        if let Some(u) = self.first_principal_component {
            let mut dot = 0.0f32;
            for d in 0..V3_DIMENSIONS {
                dot += sum[d] * u[d];
            }
            for d in 0..V3_DIMENSIONS {
                sum[d] -= u[d] * dot;
            }
        }

        l2_normalize_256(&mut sum);
        BinaryFingerprint::from_f32_slice(&sum)
    }

    /// Project a text query or code content into a 256-bit Matryoshka fingerprint.
    pub fn project_text(&self, text: &str) -> BinaryFingerprint {
        let mut sum = [0.0f32; V3_DIMENSIONS];
        let mut total_weight = 0.0f32;
        self.accumulate_text(&mut sum, &mut total_weight, text, 1.0);
        self.finalize_projection(sum, total_weight)
    }

    /// Project extracted AST grammar semantics into a 256-bit binary fingerprint.
    pub fn project_semantics(
        &self,
        sem: &groundcontrol_common::types::ExtractedGrammarSemantics,
    ) -> BinaryFingerprint {
        let mut text = String::new();
        for t in &sem.interface_tokens {
            text.push_str(&t.text);
            text.push(' ');
        }
        for t in &sem.api_tokens {
            text.push_str(&t.text);
            text.push(' ');
        }
        for p in &sem.dataflow_paths {
            text.push_str(&p.source_param);
            text.push(' ');
        }
        self.project_text(&text)
    }

    /// Project a search query into a 256-bit Matryoshka fingerprint.
    pub fn project_query(&self, query: &str) -> BinaryFingerprint {
        let mut sum = [0.0f32; V3_DIMENSIONS];
        let mut total_weight = 0.0f32;
        self.accumulate_text(&mut sum, &mut total_weight, query, 1.0);
        self.finalize_projection(sum, total_weight)
    }

    /// Project a code symbol into a 256-bit Matryoshka fingerprint.
    ///
    /// Implements RFC Pillar 1 salience multipliers:
    /// - Name: 2.0x
    /// - Signature: 1.2x
    /// - Path Context: 0.8x
    /// - Docstring: 0.8x
    pub fn project_symbol(&self, sym: &CodeSymbol, doc_path: &str) -> BinaryFingerprint {
        let mut sum = [0.0f32; V3_DIMENSIONS];
        let mut total_weight = 0.0f32;

        self.accumulate_text(&mut sum, &mut total_weight, &sym.name, 2.0);
        if !doc_path.is_empty() {
            self.accumulate_text(&mut sum, &mut total_weight, doc_path, 0.8);
        }
        if !sym.signature.is_empty() {
            self.accumulate_text(&mut sum, &mut total_weight, &sym.signature, 1.2);
        }
        if let Some(ref doc) = sym.docstring {
            if !doc.is_empty() {
                self.accumulate_text(&mut sum, &mut total_weight, doc, 0.8);
            }
        }

        self.finalize_projection(sum, total_weight)
    }

    /// Project a whole document into a 256-bit Matryoshka fingerprint.
    pub fn project_document(&self, doc_path: &str, content: &str) -> BinaryFingerprint {
        let mut sum = [0.0f32; V3_DIMENSIONS];
        let mut total_weight = 0.0f32;
        if !doc_path.is_empty() {
            self.accumulate_text(&mut sum, &mut total_weight, doc_path, 1.2);
        }
        self.accumulate_text(&mut sum, &mut total_weight, content, 1.0);
        self.finalize_projection(sum, total_weight)
    }

    /// Project a document chunk into a 256-bit Matryoshka fingerprint.
    pub fn project_chunk(&self, doc_path: &str, chunk_text: &str) -> BinaryFingerprint {
        let mut sum = [0.0f32; V3_DIMENSIONS];
        let mut total_weight = 0.0f32;
        if !doc_path.is_empty() {
            self.accumulate_text(&mut sum, &mut total_weight, doc_path, 0.8);
        }
        self.accumulate_text(&mut sum, &mut total_weight, chunk_text, 1.0);
        self.finalize_projection(sum, total_weight)
    }

    /// Compute 1st principal component across candidate vectors via 5 power-iteration steps
    /// and remove it from future projections.
    pub fn compute_and_remove_first_principal_component(
        &mut self,
        vectors: &[[f32; V3_DIMENSIONS]],
    ) {
        if vectors.is_empty() {
            return;
        }

        let init_val = 1.0 / (V3_DIMENSIONS as f32).sqrt();
        let mut u = [init_val; V3_DIMENSIONS];

        for _ in 0..5 {
            let mut y = [0.0f32; V3_DIMENSIONS];
            for v in vectors {
                let mut dot = 0.0f32;
                for d in 0..V3_DIMENSIONS {
                    dot += v[d] * u[d];
                }
                for d in 0..V3_DIMENSIONS {
                    y[d] += v[d] * dot;
                }
            }
            l2_normalize_256(&mut y);
            u = y;
        }

        self.first_principal_component = Some(u);
    }
}

#[inline]
fn l2_normalize_256(vec: &mut [f32; V3_DIMENSIONS]) {
    let mut norm_sq = 0.0f32;
    for &val in vec.iter() {
        norm_sq += val * val;
    }
    if norm_sq > 1e-12 {
        let inv_norm = 1.0 / norm_sq.sqrt();
        for val in vec.iter_mut() {
            *val *= inv_norm;
        }
    }
}

/// Deterministically project any word token into 256 dimensions using Blake3 XOF bits.
fn deterministic_hash_vector(token: &str) -> [f32; V3_DIMENSIONS] {
    let mut vec = [0.0f32; V3_DIMENSIONS];
    let mut hasher = blake3::Hasher::new();
    hasher.update(token.as_bytes());
    let mut reader = hasher.finalize_xof();
    let mut bytes = [0u8; V3_DIMENSIONS];
    reader.fill(&mut bytes);

    for d in 0..V3_DIMENSIONS {
        let b = bytes[d];
        vec[d] = ((b as f32 / 127.5) - 1.0) * (if (d + b as usize) % 2 == 0 { 1.0 } else { -1.0 });
    }

    l2_normalize_256(&mut vec);
    vec
}
