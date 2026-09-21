//! Color assignment, entity label normalization, and path hashing for 3D GraphView.

use std::collections::HashMap;

/// Extract a 2-3 component directory cluster key from a relative path.
pub fn extract_directory_key(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 1 {
        return "root".to_string();
    }
    let dir_parts = &parts[..parts.len() - 1];
    let take_count = dir_parts.len().min(3);
    dir_parts[..take_count].join("/")
}

/// FNV-1a 32-bit hash for cluster strings.
pub fn fnv1a_hash(s: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in s.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// Map entity type name to a distinct cyber-aesthetic neon color.
pub fn color_for_entity_type(entity_type: &str, community: u32) -> u32 {
    match entity_type.to_lowercase().as_str() {
        "docnode" | "doc" | "document" | "markdown" => 0x3b82f6, // Sapphire Blue
        "function" | "method" => 0x10b981,                       // Emerald Neon
        "struct" | "class" => 0x8b5cf6,                          // Electric Purple
        "trait" | "interface" => 0xec4899,                       // Hot Pink
        "enum" | "typealias" | "type" => 0xf59e0b,               // Amber
        "module" | "file" | "package" => 0x06b6d4,               // Cyan
        "constant" | "macro" => 0x14b8a6,                        // Teal
        _ => {
            let palette = [
                0x3b82f6, 0x10b981, 0x8b5cf6, 0xf59e0b, 0xec4899, 0x06b6d4, 0x14b8a6, 0x6366f1,
                0xe11d48, 0x84cc16,
            ];
            palette[(community as usize) % palette.len()]
        }
    }
}

/// Normalize raw AST entity type strings into canonical PascalCase labels.
pub fn normalize_type_label(t: &str) -> String {
    match t.to_lowercase().as_str() {
        "function" | "fn" => "Function".to_string(),
        "method" => "Method".to_string(),
        "struct" => "Struct".to_string(),
        "trait" => "Trait".to_string(),
        "class" => "Class".to_string(),
        "interface" => "Interface".to_string(),
        "enum" => "Enum".to_string(),
        "typealias" | "type_alias" | "type" => "TypeAlias".to_string(),
        "module" | "namespace" => "Module".to_string(),
        "file" => "File".to_string(),
        "package" => "Package".to_string(),
        "constant" | "const" => "Constant".to_string(),
        "macro" => "Macro".to_string(),
        "docnode" | "doc" | "document" | "markdown" | "adr" => "DocNode".to_string(),
        other => {
            let mut c = other.chars();
            match c.next() {
                None => "CodeSymbol".to_string(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }
    }
}

/// Assign distinct aesthetic neon colors based on AST entity type, path heuristics, and community.
pub fn assign_color_and_type(
    path: &str,
    community: u32,
    ast_types: Option<&HashMap<String, String>>,
) -> (String, u32) {
    let clean_path = path.replace('\\', "/");

    // 1. Authoritative AST metadata lookup from meta.db
    if let Some(types) = ast_types {
        if let Some(sym_type) = types.get(path).or_else(|| types.get(&clean_path)) {
            let color = color_for_entity_type(sym_type, community);
            return (normalize_type_label(sym_type), color);
        }
        if let Some(sub) = clean_path.split('#').nth(1) {
            if let Some(sym_type) = types.get(sub) {
                let color = color_for_entity_type(sym_type, community);
                return (normalize_type_label(sym_type), color);
            }
            if let Some(leaf) = sub.split("::").last() {
                if let Some(sym_type) = types.get(leaf) {
                    let color = color_for_entity_type(sym_type, community);
                    return (normalize_type_label(sym_type), color);
                }
            }
        }
        if let Some(sub) = clean_path.split("::").last() {
            if let Some(sym_type) = types.get(sub) {
                let color = color_for_entity_type(sym_type, community);
                return (normalize_type_label(sym_type), color);
            }
        }
        let base_name = clean_path.split('/').next_back().unwrap_or(path);
        if let Some(sym_type) = types.get(base_name) {
            let color = color_for_entity_type(sym_type, community);
            return (normalize_type_label(sym_type), color);
        }
    }

    // 2. Structural file & symbol heuristics
    let leaf = clean_path.split('#').nth(1).unwrap_or(&clean_path);
    let sym_leaf = leaf.split("::").last().unwrap_or(leaf);

    if clean_path.ends_with(".md") || clean_path.contains("docs/") || clean_path.contains("adr/") {
        ("DocNode".to_string(), 0x3b82f6) // Sapphire Blue
    } else if !clean_path.contains('#')
        && (clean_path.ends_with(".rs")
            || clean_path.ends_with(".ts")
            || clean_path.ends_with(".js")
            || clean_path.ends_with(".py")
            || clean_path.ends_with(".go"))
    {
        ("Module".to_string(), 0x06b6d4) // Cyan
    } else if sym_leaf.ends_with('!') || sym_leaf.starts_with("macro_") {
        ("Macro".to_string(), 0x14b8a6) // Teal
    } else if sym_leaf.starts_with("trait ")
        || sym_leaf.ends_with("Trait")
        || sym_leaf.ends_with("Ext")
    {
        ("Trait".to_string(), 0xec4899) // Hot Pink
    } else if sym_leaf.starts_with("enum ")
        || sym_leaf.ends_with("Enum")
        || sym_leaf.ends_with("Kind")
    {
        ("Enum".to_string(), 0xf59e0b) // Amber
    } else if sym_leaf.starts_with("struct ")
        || sym_leaf.starts_with("class ")
        || sym_leaf.starts_with("interface ")
        || sym_leaf.chars().next().map_or(false, |c| c.is_uppercase())
    {
        ("Struct".to_string(), 0x8b5cf6) // Electric Purple
    } else if sym_leaf.contains("::fn ")
        || sym_leaf.contains("()")
        || sym_leaf.ends_with(".rs#")
        || sym_leaf.chars().next().map_or(false, |c| c.is_lowercase())
    {
        ("Function".to_string(), 0x10b981) // Emerald Neon
    } else {
        // Community-based gradient fallback
        let palette = [
            0x3b82f6, 0x10b981, 0x8b5cf6, 0xf59e0b, 0xec4899, 0x06b6d4, 0x14b8a6, 0x6366f1,
            0xe11d48, 0x84cc16,
        ];
        let color = palette[(community as usize) % palette.len()];
        ("CodeSymbol".to_string(), color)
    }
}
