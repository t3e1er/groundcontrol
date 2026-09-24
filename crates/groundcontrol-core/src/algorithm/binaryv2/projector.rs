//! Multi-channel semantic binary projector for binaryv2 retrieval.
//!
//! Maps documents, code symbols, and search queries into 256-bit binary fingerprints
//! across 4 active, complementary 64-bit channels:
//! - Channel 0: Core Lexical & Symbol Interface (Bits 0..63)
//! - Channel 1: Unsupervised RRI Co-occurrence Context (Bits 64..127)
//! - Channel 2: AST API Calls & Interface Topology (Bits 128..191)
//! - Channel 3: Hierarchical Path & Structural Scope (Bits 192..255)
//!
//! All channels are active for both queries and documents, derived purely from
//! tree-sitter AST symbols, grammar semantics, and corpus statistics.

use groundcontrol_common::types::{BinaryFingerprint, CodeSymbol};

use super::rri::{RriEngine, RRI_DIM};
use super::tokenizer::{expand_tokens_morphology, tokenize_code_text};
use crate::parser::code::grammar::ExtractedGrammarSemantics;

/// Dimension of intermediate float vectors for each 64-bit channel.
pub const CHANNEL_DIMENSIONS: usize = 64;

/// Multi-channel semantic hyperplane projector.
#[derive(Debug, Clone)]
pub struct BinaryV2Projector {
    /// 4 channels, each having 64 hyperplanes of 64 dimensions: `[4][64][64]`
    hyperplanes: Box<[[[f32; CHANNEL_DIMENSIONS]; CHANNEL_DIMENSIONS]; 4]>,
}

impl Default for BinaryV2Projector {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryV2Projector {
    /// Initialize the deterministic 4-channel hyperplane projector.
    pub fn new() -> Self {
        let mut hyperplanes = Box::new([[[0.0f32; CHANNEL_DIMENSIONS]; CHANNEL_DIMENSIONS]; 4]);

        for channel in 0..4 {
            for h in 0..CHANNEL_DIMENSIONS {
                let seed_text = format!("gc_v2_hp_generic_ch{channel}_row{h}");
                let hash = blake3::hash(seed_text.as_bytes());
                let bytes = hash.as_bytes();

                for d in 0..CHANNEL_DIMENSIONS {
                    let b = bytes[(d * 7 + h * 3) % 32];
                    hyperplanes[channel][h][d] = (b as f32 / 127.5) - 1.0;
                }

                l2_normalize_64(&mut hyperplanes[channel][h]);
            }
        }

        Self { hyperplanes }
    }

    /// Project a search query into a 256-bit `BinaryFingerprint([u64; 4])`.
    pub fn project_query(&self, query: &str, rri: &RriEngine) -> BinaryFingerprint {
        let raw_tokens = tokenize_code_text(query);
        if raw_tokens.is_empty() {
            return BinaryFingerprint::default();
        }

        let expanded = expand_tokens_morphology(&raw_tokens);

        // Channel 0: Core Lexical & Subwords (TF-IDF weighted)
        let mut ch0_features: Vec<(&str, f32)> = Vec::new();
        for token in &expanded {
            let idf = rri.idf(&token.text);
            let weight = idf * (token.weight as f32 / 100.0);
            if weight > 0.05 {
                ch0_features.push((token.text.as_str(), weight));
            }
        }
        let word0 = self.project_channel(&ch0_features, 0);

        // Channel 1: Unsupervised RRI Co-occurrence Context
        let rri_dense = rri.project_sequence(&raw_tokens);
        let word1 = self.project_dense_to_channel(&rri_dense, 1);

        // Channel 2: Action & Call Topology
        let mut ch2_features: Vec<(&str, f32)> = Vec::new();
        for token in &expanded {
            let idf = rri.idf(&token.text);
            ch2_features.push((token.text.as_str(), idf));
        }
        let word2 = self.project_channel(&ch2_features, 2);

        // Channel 3: Path & Namespace Intent
        let mut ch3_features: Vec<(&str, f32)> = Vec::new();
        for token in &expanded {
            let idf = rri.idf(&token.text);
            ch3_features.push((token.text.as_str(), idf));
        }
        let word3 = self.project_channel(&ch3_features, 3);

        BinaryFingerprint([word0, word1, word2, word3])
    }

    /// Project an individual code symbol into a 256-bit fingerprint.
    pub fn project_symbol(
        &self,
        symbol: &CodeSymbol,
        doc_path: &str,
        grammar: Option<&ExtractedGrammarSemantics>,
        rri: &RriEngine,
    ) -> BinaryFingerprint {
        let mut ch0_tokens = Vec::new();
        let mut ch2_tokens = Vec::new();
        let mut all_sequence_tokens = Vec::new();

        // 1. Symbol name (high salience)
        let name_tokens = tokenize_code_text(&symbol.name);
        for t in &name_tokens {
            ch0_tokens.push((t.clone(), 2.0));
            all_sequence_tokens.push(t.clone());
        }

        // 2. Signature subwords (parameters, return types)
        let sig_tokens = tokenize_code_text(&symbol.signature);
        for t in &sig_tokens {
            ch0_tokens.push((t.clone(), 1.0));
            ch2_tokens.push((t.clone(), 1.2));
            all_sequence_tokens.push(t.clone());
        }

        // 3. Docstring terms if present
        if let Some(ref doc) = symbol.docstring {
            let doc_tokens = tokenize_code_text(doc);
            for t in &doc_tokens {
                ch0_tokens.push((t.clone(), 0.7));
                all_sequence_tokens.push(t.clone());
            }
        }

        // 4. Grammar semantics (AST callees and interface tokens)
        if let Some(g) = grammar {
            for tok in &g.api_tokens {
                let subtokens = tokenize_code_text(&tok.text);
                for st in subtokens {
                    ch2_tokens.push((st.clone(), tok.weight * 1.5));
                    all_sequence_tokens.push(st);
                }
            }
            for tok in &g.interface_tokens {
                let subtokens = tokenize_code_text(&tok.text);
                for st in subtokens {
                    ch0_tokens.push((st.clone(), tok.weight));
                    ch2_tokens.push((st.clone(), tok.weight * 1.2));
                    all_sequence_tokens.push(st);
                }
            }
        }

        // Channel 0: Lexical with morphology
        let raw_ch0: Vec<String> = ch0_tokens.iter().map(|(t, _)| t.clone()).collect();
        let expanded_ch0 = expand_tokens_morphology(&raw_ch0);
        let mut ch0_features: Vec<(&str, f32)> = Vec::new();
        for exp in &expanded_ch0 {
            let idf = rri.idf(&exp.text);
            let weight = idf * (exp.weight as f32 / 100.0);
            if weight > 0.05 {
                ch0_features.push((exp.text.as_str(), weight));
            }
        }
        let word0 = self.project_channel(&ch0_features, 0);

        // Channel 1: Unsupervised RRI dense projection
        let rri_dense = rri.project_sequence(&all_sequence_tokens);
        let word1 = self.project_dense_to_channel(&rri_dense, 1);

        // Channel 2: AST API calls and types
        let mut ch2_features: Vec<(&str, f32)> = Vec::new();
        for (t, base_w) in &ch2_tokens {
            let idf = rri.idf(t);
            ch2_features.push((t.as_str(), idf * base_w));
        }
        let word2 = self.project_channel(&ch2_features, 2);

        // Channel 3: Hierarchical path context with depth decay
        let ch3_features = extract_path_features(doc_path, rri);
        let ch3_ref: Vec<(&str, f32)> =
            ch3_features.iter().map(|(s, w)| (s.as_str(), *w)).collect();
        let word3 = self.project_channel(&ch3_ref, 3);

        BinaryFingerprint([word0, word1, word2, word3])
    }

    /// Project a document or chunk text with its file path context into a 256-bit fingerprint.
    pub fn project_document_or_chunk(
        &self,
        doc_path: &str,
        text: &str,
        rri: &RriEngine,
    ) -> BinaryFingerprint {
        let raw_tokens = tokenize_code_text(text);
        let expanded = expand_tokens_morphology(&raw_tokens);

        // Channel 0: Lexical features
        let mut ch0_features: Vec<(&str, f32)> = Vec::new();
        for token in &expanded {
            let idf = rri.idf(&token.text);
            let weight = idf * (token.weight as f32 / 100.0);
            if weight > 0.05 {
                ch0_features.push((token.text.as_str(), weight));
            }
        }
        let word0 = self.project_channel(&ch0_features, 0);

        // Channel 1: Unsupervised RRI co-occurrence
        let rri_dense = rri.project_sequence(&raw_tokens);
        let word1 = self.project_dense_to_channel(&rri_dense, 1);

        // Channel 2: Structural identifiers
        let mut ch2_features: Vec<(&str, f32)> = Vec::new();
        for token in &expanded {
            let idf = rri.idf(&token.text);
            ch2_features.push((token.text.as_str(), idf));
        }
        let word2 = self.project_channel(&ch2_features, 2);

        // Channel 3: Path hierarchy
        let ch3_features = extract_path_features(doc_path, rri);
        let ch3_ref: Vec<(&str, f32)> =
            ch3_features.iter().map(|(s, w)| (s.as_str(), *w)).collect();
        let word3 = self.project_channel(&ch3_ref, 3);

        BinaryFingerprint([word0, word1, word2, word3])
    }

    /// Project an individual channel's feature set into a 64-bit word.
    fn project_channel(&self, features: &[(&str, f32)], channel: usize) -> u64 {
        if features.is_empty() {
            return 0;
        }

        let mut sorted_features = features.to_vec();
        sorted_features.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted_features.truncate(48);

        let mut accum = [0.0f32; CHANNEL_DIMENSIONS];
        let mut total_weight = 0.0f32;

        for &(feat, weight) in &sorted_features {
            if weight <= 0.0 {
                continue;
            }
            let vec = hash_feature_to_64d(feat, channel);
            for d in 0..CHANNEL_DIMENSIONS {
                accum[d] += weight * vec[d];
            }
            total_weight += weight;
        }

        if total_weight > 0.0 {
            l2_normalize_64(&mut accum);
        }

        self.project_dense_to_channel(&accum, channel)
    }

    /// Project a 64-dimensional dense float vector against a channel's hyperplanes.
    fn project_dense_to_channel(&self, dense: &[f32; RRI_DIM], channel: usize) -> u64 {
        let mut word = 0u64;
        let hp_channel = &self.hyperplanes[channel];

        for h in 0..CHANNEL_DIMENSIONS {
            let mut dot = 0.0f32;
            for d in 0..CHANNEL_DIMENSIONS {
                dot += dense[d] * hp_channel[h][d];
            }
            if dot > 0.0 {
                word |= 1u64 << h;
            }
        }

        word
    }
}

/// Extract hierarchical path features with depth decay.
fn extract_path_features(doc_path: &str, rri: &RriEngine) -> Vec<(String, f32)> {
    let mut features = Vec::new();
    let parts: Vec<&str> =
        doc_path.split(|c| c == '/' || c == '\\').filter(|p| !p.is_empty()).collect();
    let total_parts = parts.len();

    for (depth, part) in parts.iter().enumerate() {
        // Depth decay: deeper components are more specific, given higher specificity
        let depth_weight = 1.0 / (1.0 + (total_parts.saturating_sub(depth + 1)) as f32);
        let tokens = tokenize_code_text(part);
        for t in tokens {
            let idf = rri.idf(&t);
            features.push((t, idf * depth_weight * 1.5));
        }
    }

    features
}

/// Helper to L2-normalize a 64-dimensional float vector.
#[inline]
fn l2_normalize_64(v: &mut [f32; CHANNEL_DIMENSIONS]) {
    let mut norm_sq = 0.0f32;
    for &val in v.iter() {
        norm_sq += val * val;
    }
    if norm_sq > 1e-12 {
        let inv = 1.0 / norm_sq.sqrt();
        for val in v.iter_mut() {
            *val *= inv;
        }
    }
}

/// Deterministically hash any feature string into 64 dimensions using Blake3.
#[inline]
fn hash_feature_to_64d(feature: &str, channel: usize) -> [f32; CHANNEL_DIMENSIONS] {
    let mut vec = [0.0f32; CHANNEL_DIMENSIONS];
    let key_bytes = format!("gc_v2_feat_key_ch_{channel}");
    let key_hash = blake3::hash(key_bytes.as_bytes());
    let hash = blake3::keyed_hash(key_hash.as_bytes(), feature.as_bytes());
    let bytes = hash.as_bytes();

    for d in 0..CHANNEL_DIMENSIONS {
        let b = bytes[d % 32];
        let sign = if (b & (1 << (d / 32))) != 0 { 1.0f32 } else { -1.0f32 };
        let mag = (b as f32) / 255.0;
        vec[d] = sign * mag;
    }

    vec
}
