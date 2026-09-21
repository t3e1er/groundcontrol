//! Write tools: `write_note`, `delete_note`, `move_note`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use groundcontrol_common::config::CorpusMode;
use groundcontrol_common::{Error, Result};
use groundcontrol_core::engine::Engine;

#[derive(Debug, Deserialize)]
pub(crate) struct WriteNoteParams {
    pub path: String,
    pub content: String,
    pub mode: Option<String>,
    pub frontmatter: Option<Value>,
    pub template: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteNoteParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MoveNoteParams {
    pub from: String,
    pub to: String,
}

/// Build note content from optional frontmatter and body text.
fn build_note_content(frontmatter: Option<&Value>, template: Option<&str>, body: &str) -> String {
    let mut content = String::new();

    // Merge template into frontmatter if provided.
    let has_fm = frontmatter.is_some() || template.is_some();
    if has_fm {
        content.push_str("---\n");
        let mut fm_map = match frontmatter {
            Some(Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        };
        if let Some(tmpl) = template {
            let _ = fm_map.insert("template".to_string(), Value::String(tmpl.to_string()));
        }
        if let Ok(yaml) = serde_yaml::to_string(&Value::Object(fm_map)) {
            content.push_str(&yaml);
        }
        content.push_str("---\n\n");
    }

    content.push_str(body);
    if !body.ends_with('\n') {
        content.push('\n');
    }
    content
}

/// Write a note to disk (create, overwrite, append, or prepend) and index it.
pub fn handle_write_note(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: WriteNoteParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if engine.config().mode == CorpusMode::ReadOnly {
        return Err(Error::Config(format!("corpus '{}' is read-only", engine.config().name)));
    }

    let corpus_path = PathBuf::from(&engine.config().path);
    let full_path = corpus_path.join(&params.path);

    let classification = engine.classifier().classify(&full_path, None);
    let ext = full_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if let groundcontrol_core::index::classifier::FileClassification::Document(fmt) = classification
    {
        return Err(Error::NotPermitted(format!(
            "write_note cannot modify document format '{fmt}': documents are strictly read-only. Author markdown notes derived from them with 'derived_from' frontmatter."
        )));
    }
    if ext == "docx" || ext == "pdf" {
        return Err(Error::NotPermitted(
            "write_note cannot modify document format: documents are strictly read-only. Author markdown notes derived from them with 'derived_from' frontmatter.".to_string(),
        ));
    }

    let mode = params.mode.as_deref().unwrap_or("create");

    let new_content = match mode {
        "create" => {
            if full_path.exists() {
                return Err(Error::Config(format!("file already exists: {}", params.path)));
            }
            build_note_content(
                params.frontmatter.as_ref(),
                params.template.as_deref(),
                &params.content,
            )
        }
        "overwrite" => {
            if params.frontmatter.is_some() || params.template.is_some() {
                build_note_content(
                    params.frontmatter.as_ref(),
                    params.template.as_deref(),
                    &params.content,
                )
            } else {
                let mut s = params.content.clone();
                if !s.ends_with('\n') {
                    s.push('\n');
                }
                s
            }
        }
        "append" => {
            if !full_path.exists() {
                return Err(Error::NotFound(format!("file not found: {}", params.path)));
            }
            let mut existing = fs::read_to_string(&full_path).map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("cannot read {}: {}", params.path, e),
                ))
            })?;
            if !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(&params.content);
            if !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing
        }
        "prepend" => {
            if !full_path.exists() {
                return Err(Error::NotFound(format!("file not found: {}", params.path)));
            }
            let existing = fs::read_to_string(&full_path).map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("cannot read {}: {}", params.path, e),
                ))
            })?;
            let mut s = params.content.clone();
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&existing);
            s
        }
        other => {
            return Err(Error::Config(format!("unrecognized write mode: '{}'", other)));
        }
    };

    // Ensure parent directory exists.
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("cannot create directory for {}: {}", params.path, e),
            ))
        })?;
    }

    // Write file atomically-ish.
    fs::write(&full_path, &new_content).map_err(|e| {
        Error::Io(std::io::Error::new(e.kind(), format!("cannot write {}: {}", params.path, e)))
    })?;

    // Re-index.
    engine.index_file(&params.path, &new_content)?;
    engine.commit()?;

    debug!("Written note: {} (mode={})", params.path, mode);

    Ok(serde_json::json!({
        "path": params.path,
        "mode": mode,
        "written": true
    }))
}

/// Delete a note from disk and all indices.
pub fn handle_delete_note(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: DeleteNoteParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if engine.config().mode == CorpusMode::ReadOnly {
        return Err(Error::Config(format!("corpus '{}' is read-only", engine.config().name)));
    }

    let corpus_path = PathBuf::from(&engine.config().path);
    let full_path = corpus_path.join(&params.path);

    if !full_path.exists() {
        return Err(Error::NotFound(format!("file not found: {}", params.path)));
    }

    // Remove file from disk.
    fs::remove_file(&full_path).map_err(|e| {
        Error::Io(std::io::Error::new(e.kind(), format!("cannot delete {}: {}", params.path, e)))
    })?;

    // Remove from indices.
    engine.remove_file(&params.path)?;
    engine.commit()?;

    debug!("Deleted note: {}", params.path);

    Ok(serde_json::json!({
        "path": params.path,
        "deleted": true
    }))
}

/// Move/rename a note, updating wikilinks in other files.
pub fn handle_move_note(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: MoveNoteParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if engine.config().mode == CorpusMode::ReadOnly {
        return Err(Error::Config(format!("corpus '{}' is read-only", engine.config().name)));
    }

    let corpus_path = PathBuf::from(&engine.config().path);
    let from_full = corpus_path.join(&params.from);
    let to_full = corpus_path.join(&params.to);

    if !from_full.exists() {
        return Err(Error::NotFound(format!("source file not found: {}", params.from)));
    }

    if to_full.exists() {
        return Err(Error::Config(format!("destination already exists: {}", params.to)));
    }

    // Ensure destination parent directory exists.
    if let Some(parent) = to_full.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("cannot create directory for {}: {}", params.to, e),
            ))
        })?;
    }

    // Move the file.
    fs::rename(&from_full, &to_full).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("cannot move {} to {}: {}", params.from, params.to, e),
        ))
    })?;

    // Move derived projection file if it exists.
    let from_proj = engine.projection_path(&params.from);
    if from_proj.is_file() {
        let to_proj = engine.projection_path(&params.to);
        if let Some(parent) = to_proj.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::rename(&from_proj, &to_proj);
    }

    // Compute old and new note names (filename without extension) for wikilink rewriting.
    let old_name =
        Path::new(&params.from).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let new_name =
        Path::new(&params.to).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();

    // Rewrite wikilinks in other .md files if the note name changed.
    let mut links_rewritten: usize = 0;
    if old_name != new_name && !old_name.is_empty() {
        let old_link = format!("[[{}]]", old_name);
        let new_link = format!("[[{}]]", new_name);

        // Walk all .md files in corpus.
        let files = walk_markdown_files_for_rewrite(&corpus_path, engine.exclude_matcher())?;
        for (rel_path, file_path) in &files {
            // Skip the moved file itself.
            if *rel_path == params.to {
                continue;
            }

            let content = match fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if content.contains(&old_link) {
                let updated = content.replace(&old_link, &new_link);
                if let Err(e) = fs::write(file_path, &updated) {
                    debug!("Failed to rewrite links in {}: {}", rel_path, e);
                    continue;
                }
                // Re-index the modified file.
                engine.index_file(rel_path, &updated)?;
                links_rewritten += 1;
            }
        }
    }

    // Remove old path from engine.
    engine.remove_file(&params.from)?;

    // Index the file at the new path.
    let new_content = fs::read_to_string(&to_full).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("cannot read moved file {}: {}", params.to, e),
        ))
    })?;
    engine.index_file(&params.to, &new_content)?;
    engine.commit()?;

    debug!("Moved note: {} -> {} ({} links rewritten)", params.from, params.to, links_rewritten);

    Ok(serde_json::json!({
        "from": params.from,
        "to": params.to,
        "moved": true,
        "links_rewritten": links_rewritten
    }))
}

/// Walk .md files for wikilink rewriting (same as engine's internal walk but accessible here).
fn walk_markdown_files_for_rewrite(
    root: &Path,
    matcher: &groundcontrol_core::index::exclude::ExcludeMatcher,
) -> Result<Vec<(String, PathBuf)>> {
    let mut results = Vec::new();
    if !root.exists() {
        return Ok(results);
    }
    walk_dir_for_rewrite(root, root, matcher, &mut results)?;
    Ok(results)
}

fn walk_dir_for_rewrite(
    root: &Path,
    current: &Path,
    matcher: &groundcontrol_core::index::exclude::ExcludeMatcher,
    results: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let entries = fs::read_dir(current)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if matcher.is_excluded(&path, true) {
                continue;
            }
            walk_dir_for_rewrite(root, &path, matcher, results)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if matcher.is_excluded(&path, false) {
                continue;
            }
            let rel = path.strip_prefix(root).map_err(|e| {
                Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            })?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            results.push((rel_str, path.clone()));
        }
    }
    Ok(())
}
