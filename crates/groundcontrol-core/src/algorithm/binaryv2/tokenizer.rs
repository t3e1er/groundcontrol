//! Universal, code-agnostic tokenizer and morphological normalizer.
//!
//! Provides camelCase/snake_case/kebab-case splitting, language-agnostic
//! abbreviation expansion, and rule-based English suffix normalization (stemming)
//! for code identifiers and search queries without any business-domain dependencies.

/// A token extracted from code or query text, optionally carrying an expansion flag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractedToken {
    /// Normalized token text.
    pub text: String,
    /// Salience weight multiplier (e.g. 1.0 for original, 0.8 for stem/abbreviation).
    pub weight: u32,
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
];

/// Universal programming contractions and abbreviation expansions.
/// Covers standard compiler and cross-language conventions (Go, Rust, TS, C++, Java, Python).
const UNIVERSAL_ABBREVIATIONS: &[(&str, &str)] = &[
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
/// Normalizes verb tenses and plural forms so queries match symbols without domain lists.
fn stem_suffix(token: &str) -> Option<String> {
    if token.len() <= 4 {
        return None;
    }

    // -ation / -ition -> stem (e.g. "authorization" -> "authoriz", "calculation" -> "calculat")
    if token.ends_with("ation") && token.len() > 6 {
        return Some(format!("{}at", &token[..token.len() - 5]));
    }
    if token.ends_with("ition") && token.len() > 6 {
        return Some(token[..token.len() - 5].to_string());
    }

    // -sion / -tion -> stem (e.g. "conversion" -> "convert")
    if token.ends_with("sion") && token.len() > 5 {
        let base = &token[..token.len() - 4];
        if base.ends_with("ver") {
            return Some(format!("{base}t")); // conversion -> convert
        }
        return Some(base.to_string());
    }
    if token.ends_with("tion") && token.len() > 5 {
        return Some(format!("{}t", &token[..token.len() - 4]));
    }

    // -ing -> base (e.g. "shipping" -> "ship", "tracking" -> "track", "pricing" -> "price")
    if token.ends_with("ing") && token.len() > 5 {
        let base = &token[..token.len() - 3];
        // Double consonant simplification: "shipping" -> "ship", "setting" -> "sett" -> "set"
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
        return Some(token[..token.len() - 1].to_string()); // keeps the 'e': "services" -> "service"
    }

    // -s -> base (e.g. "products" -> "product", "quotes" -> "quote", "recommendations" -> "recommendation")
    if token.ends_with('s') && !token.ends_with("ss") && token.len() > 3 {
        return Some(token[..token.len() - 1].to_string());
    }

    None
}

/// Expand tokens with their morphological stems and universal code abbreviations.
pub fn expand_tokens_morphology(raw_tokens: &[String]) -> Vec<ExtractedToken> {
    let mut result = Vec::with_capacity(raw_tokens.len() * 2);

    for token in raw_tokens {
        // Original token with primary weight (100)
        result.push(ExtractedToken { text: token.clone(), weight: 100 });

        // Morphological stem with secondary weight (80)
        if let Some(stem) = stem_suffix(token) {
            if stem != *token && stem.len() >= 2 {
                result.push(ExtractedToken { text: stem, weight: 80 });
            }
        }

        // Universal abbreviation expansion
        for &(abbrev, expanded) in UNIVERSAL_ABBREVIATIONS {
            if token == abbrev {
                result.push(ExtractedToken { text: expanded.to_string(), weight: 75 });
                break;
            } else if token == expanded {
                result.push(ExtractedToken { text: abbrev.to_string(), weight: 75 });
                break;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_camel_and_snake() {
        assert_eq!(
            tokenize_code_text("ShippingService"),
            vec!["shipping".to_string(), "service".to_string()]
        );
        assert_eq!(tokenize_code_text("CartItem"), vec!["cart".to_string(), "item".to_string()]);
        assert_eq!(
            tokenize_code_text("get_quote_by_id"),
            vec!["get".to_string(), "quote".to_string(), "by".to_string(), "id".to_string()]
        );
        assert_eq!(
            tokenize_code_text("XMLParserService"),
            vec!["xml".to_string(), "parser".to_string(), "service".to_string()]
        );
    }

    #[test]
    fn test_stemming() {
        assert_eq!(stem_suffix("shipping"), Some("ship".to_string()));
        assert_eq!(stem_suffix("calculation"), Some("calculat".to_string()));
        assert_eq!(stem_suffix("payment"), Some("pay".to_string()));
        assert_eq!(stem_suffix("products"), Some("product".to_string()));
        assert_eq!(stem_suffix("currencies"), Some("currency".to_string()));
        assert_eq!(stem_suffix("conversion"), Some("convert".to_string()));
    }

    #[test]
    fn test_expand_tokens() {
        let tokens = tokenize_code_text("calculate shipping quote");
        let expanded = expand_tokens_morphology(&tokens);
        let texts: Vec<&str> = expanded.iter().map(|e| e.text.as_str()).collect();

        assert!(texts.contains(&"calculate"));
        assert!(texts.contains(&"shipping"));
        assert!(texts.contains(&"ship"));
        assert!(texts.contains(&"quote"));
    }
}
