//! Unsupervised Reflective Random Indexing (RRI) with sliding-window co-occurrence.
//!
//! Implements code-agnostic synonym bridging derived directly from code co-occurrence
//! within functions and AST scopes, matching the mathematical foundations of codebase-memory-mcp
//! without any external models or hardcoded domain dictionaries.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Dimensionality of the RRI context space (matches the 64-bit Channel 1 bitfield).
pub const RRI_DIM: usize = 64;

/// Sliding-window half-width for co-occurrence accumulation.
pub const COOCCUR_WINDOW: usize = 5;

/// Blend weight for original base vector (70%).
pub const RRI_BETA: f32 = 0.7;

/// Blend weight for learned co-occurrence context (30%).
pub const RRI_ALPHA: f32 = 0.3;

/// Generate a deterministic, pseudo-random ternary sparse base vector in {-1, 0, 1}^64
/// seeded by Blake3 hash of the token text. Exactly 8 entries are +1 and 8 entries are -1.
pub fn generate_base_vector(token: &str) -> [f32; RRI_DIM] {
    let mut vec = [0.0f32; RRI_DIM];
    let hash = blake3::hash(format!("gc_rri_base_{token}").as_bytes());
    let bytes = hash.as_bytes();
    let mut h = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let mut count_pos = 0;
    while count_pos < 8 {
        let idx = (h % (RRI_DIM as u64)) as usize;
        if vec[idx] == 0.0 {
            vec[idx] = 1.0;
            count_pos += 1;
        }
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }

    let mut count_neg = 0;
    while count_neg < 8 {
        let idx = (h % (RRI_DIM as u64)) as usize;
        if vec[idx] == 0.0 {
            vec[idx] = -1.0;
            count_neg += 1;
        }
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }

    vec
}

/// Dynamic, unsupervised in-memory co-occurrence substrate learned from the repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RriEngine {
    /// Global document frequency per token for TF-IDF damping.
    pub doc_freqs: HashMap<String, u32>,
    /// Total number of indexed documents/chunks.
    pub total_docs: u32,
    /// Accumulated co-occurrence context vector per token.
    pub context_vectors: HashMap<String, Vec<f32>>,
}

impl Default for RriEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RriEngine {
    /// Create a new, empty RRI engine.
    pub fn new() -> Self {
        Self { doc_freqs: HashMap::new(), total_docs: 0, context_vectors: HashMap::new() }
    }

    /// Clear all learned statistics.
    pub fn clear(&mut self) {
        self.doc_freqs.clear();
        self.total_docs = 0;
        self.context_vectors.clear();
    }

    /// Calculate unsupervised Inverse Document Frequency (IDF) for a token.
    pub fn idf(&self, token: &str) -> f32 {
        if self.total_docs == 0 {
            return 1.0;
        }
        let df = self.doc_freqs.get(token).copied().unwrap_or(0);
        // Smoothed IDF: ln(1 + (N + 1) / (DF + 1))
        (1.0 + (self.total_docs as f32 + 1.0) / (df as f32 + 1.0)).ln()
    }

    /// Learn within-chunk token co-occurrences using a sliding window.
    pub fn train_chunk(&mut self, tokens: &[String]) {
        if tokens.is_empty() {
            return;
        }
        self.total_docs += 1;

        // 1. Update document frequencies (unique per chunk)
        let mut seen = std::collections::HashSet::new();
        for t in tokens {
            if seen.insert(t.as_str()) {
                *self.doc_freqs.entry(t.clone()).or_insert(0) += 1;
            }
        }

        // 2. Sliding-window co-occurrence accumulation
        let len = tokens.len();
        for i in 0..len {
            let target = &tokens[i];
            let start = i.saturating_sub(COOCCUR_WINDOW);
            let end = (i + COOCCUR_WINDOW + 1).min(len);

            let mut context_acc = [0.0f32; RRI_DIM];
            let mut neighbors = 0;

            for j in start..end {
                if j == i {
                    continue;
                }
                let neighbor = &tokens[j];
                let n_base = generate_base_vector(neighbor);
                let weight = self.idf(neighbor);
                for k in 0..RRI_DIM {
                    context_acc[k] += n_base[k] * weight;
                }
                neighbors += 1;
            }

            if neighbors > 0 {
                let ctx = self
                    .context_vectors
                    .entry(target.clone())
                    .or_insert_with(|| vec![0.0f32; RRI_DIM]);
                for k in 0..RRI_DIM {
                    ctx[k] += context_acc[k];
                }
            }
        }
    }

    /// Get the enriched semantic vector for a token, blending base and learned context.
    pub fn get_enriched_vector(&self, token: &str) -> [f32; RRI_DIM] {
        let base = generate_base_vector(token);
        if let Some(ctx) = self.context_vectors.get(token) {
            // Compute context magnitude
            let mut mag_sq = 0.0f32;
            for &v in ctx {
                mag_sq += v * v;
            }
            if mag_sq > 1e-6 {
                let inv_mag = 1.0 / mag_sq.sqrt();
                let mut enriched = [0.0f32; RRI_DIM];
                for k in 0..RRI_DIM {
                    if k < ctx.len() {
                        enriched[k] = RRI_BETA * base[k] + RRI_ALPHA * (ctx[k] * inv_mag);
                    } else {
                        enriched[k] = base[k];
                    }
                }
                return enriched;
            }
        }
        base
    }

    /// Project a list of tokens into a 64-dimensional semantic dense representation.
    pub fn project_sequence(&self, tokens: &[String]) -> [f32; RRI_DIM] {
        let mut sum = [0.0f32; RRI_DIM];
        if tokens.is_empty() {
            return sum;
        }

        // Collect unique tokens with their IDF weights
        let mut unique_tokens: Vec<(&String, f32)> = Vec::with_capacity(tokens.len());
        let mut seen = std::collections::HashSet::new();
        for t in tokens {
            if seen.insert(t.as_str()) {
                let idf = self.idf(t);
                if idf > 0.05 {
                    unique_tokens.push((t, idf));
                }
            }
        }

        // Sort descending by IDF salience and retain top 32
        unique_tokens.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        unique_tokens.truncate(32);

        for (token, weight) in unique_tokens {
            let vec = self.get_enriched_vector(token);
            for k in 0..RRI_DIM {
                sum[k] += vec[k] * weight;
            }
        }

        // L2 unit normalize
        let mut norm_sq = 0.0f32;
        for &v in &sum {
            norm_sq += v * v;
        }
        if norm_sq > 1e-6 {
            let inv_norm = 1.0 / norm_sq.sqrt();
            for v in &mut sum {
                *v *= inv_norm;
            }
        }

        sum
    }
}

/// Compute cosine similarity between two 64-dimensional dense vectors.
pub fn cosine_similarity(a: &[f32; RRI_DIM], b: &[f32; RRI_DIM]) -> f32 {
    let mut dot = 0.0f32;
    let mut mag_a = 0.0f32;
    let mut mag_b = 0.0f32;

    for i in 0..RRI_DIM {
        dot += a[i] * b[i];
        mag_a += a[i] * a[i];
        mag_b += b[i] * b[i];
    }

    let denom = mag_a.sqrt() * mag_b.sqrt();
    if denom < 1e-7 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_vector_determinism_and_sparsity() {
        let v1 = generate_base_vector("shipping");
        let v2 = generate_base_vector("shipping");
        assert_eq!(v1, v2);

        let non_zeros = v1.iter().filter(|&&x| x != 0.0).count();
        assert_eq!(non_zeros, 16);
    }

    #[test]
    fn test_rri_cooccurrence_bridges_synonyms() {
        let mut rri = RriEngine::new();

        // Train co-occurrence: "quote" and "shipping" appear together in multiple functions
        for _ in 0..10 {
            let doc = vec![
                "fn".to_string(),
                "get".to_string(),
                "shipping".to_string(),
                "quote".to_string(),
                "rate".to_string(),
            ];
            rri.train_chunk(&doc);
        }

        // Unrelated document: "database" and "connection"
        for _ in 0..10 {
            let doc = vec![
                "db".to_string(),
                "connect".to_string(),
                "pool".to_string(),
                "sql".to_string(),
            ];
            rri.train_chunk(&doc);
        }

        let vec_quote = rri.get_enriched_vector("quote");
        let vec_shipping = rri.get_enriched_vector("shipping");
        let vec_sql = rri.get_enriched_vector("sql");

        let sim_related = cosine_similarity(&vec_quote, &vec_shipping);
        let sim_unrelated = cosine_similarity(&vec_quote, &vec_sql);

        assert!(
            sim_related > sim_unrelated,
            "Related terms via co-occurrence ({sim_related:.3}) must have higher similarity than unrelated ({sim_unrelated:.3})"
        );
    }
}
