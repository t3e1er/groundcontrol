//! Domain types and configuration options for binaryv3 retrieval.
//!
//! Implements Pillar 2: Zero-Cost Bayesian Structural Prior (`EntityPriorFlags`)
//! and packed in-memory record layout (`FingerprintV3Record`).

use groundcontrol_common::types::{BinaryFingerprint, CodeSymbol, CodeSymbolType, Modality};
use serde::{Deserialize, Serialize};

/// 1-byte packed structural metadata prior flags.
///
/// Encodes entity hierarchy, visibility, and role without corrupting the
/// 256-bit continuous metric embedding space. Applied as an O(1) post-Hamming multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct EntityPriorFlags(pub u8);

impl EntityPriorFlags {
    /// Entity is a public or exported symbol (1.15x boost).
    pub const IS_PUBLIC_EXPORT: u8 = 1 << 0;
    /// Entity is in a test, spec, mock, or fixture file (0.75x penalty).
    pub const IS_TEST_OR_MOCK: u8 = 1 << 1;
    /// Entity is a function or method (1.10x boost).
    pub const KIND_FUNCTION: u8 = 1 << 2;
    /// Entity is a class, struct, trait, or interface (1.05x boost).
    pub const KIND_CLASS: u8 = 1 << 3;
    /// Entity is an HTTP route, controller, endpoint, or API handler (1.20x boost).
    pub const KIND_ROUTE_API: u8 = 1 << 4;
    /// Entity is at root file/module scope (1.05x boost).
    pub const IS_ROOT_SCOPE: u8 = 1 << 5;

    /// Compute the Bayesian structural prior multiplier P(D).
    #[inline]
    pub fn compute_multiplier(&self) -> f32 {
        let mut m = 1.0f32;
        if self.0 & Self::IS_PUBLIC_EXPORT != 0 {
            m *= 1.15;
        }
        if self.0 & Self::KIND_ROUTE_API != 0 {
            m *= 1.20;
        } else if self.0 & Self::KIND_FUNCTION != 0 {
            m *= 1.10;
        } else if self.0 & Self::KIND_CLASS != 0 {
            m *= 1.05;
        }
        if self.0 & Self::IS_ROOT_SCOPE != 0 {
            m *= 1.05;
        }
        if self.0 & Self::IS_TEST_OR_MOCK != 0 {
            m *= 0.75;
        }
        m
    }

    /// Infer prior flags for a code symbol.
    pub fn from_symbol(sym: &CodeSymbol, doc_path: &str) -> Self {
        let path_lower = doc_path.to_lowercase();
        let name_lower = sym.name.to_lowercase();
        let sig_lower = sym.signature.to_lowercase();

        let is_test = path_lower.contains("test")
            || path_lower.contains("spec")
            || path_lower.contains("mock")
            || path_lower.contains("fixture")
            || path_lower.contains("loadgenerator")
            || path_lower.contains("locust")
            || path_lower.contains("cypress")
            || name_lower.starts_with("test")
            || name_lower.ends_with("test")
            || name_lower.contains("mock");

        let is_route = name_lower.contains("route")
            || name_lower.contains("endpoint")
            || name_lower.contains("handler")
            || name_lower.contains("controller")
            || name_lower.contains("quote")
            || sig_lower.contains("request")
            || sig_lower.contains("response")
            || sig_lower.contains("req")
            || sig_lower.contains("resp")
            || sig_lower.contains("grpc");

        let is_fn = matches!(sym.symbol_type, CodeSymbolType::Function | CodeSymbolType::Method);

        let is_class = matches!(
            sym.symbol_type,
            CodeSymbolType::Class
                | CodeSymbolType::Interface
                | CodeSymbolType::Trait
                | CodeSymbolType::Struct
                | CodeSymbolType::Enum
                | CodeSymbolType::TypeAlias
        );

        let is_public = sym.signature.contains("pub ")
            || sym.signature.contains("export ")
            || sym.name.chars().next().map_or(false, |c| c.is_uppercase())
            || (!sym.name.starts_with('_') && !sym.name.is_empty());

        let is_root = !sym.scope_path.contains("::") && !sym.scope_path.contains('.');

        let mut flags = 0u8;
        if is_public {
            flags |= Self::IS_PUBLIC_EXPORT;
        }
        if is_test {
            flags |= Self::IS_TEST_OR_MOCK;
        }
        if is_route {
            flags |= Self::KIND_ROUTE_API;
        } else if is_fn {
            flags |= Self::KIND_FUNCTION;
        } else if is_class {
            flags |= Self::KIND_CLASS;
        }
        if is_root {
            flags |= Self::IS_ROOT_SCOPE;
        }

        Self(flags)
    }

    /// Infer prior flags for a whole document or file.
    pub fn from_path(doc_path: &str) -> Self {
        let path_lower = doc_path.to_lowercase();
        let mut flags = 0u8;

        if path_lower.contains("test")
            || path_lower.contains("mock")
            || path_lower.contains("spec")
            || path_lower.contains("fixture")
        {
            flags |= Self::IS_TEST_OR_MOCK;
        }

        if path_lower.ends_with(".proto") {
            flags |= Self::IS_PUBLIC_EXPORT;
        } else if path_lower.ends_with("main.go")
            || path_lower.ends_with("main.rs")
            || path_lower.ends_with("server.cpp")
            || path_lower.ends_with("index.js")
            || path_lower.ends_with("charge.js")
        {
            flags |= Self::IS_PUBLIC_EXPORT | Self::IS_ROOT_SCOPE;
        }

        Self(flags)
    }

    /// Infer prior flags for a document chunk.
    pub fn from_chunk(doc_path: &str) -> Self {
        let path_lower = doc_path.to_lowercase();
        let mut flags = 0u8;
        if path_lower.contains("test")
            || path_lower.contains("mock")
            || path_lower.contains("spec")
            || path_lower.contains("fixture")
        {
            flags |= Self::IS_TEST_OR_MOCK;
        }
        Self(flags)
    }
}

/// An indexed binaryv3 entity fingerprint record with cached structural prior flags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintV3Record {
    /// Fully qualified entity ID (`path`, `path#scope`, or `path:chunk:idx`).
    pub id: String,
    /// 256-bit Matryoshka binary fingerprint.
    pub fingerprint: BinaryFingerprint,
    /// Modality partition (Code or Docs).
    pub modality: Modality,
    /// 1-byte Bayesian structural prior flags.
    pub flags: EntityPriorFlags,
}

/// Runtime configuration options for the binaryv3 retrieval algorithm.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BinaryV3Config {
    /// Multiplier on `limit` for candidate pool size in Hamming search.
    pub pool_multiplier: usize,
    /// Top M candidates to pass to Stage 2 Bayesian prior rescoring (default: 50).
    pub candidate_pool: usize,
    /// Whether to apply Stage 2 Bayesian structural prior (Pillar 2).
    pub apply_prior: bool,
    /// Whether to apply Matryoshka early-exit filtering on Word 0 (Pillar 3).
    pub early_exit: bool,
    /// Early exit Hamming distance threshold on Word 0 (default: 42).
    pub early_exit_threshold: u32,
}

impl Default for BinaryV3Config {
    fn default() -> Self {
        Self {
            pool_multiplier: 50,
            candidate_pool: 50,
            apply_prior: true,
            early_exit: false,
            early_exit_threshold: 42,
        }
    }
}
