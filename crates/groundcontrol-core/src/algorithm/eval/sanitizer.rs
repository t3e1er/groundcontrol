//! Query sanitization and Lucene syntax escaping for search queries.

/// Sanitize a raw natural language or code query to eliminate Lucene/Tantivy syntax errors.
pub fn sanitize_lucene_query(raw: &str) -> String {
    let mut cleaned_lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            continue;
        }
        cleaned_lines.push(line);
    }

    let joined = cleaned_lines.join(" ");

    let mut sanitized = String::with_capacity(joined.len());
    for ch in joined.chars() {
        match ch {
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
