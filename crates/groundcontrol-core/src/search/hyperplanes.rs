//! Multi-channel partitioned hyperplane binary projections for code search.
//!
//! Replaces monolithic Bag-of-Words sign thresholding with 4 orthogonal 64-bit
//! semantic channels:
//! - Word 0 (Bits 0..63): Interface and declaration signatures.
//! - Word 1 (Bits 64..127): Outbound API invocations and dependencies.
//! - Word 2 (Bits 128..191): Intra-symbol def-use data flow paths.
//! - Word 3 (Bits 192..255): Structural control grammar transitions and AST shape.
//!
//! Each channel is projected through 64 deterministic orthonormal hyperplanes generated
//! via Blake3, ensuring maximum bit entropy and sub-millisecond single-cycle POPCOUNT matching.

use groundcontrol_common::types::BinaryFingerprint;

use crate::parser::code::grammar::{DataFlowSink, ExtractedGrammarSemantics};
use crate::parser::code::patterns::split_identifier;

/// Dimension of intermediate float vectors for each 64-bit channel.
pub const CHANNEL_DIMENSIONS: usize = 64;

/// Deterministic 32-byte keyed hash keys for each channel.
const CHANNEL_KEYS: [[u8; 32]; 4] = [
    *b"gc_hyperplane_channel_interface0",
    *b"gc_hyperplane_channel_api_calls1",
    *b"gc_hyperplane_channel_dataflow_2",
    *b"gc_hyperplane_channel_grammar_03",
];

/// Multi-channel partitioned hyperplane projection engine.
#[derive(Debug, Clone)]
pub struct PartitionedHyperplaneProjector {
    /// 4 channels, each having 64 hyperplanes of 64 dimensions: `[4][64][64]`
    hyperplanes: Box<[[[f32; CHANNEL_DIMENSIONS]; CHANNEL_DIMENSIONS]; 4]>,
}

impl Default for PartitionedHyperplaneProjector {
    fn default() -> Self {
        Self::new()
    }
}

impl PartitionedHyperplaneProjector {
    /// Initialize the deterministic 4-channel hyperplane projector.
    pub fn new() -> Self {
        let mut hyperplanes = Box::new([[[0.0f32; CHANNEL_DIMENSIONS]; CHANNEL_DIMENSIONS]; 4]);

        for channel in 0..4 {
            for h in 0..CHANNEL_DIMENSIONS {
                let seed_text = format!("gc_hp_ch{channel}_row{h}");
                let hash = blake3::hash(seed_text.as_bytes());
                let bytes = hash.as_bytes();

                for d in 0..CHANNEL_DIMENSIONS {
                    let b = bytes[(d * 7 + h * 3) % 32];
                    hyperplanes[channel][h][d] = (b as f32 / 127.5) - 1.0;
                }

                // Orthonormalize row
                l2_normalize_64(&mut hyperplanes[channel][h]);
            }
        }

        Self { hyperplanes }
    }

    /// Project extracted AST grammar semantics into a 256-bit `BinaryFingerprint([u64; 4])`.
    pub fn project_semantics(&self, semantics: &ExtractedGrammarSemantics) -> BinaryFingerprint {
        // Channel 0: Interface and Declaration
        let ch0_features: Vec<(&str, f32)> =
            semantics.interface_tokens.iter().map(|t| (t.text.as_str(), t.weight)).collect();
        let word0 = self.project_channel(&ch0_features, 0);

        // Channel 1: Outbound API Calls
        let ch1_features: Vec<(&str, f32)> =
            semantics.api_tokens.iter().map(|t| (t.text.as_str(), t.weight)).collect();
        let word1 = self.project_channel(&ch1_features, 1);

        // Channel 2: Def-Use Data Flow
        let flow_strings: Vec<String> = semantics
            .dataflow_paths
            .iter()
            .map(|p| match &p.sink {
                DataFlowSink::Call(target) => format!("flow:{}=>call({})", p.source_param, target),
                DataFlowSink::Return => format!("flow:{}=>return", p.source_param),
                DataFlowSink::Condition => format!("flow:{}=>cond", p.source_param),
            })
            .collect();
        let ch2_features: Vec<(&str, f32)> =
            flow_strings.iter().map(|s| (s.as_str(), 1.0f32)).collect();
        let word2 = self.project_channel(&ch2_features, 2);

        // Channel 3: Structural Control Grammar Transitions
        let grammar_strings: Vec<(String, f32)> = semantics
            .grammar_transitions
            .iter()
            .map(|t| {
                let weight = 1.0 / (1.0 + t.depth as f32).sqrt();
                (format!("{}->{}", t.parent_kind, t.child_kind), weight)
            })
            .collect();
        let ch3_features: Vec<(&str, f32)> =
            grammar_strings.iter().map(|(s, w)| (s.as_str(), *w)).collect();
        let word3 = self.project_channel(&ch3_features, 3);

        BinaryFingerprint([word0, word1, word2, word3])
    }

    /// Project a natural-language search query into a 256-bit `BinaryFingerprint`.
    ///
    /// # Channel Projection Invariant
    ///
    /// Documents are indexed with **structurally heterogeneous** feature types per channel:
    /// - Ch0: identifier sub-tokens from the symbol interface
    /// - Ch1: callee names with depth attenuation
    /// - Ch2: dataflow path strings (`flow:param=>call(target)`, `flow:param=>return`)
    /// - Ch3: Tree-sitter parent→child kind bigrams (`function_item->block`)
    ///
    /// A natural-language query carries *only* raw text tokens. Projecting those into
    /// Ch2 and Ch3 with their channel-keyed Blake3 hashes produces fingerprints that are
    /// orthogonal to any indexed document's Ch2/Ch3 words (different vocabulary, different
    /// key domain). This destroys Hamming similarity and collapses Sep Ratio.
    ///
    /// **Fix**: Ch2 and Ch3 are zeroed. The search layer (`search_hamming`) must use
    /// `masked_hamming_distance([true, true, false, false])` so that only Ch0 and Ch1
    /// contribute to ranking when comparing against a text-query fingerprint.
    pub fn project_query(&self, query: &str) -> BinaryFingerprint {
        let words: Vec<&str> = query
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .filter(|w| !w.is_empty())
            .collect();

        if words.is_empty() {
            return BinaryFingerprint::default();
        }

        let mut query_features: Vec<(String, f32)> = Vec::new();
        for &w in &words {
            query_features.push((w.to_lowercase(), 1.0));
            for sub in split_identifier(w) {
                let sub_lower = sub.to_lowercase();
                if sub_lower != w.to_lowercase() {
                    query_features.push((sub_lower, 0.8));
                }
            }
        }

        let features_ref: Vec<(&str, f32)> =
            query_features.iter().map(|(s, w)| (s.as_str(), *w)).collect();

        // Ch0 (interface) and Ch1 (API calls): both receive plain-text query tokens.
        let word0 = self.project_channel(&features_ref, 0);
        let word1 = self.project_channel(&features_ref, 1);
        // Ch2 (dataflow) and Ch3 (grammar bigrams): MUST be zero for query fingerprints.
        // Their feature vocabularies (flow path strings, AST kind bigrams) are structurally
        // incompatible with plain text — projecting text into these channels produces
        // random orthogonal bits that destroy Hamming proximity.
        let word2 = 0u64;
        let word3 = 0u64;

        BinaryFingerprint([word0, word1, word2, word3])
    }

    /// Project an individual channel's feature set into a 64-bit word.
    fn project_channel(&self, features: &[(&str, f32)], channel: usize) -> u64 {
        if features.is_empty() {
            return 0;
        }

        let mut accum = [0.0f32; CHANNEL_DIMENSIONS];
        let mut total_weight = 0.0f32;

        for &(feat, weight) in features {
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

        let mut word = 0u64;
        let hp_channel = &self.hyperplanes[channel];

        for h in 0..CHANNEL_DIMENSIONS {
            let mut dot = 0.0f32;
            for d in 0..CHANNEL_DIMENSIONS {
                dot += accum[d] * hp_channel[h][d];
            }
            if dot > 0.0 {
                word |= 1u64 << h;
            }
        }

        word
    }
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
    let key = &CHANNEL_KEYS[channel.min(3)];
    let hash = blake3::keyed_hash(key, feature.as_bytes());
    let bytes = hash.as_bytes(); // 32 bytes

    for d in 0..CHANNEL_DIMENSIONS {
        let b = bytes[d % 32];
        let sign = if (b & (1 << (d / 32))) != 0 { 1.0f32 } else { -1.0f32 };
        let mag = (b as f32) / 255.0;
        vec[d] = sign * mag;
    }

    vec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::code::grammar::WeightedToken;

    #[test]
    fn test_hyperplane_projector_basic() {
        let projector = PartitionedHyperplaneProjector::new();

        let mut sem1 = ExtractedGrammarSemantics::default();
        sem1.interface_tokens
            .push(WeightedToken { text: "auth_token_validator".into(), weight: 1.0 });
        sem1.api_tokens.push(WeightedToken { text: "verify_jwt".into(), weight: 1.0 });

        let mut sem2 = ExtractedGrammarSemantics::default();
        sem2.interface_tokens
            .push(WeightedToken { text: "auth_token_validator".into(), weight: 1.0 });
        sem2.api_tokens.push(WeightedToken { text: "verify_jwt".into(), weight: 1.0 });

        let fp1 = projector.project_semantics(&sem1);
        let fp2 = projector.project_semantics(&sem2);

        // Identical semantics produce identical fingerprints
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.hamming_distance(&fp2), 0);
    }

    #[test]
    fn test_subject_object_inversion_distinction() {
        let projector = PartitionedHyperplaneProjector::new();

        // Symbol A: client.send(packet)
        let mut sem_a = ExtractedGrammarSemantics::default();
        sem_a.interface_tokens.push(WeightedToken { text: "dispatch".into(), weight: 1.0 });
        sem_a.api_tokens.push(WeightedToken { text: "client.send".into(), weight: 1.0 });
        sem_a.dataflow_paths.push(crate::parser::code::grammar::DataFlowPath {
            source_param: "packet".into(),
            sink: DataFlowSink::Call("client.send".into()),
        });

        // Symbol B: packet.send(client)
        let mut sem_b = ExtractedGrammarSemantics::default();
        sem_b.interface_tokens.push(WeightedToken { text: "dispatch".into(), weight: 1.0 });
        sem_b.api_tokens.push(WeightedToken { text: "packet.send".into(), weight: 1.0 });
        sem_b.dataflow_paths.push(crate::parser::code::grammar::DataFlowPath {
            source_param: "client".into(),
            sink: DataFlowSink::Call("packet.send".into()),
        });

        let fp_a = projector.project_semantics(&sem_a);
        let fp_b = projector.project_semantics(&sem_b);

        let dist = fp_a.hamming_distance(&fp_b);
        assert!(
            dist >= 12,
            "Subject/object inversion must produce significant Hamming distance, got {}",
            dist
        );
    }

    #[test]
    fn test_channel_separation() {
        let projector = PartitionedHyperplaneProjector::new();

        let mut sem = ExtractedGrammarSemantics::default();
        sem.interface_tokens.push(WeightedToken { text: "login".into(), weight: 1.0 });
        sem.api_tokens.push(WeightedToken { text: "authenticate".into(), weight: 1.0 });

        let fp = projector.project_semantics(&sem);

        // Word 0 and Word 1 should have bits set
        assert_ne!(fp.0[0], 0);
        assert_ne!(fp.0[1], 0);
        // Word 2 and Word 3 should be 0 because no dataflow or grammar transitions were added
        assert_eq!(fp.0[2], 0);
        assert_eq!(fp.0[3], 0);
    }
}
