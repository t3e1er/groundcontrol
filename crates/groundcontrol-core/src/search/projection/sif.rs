//! Arora et al. Smooth Inverse Frequency (SIF) code & doc projection engine.
//!
//! Projects arbitrary natural-language queries, doc passages, and code symbols into
//! 256-dimensional Matryoshka semantic space in linear CPU time without transformer
//! forward passes:
//!
//!   v_s = (1 / |T_s|) * \sum_{w \in T_s} (a / (a + p(w))) * v_w
//!
//! Followed by 5-iteration power method first principal component removal:
//!   v'_s = v_s - u(u^T v_s)

use std::collections::HashMap;

use groundcontrol_common::types::{BinaryFingerprint, ExtractedGrammarSemantics};

use super::BinaryProjector;

/// Default SIF smoothing parameter `a` (1e-4).
pub const SIF_SMOOTHING_PARAM: f32 = 1e-4;

/// Target dimension for Matryoshka binary embeddings.
pub const SIF_DIMENSIONS: usize = 256;

/// A deterministic static token embedding table with SIF smoothing and PCA removal.
#[derive(Debug, Clone)]
pub struct SifEngine {
    token_weights: HashMap<String, [f32; SIF_DIMENSIONS]>,
    token_frequencies: HashMap<String, f32>,
    total_token_count: f32,
    smoothing_a: f32,
    first_principal_component: Option<[f32; SIF_DIMENSIONS]>,
}

impl Default for SifEngine {
    fn default() -> Self {
        Self::new(SIF_SMOOTHING_PARAM)
    }
}

impl SifEngine {
    /// Create a new SIF engine with the given smoothing scalar parameter `a`.
    pub fn new(smoothing_a: f32) -> Self {
        let mut engine = Self {
            token_weights: HashMap::new(),
            token_frequencies: HashMap::new(),
            total_token_count: 0.0,
            smoothing_a,
            first_principal_component: None,
        };
        engine.seed_common_vocabulary();
        engine
    }

    /// Seed the dictionary with canonical semantic vectors for common code & doc tokens.
    fn seed_common_vocabulary(&mut self) {
        // Common syntax and stop words receive high frequency so SIF downweights them
        let stop_words = [
            "in", "to", "for", "of", "and", "the", "a", "an", "is", "at", "by", "from", "with",
            "on", "as", "it", "this", "that", "be", "or", "if", "let", "mut",
        ];
        for &sw in &stop_words {
            self.token_frequencies.insert(sw.to_string(), 5000.0);
            self.total_token_count += 5000.0;
        }

        // Essential keywords across programming languages & documentation
        let seed_tokens = [
            ("error", 0),
            ("exception", 0),
            ("panic", 0),
            ("fail", 0),
            ("fault", 0),
            ("failure", 0),
            ("auth", 1),
            ("authentication", 1),
            ("token", 1),
            ("security", 1),
            ("jwt", 1),
            ("validate", 1),
            ("validation", 1),
            ("route", 2),
            ("endpoint", 2),
            ("api", 2),
            ("get", 2),
            ("post", 2),
            ("http", 2),
            ("storage", 3),
            ("database", 3),
            ("file", 3),
            ("disk", 3),
            ("read", 3),
            ("write", 3),
            ("open", 4),
            ("close", 4),
            ("flush", 4),
            ("sync", 4),
            ("commit", 4),
            ("network", 5),
            ("socket", 5),
            ("connection", 5),
            ("tcp", 5),
            ("rpc", 5),
            ("test", 6),
            ("assert", 6),
            ("mock", 6),
            ("fixture", 6),
            ("config", 7),
            ("settings", 7),
            ("environment", 7),
            ("options", 7),
            ("volume", 8),
            ("pvc", 8),
            ("claim", 8),
            ("kubernetes", 8),
            ("controller", 8),
            ("reconcile", 9),
            ("manager", 9),
            ("handler", 9),
            ("dispatch", 9),
            ("document", 10),
            ("markdown", 10),
            ("heading", 10),
            ("note", 10),
            ("adr", 10),
            ("search", 11),
            ("query", 11),
            ("index", 11),
            ("ranking", 11),
            ("vector", 11),
        ];

        for &(token, cluster_id) in &seed_tokens {
            let vec = generate_cluster_vector(token, cluster_id);
            self.token_weights.insert(token.to_string(), vec);
            self.token_frequencies.insert(token.to_string(), 10.0);
            self.total_token_count += 10.0;
        }
    }

    /// Retrieve the SIF weighting coefficient for a given token string: `a / (a + p(w))`.
    fn token_sif_weight(&self, token: &str) -> f32 {
        let count = self.token_frequencies.get(token).copied().unwrap_or(1.0);
        let prob = count / self.total_token_count.max(1.0);
        self.smoothing_a / (self.smoothing_a + prob)
    }

    /// Retrieve or deterministically synthesize a 256-dimensional unit vector for any token.
    fn token_vector(&self, token: &str) -> [f32; SIF_DIMENSIONS] {
        if let Some(vec) = self.token_weights.get(token) {
            *vec
        } else {
            deterministic_hash_vector(token)
        }
    }

    /// Project a text string into a 256-dimensional SIF vector.
    pub fn project_text(&self, text: &str) -> [f32; SIF_DIMENSIONS] {
        let words: Vec<&str> = text
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .filter(|w| !w.is_empty())
            .collect();

        if words.is_empty() {
            return [0.0; SIF_DIMENSIONS];
        }

        let mut sum = [0.0f32; SIF_DIMENSIONS];
        let mut total_weight = 0.0f32;

        for word in &words {
            let lower = word.to_lowercase();
            let weight = self.token_sif_weight(&lower);
            let vec = self.token_vector(&lower);

            for d in 0..SIF_DIMENSIONS {
                sum[d] += weight * vec[d];
            }
            total_weight += weight;

            // Incorporate sub-tokens from camelCase and snake_case identifiers
            let sub_tokens = crate::parser::code::patterns::split_identifier(word);
            if sub_tokens.len() > 1 {
                for sub in sub_tokens {
                    let sub_lower = sub.to_lowercase();
                    if sub_lower != lower {
                        let sub_weight = self.token_sif_weight(&sub_lower);
                        let sub_vec = self.token_vector(&sub_lower);
                        for d in 0..SIF_DIMENSIONS {
                            sum[d] += sub_weight * sub_vec[d];
                        }
                        total_weight += sub_weight;
                    }
                }
            }
        }

        if total_weight > 0.0 {
            for d in 0..SIF_DIMENSIONS {
                sum[d] /= total_weight;
            }
        }

        // Subtract 1st principal component if computed
        if let Some(u) = self.first_principal_component {
            let mut dot = 0.0f32;
            for d in 0..SIF_DIMENSIONS {
                dot += sum[d] * u[d];
            }
            for d in 0..SIF_DIMENSIONS {
                sum[d] -= u[d] * dot;
            }
        }

        // L2 normalize
        l2_normalize(&mut sum);
        sum
    }

    /// Project a text string into a 256-bit binary fingerprint.
    pub fn project_to_fingerprint(&self, text: &str) -> BinaryFingerprint {
        let vec = self.project_text(text);
        BinaryFingerprint::from_f32_slice(&vec)
    }

    /// Compute the 1st principal component across a collection of candidate vectors
    /// using 5 iterations of the power method, and remove it from future projections.
    pub fn compute_and_remove_first_principal_component(
        &mut self,
        vectors: &[[f32; SIF_DIMENSIONS]],
    ) {
        if vectors.is_empty() {
            return;
        }

        // Initialize u0 to unit uniform vector
        let init_val = 1.0 / (SIF_DIMENSIONS as f32).sqrt();
        let mut u = [init_val; SIF_DIMENSIONS];

        // 5 power-iteration steps
        for _ in 0..5 {
            let mut y = [0.0f32; SIF_DIMENSIONS];
            for v in vectors {
                let mut dot = 0.0f32;
                for d in 0..SIF_DIMENSIONS {
                    dot += v[d] * u[d];
                }
                for d in 0..SIF_DIMENSIONS {
                    y[d] += v[d] * dot;
                }
            }
            l2_normalize(&mut y);
            u = y;
        }

        self.first_principal_component = Some(u);
    }
}

impl BinaryProjector for SifEngine {
    fn project_query(&self, text: &str) -> BinaryFingerprint {
        self.project_to_fingerprint(text)
    }

    fn project_semantics(&self, sem: &ExtractedGrammarSemantics) -> BinaryFingerprint {
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
        self.project_to_fingerprint(&text)
    }
}

/// Helper to L2-normalize a float slice in place.
#[inline]
pub fn l2_normalize(vec: &mut [f32; SIF_DIMENSIONS]) {
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

/// Generate a structured cluster vector for a semantic category.
fn generate_cluster_vector(word: &str, cluster_id: usize) -> [f32; SIF_DIMENSIONS] {
    let mut cluster_base = [0.0f32; SIF_DIMENSIONS];
    let cluster_hash = blake3::hash(format!("cluster_{cluster_id}").as_bytes());
    let cluster_bytes = cluster_hash.as_bytes();
    for d in 0..SIF_DIMENSIONS {
        let b = cluster_bytes[(d * 5) % 32];
        cluster_base[d] = ((b as f32 / 127.5) - 1.0) * 3.0;
    }

    let word_vec = deterministic_hash_vector(word);
    for d in 0..SIF_DIMENSIONS {
        cluster_base[d] += word_vec[d];
    }
    l2_normalize(&mut cluster_base);
    cluster_base
}

/// Deterministically project any word token into 256 dimensions using Blake3 hash bits.
fn deterministic_hash_vector(token: &str) -> [f32; SIF_DIMENSIONS] {
    let mut vec = [0.0f32; SIF_DIMENSIONS];
    let hash = blake3::hash(token.as_bytes());
    let bytes = hash.as_bytes();

    for d in 0..SIF_DIMENSIONS {
        let byte_idx = (d * 7) % 32;
        let b = bytes[byte_idx];
        // Centered around 0.0 in [-1.0, 1.0]
        vec[d] = ((b as f32 / 127.5) - 1.0) * (if (d + b as usize) % 2 == 0 { 1.0 } else { -1.0 });
    }

    l2_normalize(&mut vec);
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sif_projection_and_fingerprint() {
        let engine = SifEngine::default();
        let fp1 = engine.project_to_fingerprint("handle authentication error in token validation");
        let fp2 = engine.project_to_fingerprint("validate jwt auth token failure");
        let fp_unrelated = engine.project_to_fingerprint("css flexbox layout alignment margin");

        let dist_related = fp1.hamming_distance(&fp2);
        let dist_unrelated = fp1.hamming_distance(&fp_unrelated);

        // Related concepts should be closer in Hamming space
        assert!(
            dist_related < dist_unrelated,
            "expected related dist {dist_related} < unrelated dist {dist_unrelated}"
        );
    }

    #[test]
    fn test_first_principal_component_removal() {
        let mut engine = SifEngine::default();
        let texts = [
            "fn open_file(path: &str) -> Result<File>",
            "fn read_file(path: &str) -> Result<Vec<u8>>",
            "fn write_file(path: &str, data: &[u8]) -> Result<()>",
        ];

        let vectors: Vec<[f32; SIF_DIMENSIONS]> =
            texts.iter().map(|t| engine.project_text(t)).collect();
        engine.compute_and_remove_first_principal_component(&vectors);

        let v_after = engine.project_text("fn flush_file(path: &str) -> Result<()>");
        let u = engine.first_principal_component.unwrap();

        // Dot product with u should be approximately zero after subtraction
        let mut dot = 0.0f32;
        for d in 0..SIF_DIMENSIONS {
            dot += v_after[d] * u[d];
        }
        assert!(dot.abs() < 1e-4, "expected dot product near 0, got {dot}");
    }
}
