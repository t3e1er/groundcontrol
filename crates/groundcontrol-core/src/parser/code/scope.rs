//! Scope path normalization utilities.

/// Normalize a scope path by stripping balanced angle brackets `<...>` and
/// lifetime annotations while preserving scope hierarchy separators (` > `).
///
/// Example: `EarlyBinder<'tcx, T> > instantiate` -> `EarlyBinder > instantiate`.
pub fn normalize_scope_path(scope: &str) -> String {
    let segments: Vec<String> = scope
        .split(" > ")
        .map(|segment| {
            let mut out = String::with_capacity(segment.len());
            let mut depth = 0usize;
            for ch in segment.chars() {
                match ch {
                    '<' => depth += 1,
                    '>' => {
                        if depth > 0 {
                            depth -= 1;
                        }
                    }
                    _ if depth == 0 => out.push(ch),
                    _ => {}
                }
            }
            let cleaned = strip_loose_lifetimes(&out);
            cleaned.trim().to_string()
        })
        .collect();

    segments.join(" > ")
}

/// Check whether a candidate scope path matches a query scope path, allowing
/// module path prefixes (e.g. `ty::EarlyBinder` matching `EarlyBinder`) and
/// hierarchy prefixes (e.g. `Outer > Inner > method` matching `Inner > method`).
pub fn scope_matches(norm_candidate: &str, norm_query: &str) -> bool {
    let q_segs: Vec<&str> = norm_query.split(" > ").map(str::trim).collect();
    let c_segs: Vec<&str> = norm_candidate.split(" > ").map(str::trim).collect();

    if q_segs.is_empty() || q_segs.len() > c_segs.len() {
        return false;
    }

    let offset = c_segs.len() - q_segs.len();
    for (i, &q_seg) in q_segs.iter().enumerate() {
        let c_seg = c_segs[offset + i];
        let matches = c_seg == q_seg
            || c_seg.ends_with(&format!("::{q_seg}"))
            || c_seg.ends_with(&format!(".{q_seg}"));
        if !matches {
            return false;
        }
    }
    true
}

fn strip_loose_lifetimes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if let Some(&next_ch) = chars.peek() {
                if next_ch.is_alphabetic() || next_ch == '_' {
                    while let Some(&ident_ch) = chars.peek() {
                        if ident_ch.is_alphanumeric() || ident_ch == '_' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    continue;
                }
            }
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_scope_path_basic() {
        assert_eq!(
            normalize_scope_path("EarlyBinder<'tcx, T> > instantiate"),
            "EarlyBinder > instantiate"
        );
    }

    #[test]
    fn test_normalize_scope_path_nested_generics() {
        assert_eq!(normalize_scope_path("Foo<Bar<T>> > baz"), "Foo > baz");
        assert_eq!(normalize_scope_path("Outer<A<B>, C<D<E>>> > method"), "Outer > method");
    }

    #[test]
    fn test_normalize_scope_path_lifetimes() {
        assert_eq!(normalize_scope_path("Closure<'a>"), "Closure");
        assert_eq!(normalize_scope_path("Ref<'a, 'b, T> > borrow"), "Ref > borrow");
    }

    #[test]
    fn test_normalize_scope_path_already_clean() {
        assert_eq!(normalize_scope_path("Router > dispatch"), "Router > dispatch");
        assert_eq!(normalize_scope_path("simple_fn"), "simple_fn");
    }

    #[test]
    fn test_normalize_scope_path_multi_level() {
        assert_eq!(
            normalize_scope_path("crate::module<T> > Struct<'a> > method<U>"),
            "crate::module > Struct > method"
        );
    }
}
