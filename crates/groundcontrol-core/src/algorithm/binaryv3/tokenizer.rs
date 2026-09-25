//! Universal, code-agnostic tokenizer and morphological normalizer for binaryv3.
//!
//! Provides camelCase/snake_case/kebab-case splitting, structural suffix handling,
//! universal programming abbreviation expansions, and rule-based English suffix normalization (stemming)
//! with salience multipliers specified in RFC Pillar 1.

/// Semantic role and salience category of an extracted token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Primary symbol identifier or query root token (2.0x weight).
    Name,
    /// Function/method signature parameter or type token (1.2x weight).
    Sig,
    /// Subword morphology stem or camelCase split (1.0x weight).
    Subword,
    /// Docstring or prose context token (0.8x weight).
    Doc,
    /// Universal abbreviation expansion (0.75x weight).
    Abbrev,
}

impl TokenKind {
    /// Return the SIF salience multiplier μ_t specified in Pillar 1.
    #[inline]
    pub fn salience_multiplier(&self) -> f32 {
        match self {
            Self::Name => 2.0,
            Self::Sig => 1.2,
            Self::Subword => 1.0,
            Self::Doc => 0.8,
            Self::Abbrev => 0.75,
        }
    }
}

/// An extracted token tagged with its semantic role.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractedToken {
    /// Normalized lowercase token text.
    pub text: String,
    /// Semantic category for salience weighting.
    pub kind: TokenKind,
}

/// Tokenize an identifier, code snippet, path, or query string into normalized subwords.
pub fn tokenize_code_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return tokens;
    }

    let mut i = 0;
    while i < len {
        // Skip non-alphanumeric characters
        while i < len && !chars[i].is_alphanumeric() {
            i += 1;
        }
        if i >= len {
            break;
        }

        let start = i;
        // Check if starting a numeric segment
        if chars[i].is_numeric() {
            while i < len && chars[i].is_numeric() {
                i += 1;
            }
            let num_str: String = chars[start..i].iter().collect();
            if num_str.len() <= 6 {
                tokens.push(num_str);
            }
            continue;
        }

        // Alphabetic segment: handle camelCase and PascalCase boundaries
        while i < len && chars[i].is_alphabetic() {
            // Boundary: lower followed by upper (e.g. "cartItem" -> "cart", "Item")
            if i > start && chars[i].is_uppercase() && chars[i - 1].is_lowercase() {
                break;
            }
            // Boundary: acronym followed by title case (e.g. "XMLParser" -> "XML", "Parser")
            if i > start
                && i + 1 < len
                && chars[i].is_uppercase()
                && chars[i - 1].is_uppercase()
                && chars[i + 1].is_lowercase()
            {
                break;
            }
            i += 1;
        }

        let word: String = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
        if word.len() >= 2 {
            tokens.push(word.clone());
            // Structural component suffix splitting (e.g. "shippingservice" -> "shipping", "service")
            for &suf in STRUCTURAL_SUFFIXES {
                if word.len() > suf.len() + 2 && word.ends_with(suf) {
                    let prefix = &word[..word.len() - suf.len()];
                    tokens.push(prefix.to_string());
                    tokens.push(suf.to_string());
                    break;
                }
            }
        }
    }

    tokens
}

/// Universal architectural component suffixes common in software packages and directories.
const STRUCTURAL_SUFFIXES: &[&str] = &[
    "service",
    "server",
    "client",
    "store",
    "handler",
    "controller",
    "manager",
    "provider",
    "gateway",
    "worker",
    "catalog",
    "factory",
    "stream",
    "queue",
    "router",
    "model",
    "view",
    "repository",
];

/// Universal programming contractions and abbreviation expansions.
/// Covers standard conventions across Go, Rust, TS, C++, Java, and Python.
pub const UNIVERSAL_ABBREVIATIONS: &[(&str, &str)] = &[
    ("req", "request"),
    ("resp", "response"),
    ("res", "response"),
    ("err", "error"),
    ("exc", "exception"),
    ("msg", "message"),
    ("ctx", "context"),
    ("cfg", "config"),
    ("conf", "configuration"),
    ("init", "initialize"),
    ("auth", "authentication"),
    ("tx", "transaction"),
    ("txn", "transaction"),
    ("id", "identifier"),
    ("num", "number"),
    ("str", "string"),
    ("calc", "calculate"),
    ("param", "parameter"),
    ("arg", "argument"),
    ("pkg", "package"),
    ("fn", "function"),
    ("func", "function"),
    ("cb", "callback"),
    ("db", "database"),
    ("recv", "receive"),
    ("sync", "synchronize"),
    ("desc", "description"),
    ("info", "information"),
    ("doc", "document"),
    ("proto", "protobuf"),
];

/// Rule-based morphological suffix stripping (Porter-Lite for code identifiers).
pub fn stem_suffix(token: &str) -> Option<String> {
    if token.len() <= 4 {
        return None;
    }

    // -ization / -isation -> stem (e.g. "authorization" -> "authoriz", "optimization" -> "optimiz")
    if token.ends_with("ization") && token.len() > 7 {
        return Some(token[..token.len() - 5].to_string());
    }
    if token.ends_with("isation") && token.len() > 7 {
        return Some(format!("{}z", &token[..token.len() - 6]));
    }
    if token.ends_with("ize") && token.len() > 4 {
        return Some(token[..token.len() - 1].to_string());
    }
    if token.ends_with("ise") && token.len() > 4 {
        return Some(format!("{}z", &token[..token.len() - 2]));
    }

    // -ation / -ition -> stem (e.g. "calculation" -> "calculat")
    if token.ends_with("ation") && token.len() > 6 {
        return Some(format!("{}at", &token[..token.len() - 5]));
    }
    // -ate -> stem (e.g. "calculate" -> "calculat", "validate" -> "validat")
    if token.ends_with("ate") && token.len() > 5 {
        return Some(format!("{}at", &token[..token.len() - 3]));
    }
    if token.ends_with("ition") && token.len() > 6 {
        return Some(token[..token.len() - 5].to_string());
    }

    // -sion / -tion -> stem (e.g. "conversion" -> "convert")
    if token.ends_with("sion") && token.len() > 5 {
        let base = &token[..token.len() - 4];
        if base.ends_with("ver") {
            return Some(format!("{base}t"));
        }
        return Some(base.to_string());
    }
    if token.ends_with("tion") && token.len() > 5 {
        return Some(format!("{}t", &token[..token.len() - 4]));
    }

    // -ing -> base (e.g. "shipping" -> "ship", "tracking" -> "track")
    if token.ends_with("ing") && token.len() > 5 {
        let base = &token[..token.len() - 3];
        let chars: Vec<char> = base.chars().collect();
        if chars.len() >= 3 && chars[chars.len() - 1] == chars[chars.len() - 2] {
            return Some(chars[..chars.len() - 1].iter().collect());
        }
        return Some(base.to_string());
    }

    // -ment -> base (e.g. "payment" -> "pay", "settlement" -> "settle")
    if token.ends_with("ment") && token.len() > 6 {
        return Some(token[..token.len() - 4].to_string());
    }

    // -able / -ible (e.g. "payable" -> "pay")
    if (token.ends_with("able") || token.ends_with("ible")) && token.len() > 6 {
        return Some(token[..token.len() - 4].to_string());
    }

    // -ies -> -y (e.g. "currencies" -> "currency", "categories" -> "category")
    if token.ends_with("ies") && token.len() > 4 {
        return Some(format!("{}y", &token[..token.len() - 3]));
    }

    // -es -> base (e.g. "rates" -> "rate", "services" -> "service")
    if token.ends_with("es") && token.len() > 4 {
        return Some(token[..token.len() - 1].to_string());
    }

    // -s -> base (e.g. "products" -> "product", "quotes" -> "quote")
    if token.ends_with('s') && !token.ends_with("ss") && token.len() > 3 {
        return Some(token[..token.len() - 1].to_string());
    }

    None
}

/// Expand tokens with their morphological stems and universal code abbreviations.
pub fn expand_tokens_morphology(raw_tokens: &[String]) -> Vec<ExtractedToken> {
    let mut result = Vec::with_capacity(raw_tokens.len() * 3);

    for token in raw_tokens {
        // Original token as Subword
        result.push(ExtractedToken { text: token.clone(), kind: TokenKind::Subword });

        // Morphological stem as Subword
        if let Some(stem) = stem_suffix(token) {
            if stem != *token && stem.len() >= 2 {
                result.push(ExtractedToken { text: stem, kind: TokenKind::Subword });
            }
        }

        // Universal abbreviation expansion
        for &(abbrev, expanded) in UNIVERSAL_ABBREVIATIONS {
            if token == abbrev {
                result.push(ExtractedToken { text: expanded.to_string(), kind: TokenKind::Abbrev });
                break;
            } else if token == expanded {
                result.push(ExtractedToken { text: abbrev.to_string(), kind: TokenKind::Abbrev });
                break;
            }
        }
    }

    result
}
