//! Syntactic identifier normalization and compound token expansion.
//!
//! Bridges lexical and semantic gaps by splitting and expanding compound identifiers
//! across camelCase, PascalCase, snake_case, and kebab-case, and expanding common abbreviations.

use std::collections::HashSet;

/// Split an identifier by camelCase, PascalCase, snake_case, and kebab-case into normalized lower-case sub-tokens.
pub fn split_identifier(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];
        if c == '_' || c == '-' || c == ':' || c == '.' || c == '/' || c == '\\' {
            if !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
            continue;
        }

        if c.is_uppercase() {
            // Check if boundary between lower and upper (e.g. `camelCase`)
            // or upper sequence followed by lower (e.g. `HTTPRequest` -> `HTTP`, `Request`)
            let is_boundary = if i > 0 && chars[i - 1].is_lowercase() {
                true
            } else if i > 0
                && chars[i - 1].is_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_lowercase()
            {
                true
            } else {
                false
            };

            if is_boundary && !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
        }

        current.push(c);
    }

    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }

    tokens
}

/// Expand common abbreviations and acronyms into full natural-language concepts.
pub fn expand_abbreviation(token: &str) -> Option<&'static [&'static str]> {
    match token {
        "ctx" => Some(&["context"]),
        "auth" => Some(&["authentication", "authorization"]),
        "req" => Some(&["request"]),
        "res" | "resp" => Some(&["response"]),
        "err" => Some(&["error"]),
        "msg" => Some(&["message"]),
        "cfg" => Some(&["config", "configuration"]),
        "repo" => Some(&["repository"]),
        "db" => Some(&["database"]),
        "init" => Some(&["initialize", "initialization"]),
        "alloc" => Some(&["allocation", "allocate"]),
        "sync" => Some(&["synchronize", "synchronization"]),
        "async" => Some(&["asynchronous"]),
        "ptr" => Some(&["pointer"]),
        "doc" | "docs" => Some(&["document", "documentation"]),
        "jwt" => Some(&["token", "authentication"]),
        "api" => Some(&["endpoint", "interface"]),
        "pvc" => Some(&["persistentvolumeclaim", "volume", "storage"]),
        "k8s" => Some(&["kubernetes"]),
        _ => None,
    }
}

/// Extract identifier sub-tokens and acronym expansions from symbol name and scope path.
pub fn extract_semantic_tokens(
    _raw_text: &str,
    symbol_name: &str,
    scope_path: &str,
) -> Vec<String> {
    let mut tokens = HashSet::new();

    // 1. Compound identifier decomposition & acronym expansion for symbol name
    for sub in split_identifier(symbol_name) {
        if let Some(expansions) = expand_abbreviation(&sub) {
            for &exp in expansions {
                tokens.insert(exp.to_string());
            }
        }
        tokens.insert(sub);
    }

    // 2. Compound identifier decomposition & acronym expansion for scope path
    for sub in split_identifier(scope_path) {
        if let Some(expansions) = expand_abbreviation(&sub) {
            for &exp in expansions {
                tokens.insert(exp.to_string());
            }
        }
        tokens.insert(sub);
    }

    tokens.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_identifier() {
        assert_eq!(split_identifier("handleAuthFailure"), vec!["handle", "auth", "failure"]);
        assert_eq!(split_identifier("reconcile_pvc_binding"), vec!["reconcile", "pvc", "binding"]);
        assert_eq!(split_identifier("HTTPRequestParser"), vec!["http", "request", "parser"]);
    }

    #[test]
    fn test_extract_semantic_tokens_decomposition() {
        let text = "fn validate_jwt(token: &str) -> Result<Claims, AuthError> { ... }";
        let tags = extract_semantic_tokens(text, "validate_jwt", "auth::service");
        assert!(tags.iter().any(|t| t == "validate"));
        assert!(tags.iter().any(|t| t == "jwt"));
        assert!(tags.iter().any(|t| t == "token"));
        assert!(tags.iter().any(|t| t == "authentication"));
        assert!(tags.iter().any(|t| t == "auth"));
        assert!(tags.iter().any(|t| t == "service"));
    }

    #[test]
    fn test_extract_semantic_tokens_scope() {
        let text = "def list_users(): return []";
        let tags = extract_semantic_tokens(text, "list_users", "controllers::api_v1");
        assert!(tags.iter().any(|t| t == "list"));
        assert!(tags.iter().any(|t| t == "users"));
        assert!(tags.iter().any(|t| t == "controllers"));
        assert!(tags.iter().any(|t| t == "api"));
        assert!(tags.iter().any(|t| t == "endpoint"));
    }
}
