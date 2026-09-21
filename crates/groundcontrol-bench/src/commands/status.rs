//! `status` benchmark command handler.

use std::path::PathBuf;

/// Handle the `status` subcommand: check index health and file counts.
pub fn handle_status(corpus: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let index_dir = corpus.join(".index");
    let db_path = index_dir.join("meta.db");
    if !db_path.exists() {
        eprintln!("Index database does not exist: {}", db_path.display());
        std::process::exit(1);
    }
    let store = match groundcontrol_core::persistence::Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open meta.db: {e}");
            std::process::exit(1);
        }
    };
    let files = match store.list_files() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to read files from meta.db: {e}");
            std::process::exit(1);
        }
    };
    if files.is_empty() {
        eprintln!("Index contains 0 documents");
        std::process::exit(1);
    }
    println!("Index healthy: {} documents indexed in {}", files.len(), corpus.display());
    Ok(())
}
