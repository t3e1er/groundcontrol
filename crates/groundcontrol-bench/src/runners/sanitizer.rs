//! Query sanitization and Lucene syntax escaping for Tantivy search queries.

/// Sanitize a raw natural language or code query to eliminate Lucene/Tantivy syntax errors.
///
/// Handles:
/// - Markdown code fences (` ``` `, ` ~~~ `)
/// - Inline backticks (``` `foo` ```)
/// - Tantivy reserved syntax characters (`+`, `-`, `&`, `|`, `!`, `(`, `)`, `{`, `}`, `[`, `]`, `^`, `"`, `~`, `*`, `?`, `:`, `\`, `/`, `>`, `<`, `=`)
/// - Isolated boolean operators (`AND`, `OR`, `NOT` normalized to lowercase)
/// - Collapsing consecutive whitespace
pub fn sanitize_lucene_query(raw: &str) -> String {
    let mut cleaned_lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        // Skip markdown code fence lines
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            continue;
        }
        cleaned_lines.push(line);
    }

    let joined = cleaned_lines.join(" ");

    // Replace Tantivy special syntax characters and punctuation with space
    let mut sanitized = String::with_capacity(joined.len());
    for ch in joined.chars() {
        match ch {
            // Reserved Lucene/Tantivy syntax characters
            '+' | '-' | '&' | '|' | '!' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '"' | '~'
            | '*' | '?' | ':' | '\\' | '/' | '>' | '<' | '=' | '`' | ';' | '@' | '#' | '$'
            | '%' | ',' => {
                sanitized.push(' ');
            }
            other => {
                sanitized.push(other);
            }
        }
    }

    // Tokenize, normalize isolated AND/OR/NOT, and collapse whitespace
    let mut tokens = Vec::new();
    for token in sanitized.split_whitespace() {
        match token {
            "AND" => tokens.push("and"),
            "OR" => tokens.push("or"),
            "NOT" => tokens.push("not"),
            other => tokens.push(other),
        }
    }

    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_markdown_fences() {
        let raw = "Fix issue in handler:\n```python\nx = [1, 2]\nprint(x)\n```\nExpected output.";
        let clean = sanitize_lucene_query(raw);
        assert!(!clean.contains("```"));
        assert!(!clean.contains('['));
        assert!(!clean.contains(']'));
        assert!(clean.contains("Fix issue in handler"));
        assert!(clean.contains("x 1 2"));
    }

    #[test]
    fn test_sanitize_lucene_operators() {
        let raw = ">>> def test(): & 1 | 2; KeyError: 'foo[bar]'";
        let clean = sanitize_lucene_query(raw);
        assert!(!clean.contains(">>>"));
        assert!(!clean.contains('&'));
        assert!(!clean.contains('|'));
        assert!(!clean.contains(':'));
        assert!(!clean.contains('['));
        assert!(!clean.contains(']'));
        assert!(!clean.contains(';'));
        assert!(clean.contains("KeyError"));
        assert!(clean.contains("foo bar"));
    }

    #[test]
    fn test_sanitize_boolean_keywords() {
        let raw = "AND query OR something NOT else";
        let clean = sanitize_lucene_query(raw);
        assert_eq!(clean, "and query or something not else");
    }

    #[test]
    fn test_empty_and_whitespace() {
        assert_eq!(sanitize_lucene_query(""), "");
        assert_eq!(sanitize_lucene_query("   >>>   & | + -   "), "");
    }
}
