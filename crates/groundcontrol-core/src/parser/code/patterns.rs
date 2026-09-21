//! Syntactic AST pattern token injection and identifier normalization.
//!
//! Bridges lexical and semantic gaps by injecting canonical semantic markers
//! (`__sem_error`, `__sem_endpoint`, `__sem_auth`, `__sem_lifecycle`) into
//! full-text index postings at parse time, and splitting/expanding compound identifiers.

use std::collections::HashSet;

/// Semantic tags emitted for error handling constructs.
pub const ERROR_TAGS: &[&str] = &["__sem_error", "__sem_exception", "__sem_handler"];

/// Semantic tags emitted for HTTP/API routing constructs.
pub const ENDPOINT_TAGS: &[&str] = &["__sem_endpoint", "__sem_api"];

/// Semantic tags emitted for authentication and security constructs.
pub const AUTH_TAGS: &[&str] = &["__sem_auth", "__sem_security"];

/// Semantic tags emitted for lifecycle and I/O constructs.
pub const LIFECYCLE_TAGS: &[&str] = &["__sem_lifecycle", "__sem_io"];

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

/// Extract AST pattern tokens, identifier sub-tokens, and acronym expansions from code text and scope.
pub fn extract_semantic_tokens(raw_text: &str, symbol_name: &str, scope_path: &str) -> Vec<String> {
    let mut tokens = HashSet::new();
    let lower_text = raw_text.to_lowercase();
    let lower_name = symbol_name.to_lowercase();
    let lower_scope = scope_path.to_lowercase();

    // 1. Error Handling detection
    let has_error = lower_text.contains("catch")
        || lower_text.contains("except")
        || lower_text.contains("panic!")
        || lower_text.contains("panic(")
        || lower_text.contains("if err != nil")
        || lower_text.contains("result<")
        || lower_text.contains("raise ")
        || lower_text.contains("throw ")
        || lower_name.contains("error")
        || lower_name.contains("panic")
        || lower_name.contains("handler")
        || lower_name.contains("fault");

    if has_error {
        for &t in ERROR_TAGS {
            tokens.insert(t.to_string());
        }
    }

    // 2. API & Routing detection
    let has_endpoint = lower_text.contains("@get")
        || lower_text.contains("@post")
        || lower_text.contains("@put")
        || lower_text.contains("@delete")
        || lower_text.contains("#[get")
        || lower_text.contains("#[post")
        || lower_text.contains("#[route")
        || lower_text.contains("app.get")
        || lower_text.contains("app.post")
        || lower_text.contains("router.get")
        || lower_text.contains("router.post")
        || lower_name.contains("endpoint")
        || lower_name.contains("route")
        || lower_name.contains("handler")
        || lower_name.contains("api_view")
        || lower_scope.contains("controller")
        || lower_scope.contains("router");

    if has_endpoint {
        for &t in ENDPOINT_TAGS {
            tokens.insert(t.to_string());
        }
    }

    // 3. Authentication & Security detection
    let has_auth = lower_text.contains("bearer ")
        || lower_text.contains("jwt")
        || lower_text.contains("password")
        || lower_text.contains("credential")
        || lower_text.contains("permission")
        || lower_text.contains("rbac")
        || lower_name.contains("token")
        || lower_name.contains("auth")
        || lower_name.contains("security")
        || lower_name.contains("login")
        || lower_name.contains("logout");

    if has_auth {
        for &t in AUTH_TAGS {
            tokens.insert(t.to_string());
        }
    }

    // 4. Lifecycle & Storage / I/O detection
    let has_lifecycle = lower_name.starts_with("open")
        || lower_name.starts_with("close")
        || lower_name.starts_with("flush")
        || lower_name.starts_with("sync")
        || lower_name.starts_with("commit")
        || lower_name.starts_with("rollback")
        || lower_text.contains("fs::")
        || lower_text.contains("std::fs")
        || lower_text.contains("seek(")
        || lower_text.contains("reconcile");

    if has_lifecycle {
        for &t in LIFECYCLE_TAGS {
            tokens.insert(t.to_string());
        }
    }

    // 5. Compound identifier decomposition & acronym expansion
    for sub in split_identifier(symbol_name) {
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
    fn test_extract_semantic_tokens_error_and_auth() {
        let text = "fn validate_jwt(token: &str) -> Result<Claims, AuthError> { panic!(\"unimplemented\"); }";
        let tags = extract_semantic_tokens(text, "validate_jwt", "auth::service");
        assert!(tags.iter().any(|t| t == "__sem_error"));
        assert!(tags.iter().any(|t| t == "__sem_auth"));
        assert!(tags.iter().any(|t| t == "authentication"));
    }

    #[test]
    fn test_extract_semantic_tokens_endpoint() {
        let text = "@Get(\"/users\")\nasync def list_users(): return []";
        let tags = extract_semantic_tokens(text, "list_users", "controllers::users");
        assert!(tags.iter().any(|t| t == "__sem_endpoint"));
        assert!(tags.iter().any(|t| t == "__sem_api"));
    }
}
