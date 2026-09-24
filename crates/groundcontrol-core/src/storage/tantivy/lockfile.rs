//! Stale lockfile healing for Tantivy index directories.

use std::path::Path;

/// Scan the Tantivy index directory for stale lockfiles (`.tantivy-*.lock`)
/// and clean them up if no active process holds an advisory lock on them.
pub fn heal_stale_lockfiles(index_path: &Path) {
    if !index_path.exists() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(index_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with(".tantivy-") && file_name.ends_with(".lock") {
                    if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path)
                    {
                        use fs4::fs_std::FileExt;
                        if file.try_lock_exclusive().is_ok() {
                            drop(file);
                            if let Err(e) = std::fs::remove_file(&path) {
                                tracing::debug!(
                                    "Failed to remove stale lockfile {}: {}",
                                    path.display(),
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "Removed stale Tantivy lockfile: {}",
                                    path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
