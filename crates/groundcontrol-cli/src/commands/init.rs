//! `init` subcommand handler.

use std::path::PathBuf;

/// Execute the `init` command: initialize a new repository configuration (`groundcontrol.toml`).
pub fn handle_init(path: Option<PathBuf>, force: bool) -> anyhow::Result<()> {
    let target_dir = match path {
        Some(p) => {
            let _ = std::fs::create_dir_all(&p);
            p.canonicalize().unwrap_or(p)
        }
        None => std::env::current_dir()?,
    };
    let config_file = target_dir.join("groundcontrol.toml");
    if config_file.exists() && !force {
        eprintln!("[-] '{}' already exists. Use --force to overwrite.", config_file.display());
        return Ok(());
    }

    let repo_name = target_dir.file_name().and_then(|n| n.to_str()).unwrap_or("repo").to_string();

    let mut exclude = groundcontrol_common::config::ExcludeConfig::default();
    let gitignore_path = target_dir.join(".gitignore");
    if gitignore_path.exists() {
        exclude.import_gitignore(&gitignore_path);
        println!("[+] Imported patterns from '{}'", gitignore_path.display());
    }

    let corpus_config = groundcontrol_common::config::CorpusConfig {
        name: repo_name.clone(),
        path: ".".to_string(),
        mode: groundcontrol_common::config::CorpusMode::ReadWrite,
        index_mode: groundcontrol_common::config::IndexMode::Full,
        chunking: groundcontrol_common::config::ChunkingConfig::default(),
        embedding: groundcontrol_common::config::EmbeddingConfig::default(),
        graph: groundcontrol_common::config::GraphConfig::default(),
        templates_dir: Some("docs/.templates".to_string()),
        exclude,
        docs: groundcontrol_common::config::DocsConfig {
            patterns: vec![
                "docs/**".to_string(),
                "wiki/**".to_string(),
                "architecture/**".to_string(),
            ],
        },
    };

    let toml_str = toml::to_string_pretty(&corpus_config)?;
    std::fs::write(&config_file, toml_str)?;

    let mut global = groundcontrol_common::config::load_global_config();
    let abs_path = target_dir
        .canonicalize()
        .unwrap_or_else(|_| target_dir.clone())
        .to_string_lossy()
        .to_string();
    global.corpora.registered.insert(
        repo_name.clone(),
        groundcontrol_common::config::RegisteredCorpus {
            path: abs_path,
            index_mode: Some(corpus_config.index_mode),
        },
    );
    if global.corpora.default.is_none() {
        global.corpora.default = Some(repo_name.clone());
    }
    let _ = groundcontrol_common::config::save_global_config(&global);

    println!("[+] Initialized repository configuration at '{}'", config_file.display());
    println!("    Corpus name: {}", repo_name);
    println!("    Docs patterns: docs/**, wiki/**, architecture/**");
    println!("    Exclusion rules count: {}", corpus_config.exclude.patterns.len());
    Ok(())
}
