//! Configuration types for corpus, chunking, graph, and templates.
//!
//! These are deserialized from TOML files. Each corpus has its own config.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level corpus configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusConfig {
    /// Human-readable corpus name.
    pub name: String,
    /// Path to the markdown directory.
    pub path: String,
    /// Access mode for this corpus.
    #[serde(default)]
    pub mode: CorpusMode,
    /// Indexing mode: "full" (default) or "fast" (BM25+Graph only, no embedding).
    #[serde(default)]
    pub index_mode: IndexMode,
    /// Chunking strategy and parameters.
    #[serde(default)]
    pub chunking: ChunkingConfig,
    /// Embedding model configuration (alias: `[embedder]`).
    #[serde(default, alias = "embedder")]
    pub embedding: EmbeddingConfig,
    /// Graph edge type definitions.
    #[serde(default)]
    pub graph: GraphConfig,
    /// Path to templates directory (relative to corpus root).
    /// If unset (`None`), automatic candidate discovery probes:
    /// 1. `docs/.templates`
    /// 2. `.templates`
    /// 3. `.groundcontrol/templates`
    /// 4. `docs/templates`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub templates_dir: Option<String>,
    /// File exclusion configuration for file discovery and indexing.
    #[serde(default)]
    pub exclude: ExcludeConfig,
    /// Rich documentation promotion configuration.
    #[serde(default)]
    pub docs: DocsConfig,
}

impl Default for CorpusConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            path: ".".to_string(),
            mode: CorpusMode::default(),
            index_mode: IndexMode::default(),
            chunking: ChunkingConfig::default(),
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig::default(),
            templates_dir: None,
            exclude: ExcludeConfig::default(),
            docs: DocsConfig::default(),
        }
    }
}

/// Rich documentation promotion configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DocsConfig {
    /// Explicit glob patterns governing promotion of rich files to documentation (e.g. `["docs/**", "wiki/**"]`).
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// Configuration for file and directory exclusion during indexing and watching.
/// Uses gitignore-compatible glob pattern syntax.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludeConfig {
    /// Array of gitignore-style glob patterns for excluding paths from indexing.
    /// Defaults to standard VCS, build artifact, dependency, test, and binary patterns.
    #[serde(default = "default_exclude_patterns")]
    pub patterns: Vec<String>,
}

impl Default for ExcludeConfig {
    fn default() -> Self {
        Self { patterns: default_exclude_patterns() }
    }
}

impl ExcludeConfig {
    /// Import patterns from a `.gitignore` file, appending any new lines to `patterns`.
    pub fn from_gitignore(gitignore_path: &std::path::Path) -> Self {
        let mut cfg = Self::default();
        cfg.import_gitignore(gitignore_path);
        cfg
    }

    /// Parse lines from a `.gitignore` file and append non-duplicate patterns.
    pub fn import_gitignore(&mut self, gitignore_path: &std::path::Path) {
        if let Ok(content) = std::fs::read_to_string(gitignore_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    if !self.patterns.iter().any(|p| p == trimmed) {
                        self.patterns.push(trimmed.to_string());
                    }
                }
            }
        }
    }
}

/// Default indexing exclude patterns.
pub fn default_exclude_patterns() -> Vec<String> {
    vec![
        // VCS & Tool metadata
        ".git".to_string(),
        ".svn".to_string(),
        ".hg".to_string(),
        ".index/".to_string(),
        ".fastembed_cache/".to_string(),
        ".github/".to_string(),
        ".circleci/".to_string(),
        // Dependencies
        "node_modules/".to_string(),
        "vendor/".to_string(),
        "Pods/".to_string(),
        // Build artifacts & compiler output
        "target/".to_string(),
        "dist/".to_string(),
        "build/".to_string(),
        "out/".to_string(),
        "bin/".to_string(),
        "obj/".to_string(),
        // Virtualenvs & runtime caches
        ".venv/".to_string(),
        "venv/".to_string(),
        "env/".to_string(),
        "__pycache__/".to_string(),
        ".cache/".to_string(),
        ".mypy_cache/".to_string(),
        ".pytest_cache/".to_string(),
        ".ruff_cache/".to_string(),
        ".next/".to_string(),
        ".nuxt/".to_string(),
        ".turbo/".to_string(),
        // Test suites & fixture directories (avoids ingesting test suites/data by default)
        "tests/".to_string(),
        "test/".to_string(),
        "__tests__/".to_string(),
        "testdata/".to_string(),
        "fixtures/".to_string(),
        "spec/".to_string(),
        "specs/".to_string(),
        // Common test filename patterns
        "*.test.*".to_string(),
        "*.spec.*".to_string(),
        "*_test.go".to_string(),
        "*_test.py".to_string(),
        // Binaries, compiled objects & databases
        "*.exe".to_string(),
        "*.dll".to_string(),
        "*.so".to_string(),
        "*.dylib".to_string(),
        "*.bin".to_string(),
        "*.wasm".to_string(),
        "*.pyc".to_string(),
        "*.pyo".to_string(),
        "*.class".to_string(),
        "*.o".to_string(),
        "*.a".to_string(),
        "*.db".to_string(),
        "*.sqlite".to_string(),
        "*.sqlite3".to_string(),
        // Archives
        "*.zip".to_string(),
        "*.tar".to_string(),
        "*.gz".to_string(),
        "*.bz2".to_string(),
        "*.xz".to_string(),
        "*.7z".to_string(),
        // Package manager lockfiles
        "package-lock.json".to_string(),
        "pnpm-lock.yaml".to_string(),
        "yarn.lock".to_string(),
        "Cargo.lock".to_string(),
        "composer.lock".to_string(),
        "Gemfile.lock".to_string(),
        "poetry.lock".to_string(),
    ]
}

/// Indexing mode controlling which index backends are populated.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IndexMode {
    /// Full indexing: Dense Embeddings (Jina ONNX) for Docs; Binary Hamming for Code; BM25 + Graph for both (default).
    #[default]
    Full,
    /// Fast mode: Algorithmic Binary Hamming + BM25 + Graph across both Docs and Code (Zero ONNX inference).
    Fast,
}

/// Corpus access mode.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusMode {
    /// Full read and write access.
    #[default]
    ReadWrite,
    /// Search and read only — write tools are suppressed.
    ReadOnly,
}

/// Chunking strategy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Chunking strategy.
    #[serde(default = "default_strategy")]
    pub strategy: ChunkingStrategy,
    /// Target chunk size in tokens.
    #[serde(default = "default_target_tokens")]
    pub target_tokens: usize,
    /// Maximum chunk size in tokens.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Overlap between chunks in tokens.
    #[serde(default = "default_overlap")]
    pub overlap_tokens: usize,
    /// Never split across heading boundaries.
    #[serde(default = "default_true")]
    pub respect_headings: bool,
    /// Discard chunks smaller than this.
    #[serde(default = "default_min_tokens")]
    pub min_chunk_tokens: usize,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            strategy: ChunkingStrategy::Heading,
            target_tokens: 512,
            max_tokens: 1024,
            overlap_tokens: 64,
            respect_headings: true,
            min_chunk_tokens: 50,
        }
    }
}

fn default_strategy() -> ChunkingStrategy {
    ChunkingStrategy::Heading
}
fn default_target_tokens() -> usize {
    512
}
fn default_max_tokens() -> usize {
    1024
}
fn default_overlap() -> usize {
    64
}
fn default_min_tokens() -> usize {
    50
}
fn default_true() -> bool {
    true
}

/// Available chunking strategies.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChunkingStrategy {
    /// Fixed character count split.
    Fixed,
    /// Split at paragraph boundaries (double newlines), merge to target size.
    Paragraph,
    /// Line-based accumulation to target size (legacy default).
    Semantic,
    /// Each heading section is one chunk (best for documentation).
    #[default]
    Heading,
    /// Tree-sitter AST-guided syntactic node chunking (for polyglot source code).
    CodeAst,
}

/// Embedding model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// fastembed model identifier (e.g., "jinaai/jina-embeddings-v2-base-code").
    #[serde(default = "default_embedding_model")]
    pub model: String,
}

fn default_embedding_model() -> String {
    "jina-embeddings-v2-base-code-int8".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self { model: default_embedding_model() }
    }
}

/// Graph configuration: user-defined edge types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphConfig {
    /// Registered edge types for this corpus.
    #[serde(default)]
    pub edge_types: Vec<EdgeTypeConfig>,
}

/// Configuration for a single edge type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeTypeConfig {
    /// Name of the edge type (e.g., "ParentChild", "Supersedes").
    pub name: String,
    /// Source of this edge.
    pub source: EdgeSource,
    /// Weight applied to edges of this type.
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// Whether edges are created in both directions.
    #[serde(default)]
    pub bidirectional: bool,
    /// Frontmatter field name (required if source is "frontmatter").
    pub field: Option<String>,
    /// Direction for frontmatter-derived edges.
    pub direction: Option<EdgeDirection>,
    /// Maximum tag frequency (for tag-based edges).
    pub max_frequency: Option<usize>,
    /// Edge class: semantic (discovery), structural (navigation), or hybrid (both).
    /// If absent, inferred from source type.
    pub class: Option<EdgeClass>,
    /// Human-readable description of this edge relationship.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Allowed templates for source notes (optional constraint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_source_templates: Option<Vec<String>>,
    /// Allowed templates for target notes (optional constraint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_target_templates: Option<Vec<String>>,
}

fn default_weight() -> f32 {
    1.0
}

/// Where an edge comes from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeSource {
    /// Derived from `[[wikilinks]]` in content.
    Wikilink,
    /// Derived from shared `#tags`.
    Tag,
    /// Derived from a specific frontmatter field.
    Frontmatter,
    /// Derived from standard `[markdown](links)`.
    Reference,
    /// Derived from code AST analysis (e.g. calls, defines, imports, implements).
    Code,
}

/// Direction of a frontmatter-derived edge relative to the current note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeDirection {
    /// Edge points FROM this note TO the target.
    Outbound,
    /// Edge points FROM the target TO this note.
    Inbound,
}

fn default_edge_direction() -> EdgeDirection {
    EdgeDirection::Outbound
}

/// Declarative edge rule defined directly inside a markdown template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateEdgeSchema {
    /// Frontmatter field name containing the link target(s).
    pub field: String,
    /// Edge type name emitted in the knowledge graph (e.g. "Supersedes").
    #[serde(rename = "type")]
    pub edge_type: String,
    /// Edge class: structural, semantic, code, or crossmodal.
    #[serde(default)]
    pub class: EdgeClass,
    /// Traversal direction relative to this note: outbound or inbound.
    #[serde(default = "default_edge_direction")]
    pub direction: EdgeDirection,
    /// Whether to insert reverse edge automatically.
    #[serde(default)]
    pub bidirectional: bool,
    /// Target must follow a specific template (e.g. "adr").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_template: Option<String>,
    /// Target must be a specific kind (e.g. "code_symbol", "file", "doc").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    /// Whether this edge field is required in frontmatter.
    #[serde(default)]
    pub required: bool,
    /// Human-readable documentation for this edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl TemplateEdgeSchema {
    /// Convert this template edge declaration into an [`EdgeTypeConfig`] usable by the knowledge graph.
    pub fn to_edge_type_config(&self) -> EdgeTypeConfig {
        EdgeTypeConfig {
            name: self.edge_type.clone(),
            source: EdgeSource::Frontmatter,
            weight: 1.0,
            bidirectional: self.bidirectional,
            field: Some(self.field.clone()),
            direction: Some(self.direction.clone()),
            max_frequency: None,
            class: Some(self.class),
            description: self.description.clone(),
            allowed_source_templates: None,
            allowed_target_templates: self.target_template.as_ref().map(|t| vec![t.clone()]),
        }
    }
}

/// Classification of an edge's purpose in the knowledge graph.
/// Semantic edges support discovery/boosting; structural edges support markdown hierarchy;
/// code edges represent concrete AST relationships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EdgeClass {
    /// Discovery-oriented: shared tags, vector similarity, co-occurrence.
    /// Used by hybrid search for graph boosting, search_related, graph_communities.
    Semantic,
    /// Navigation-oriented: markdown wikilinks, frontmatter relationships, schema-declared links.
    /// Used by search_graph, graph_path, backlinks/forwardlinks.
    Structural,
    /// Code-oriented: AST relationships (calls, defines, implements, imports, type references).
    Code,
    /// Cross-modal bridge: documentation-to-code edges (documents, tested_by, specifies).
    CrossModal,
    /// Both purposes: intentional link that also signals topical proximity.
    #[default]
    Hybrid,
}

impl EdgeClass {
    /// Infer class from edge source when not explicitly configured.
    pub fn infer_from_source(source: &EdgeSource) -> Self {
        match source {
            EdgeSource::Tag => EdgeClass::Semantic,
            EdgeSource::Wikilink => EdgeClass::Structural,
            EdgeSource::Frontmatter => EdgeClass::Structural,
            EdgeSource::Reference => EdgeClass::Structural,
            EdgeSource::Code => EdgeClass::Code,
        }
    }

    /// Parse from a string (case-insensitive).
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "semantic" => Some(Self::Semantic),
            "structural" => Some(Self::Structural),
            "code" => Some(Self::Code),
            "crossmodal" | "cross-modal" => Some(Self::CrossModal),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }

    /// Check if this class matches a filter. Hybrid matches all filters.
    pub fn matches(&self, filter: EdgeClass) -> bool {
        match filter {
            EdgeClass::Hybrid => true, // Hybrid filter matches everything
            EdgeClass::Semantic => *self == EdgeClass::Semantic || *self == EdgeClass::Hybrid,
            EdgeClass::Structural => *self == EdgeClass::Structural || *self == EdgeClass::Hybrid,
            EdgeClass::Code => *self == EdgeClass::Code || *self == EdgeClass::Hybrid,
            EdgeClass::CrossModal => *self == EdgeClass::CrossModal || *self == EdgeClass::Hybrid,
        }
    }

    /// Return static str representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Structural => "structural",
            Self::Code => "code",
            Self::CrossModal => "crossmodal",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Central server / daemon runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    /// Bind address for HTTP MCP server.
    #[serde(default = "default_server_bind")]
    pub bind: String,
    /// Idle daemon timeout in minutes before graceful shutdown (0 = disabled).
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_mins: u64,
    /// Daemon log level.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Whether to auto-index new corpora on startup.
    #[serde(default = "default_true")]
    pub auto_index: bool,
    /// Default indexing mode for new corpora.
    #[serde(default)]
    pub index_mode: IndexMode,
}

fn default_server_bind() -> String {
    "127.0.0.1:9090".to_string()
}

fn default_idle_timeout() -> u64 {
    30
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_server_bind(),
            idle_timeout_mins: default_idle_timeout(),
            log_level: default_log_level(),
            auto_index: true,
            index_mode: IndexMode::Full,
        }
    }
}

/// GraphView 3D visualizer sidecar configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphviewConfig {
    /// Socket address to bind the GraphView dashboard to.
    #[serde(default = "default_graphview_bind")]
    pub bind: String,
    /// Upstream groundcontrol MCP daemon HTTP URL for live agent telemetry.
    #[serde(default = "default_upstream_daemon")]
    pub daemon: String,
    /// Dedicated authentication key for daemon-to-graphview relay.
    #[serde(default)]
    pub daemon_key: Option<String>,
}

fn default_graphview_bind() -> String {
    "127.0.0.1:9091".to_string()
}

fn default_upstream_daemon() -> String {
    "http://127.0.0.1:9090".to_string()
}

impl Default for GraphviewConfig {
    fn default() -> Self {
        Self { bind: default_graphview_bind(), daemon: default_upstream_daemon(), daemon_key: None }
    }
}

/// Registered corpus entry in central configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredCorpus {
    /// Filesystem path to the corpus repository.
    pub path: String,
    /// Optional indexing mode override ("full" or "fast").
    #[serde(default)]
    pub index_mode: Option<IndexMode>,
}

/// Central multi-corpus registry configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CorporaRegistry {
    /// Name of the default corpus to route to when omitted in tool calls.
    #[serde(default)]
    pub default: Option<String>,
    /// Map of corpus name -> configuration/path.
    #[serde(default)]
    pub registered: BTreeMap<String, RegisteredCorpus>,
}

/// Global groundcontrol client/daemon configuration (persisted at `${GROUNDCONTROL_CACHE_DIR}/config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GlobalConfig {
    /// Server / daemon settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// Authentication and client registry.
    #[serde(default)]
    pub auth: crate::client::ClientsRegistry,
    /// GraphView visualizer sidecar settings.
    #[serde(default)]
    pub graphview: GraphviewConfig,
    /// Persistent multi-corpus registry.
    #[serde(default)]
    pub corpora: CorporaRegistry,
    /// Custom cache directory override.
    #[serde(default)]
    pub cache_dir: Option<String>,
}

impl GlobalConfig {
    /// Shorthand to check if auto-index is enabled.
    pub fn auto_index(&self) -> bool {
        self.server.auto_index
    }

    /// Shorthand for default indexing mode.
    pub fn index_mode(&self) -> IndexMode {
        self.server.index_mode
    }

    /// Shorthand for idle timeout in minutes.
    pub fn idle_timeout_mins(&self) -> u64 {
        self.server.idle_timeout_mins
    }

    /// Shorthand for log level.
    pub fn log_level(&self) -> &str {
        &self.server.log_level
    }

    /// Shorthand for server bind address.
    pub fn bind(&self) -> &str {
        &self.server.bind
    }
}

/// Get the central cache directory `${GROUNDCONTROL_CACHE_DIR}`.
///
/// Precedence:
/// 1. `GROUNDCONTROL_CACHE_DIR` / `GC_CACHE_DIR` (or legacy `CTXV_CACHE_DIR`)
/// 2. Windows: `%LOCALAPPDATA%\groundcontrol\cache`
/// 3. Unix: `$XDG_CACHE_HOME/groundcontrol` or `~/.cache/groundcontrol`
pub fn get_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("GROUNDCONTROL_CACHE_DIR")
        .or_else(|_| std::env::var("GC_CACHE_DIR"))
        .or_else(|_| std::env::var("CTXV_CACHE_DIR"))
    {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data).join("groundcontrol").join("cache");
        }
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            return PathBuf::from(user_profile).join(".cache").join("groundcontrol");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            return PathBuf::from(xdg).join("groundcontrol");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".cache").join("groundcontrol");
        }
    }
    PathBuf::from(".cache").join("groundcontrol")
}

/// Directory where central multi-corpus indices are stored: `${CTXV_CACHE_DIR}/corpora`.
pub fn get_corpora_cache_dir() -> PathBuf {
    get_cache_dir().join("corpora")
}

/// Central index storage directory for a specific corpus: `${CTXV_CACHE_DIR}/corpora/<name>`.
pub fn get_corpus_index_dir(name: &str) -> PathBuf {
    get_corpora_cache_dir().join(name)
}

/// Directory where daemon logs are stored: `${CTXV_CACHE_DIR}/logs`.
pub fn get_logs_cache_dir() -> PathBuf {
    get_cache_dir().join("logs")
}

/// Path to global configuration file: `${CTXV_CACHE_DIR}/config.toml`.
pub fn get_config_path() -> PathBuf {
    get_cache_dir().join("config.toml")
}

/// Ensure global configuration exists at `${CTXV_CACHE_DIR}/config.toml`.
///
/// If the file does not exist, a fresh configuration is generated with
/// default server settings, generated client keys and `daemon_key`,
/// default GraphView settings, and an empty corpora registry, then saved to disk.
pub fn ensure_global_config() -> GlobalConfig {
    let path = get_config_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str(&content) {
                return cfg;
            }
        }
    }

    let auth = crate::client::generate_default_config();
    let graphview =
        GraphviewConfig { daemon_key: auth.daemon_key.clone(), ..GraphviewConfig::default() };
    let cfg = GlobalConfig {
        server: ServerConfig::default(),
        auth,
        graphview,
        corpora: CorporaRegistry::default(),
        cache_dir: None,
    };

    let _ = save_global_config(&cfg);
    cfg
}

/// Load global configuration or return defaults, lazily bootstrapping on first run.
pub fn load_global_config() -> GlobalConfig {
    ensure_global_config()
}

/// Save global configuration to `${CTXV_CACHE_DIR}/config.toml`.
pub fn save_global_config(cfg: &GlobalConfig) -> std::io::Result<()> {
    let path = get_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, content)
}

/// Storage footprint breakdown for a corpus in central cache (`${GROUNDCONTROL_CACHE_DIR}/corpora/<name>`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusDiskFootprint {
    /// SQLite metadata database size in bytes (`meta.db` + WAL/SHM).
    pub meta_db_bytes: u64,
    /// Tantivy inverted index directory size in bytes.
    pub tantivy_bytes: u64,
    /// Vector store file size in bytes (`vectors.json` or binary vectors).
    pub vectors_bytes: u64,
    /// Graph serialization file size in bytes if present (`graph.bin`).
    pub graph_bytes: u64,
    /// Total storage size in bytes across all index components.
    pub total_bytes: u64,
}

/// Calculate the disk usage breakdown for an indexed corpus in central cache.
pub fn calculate_corpus_disk_usage(name: &str) -> CorpusDiskFootprint {
    let index_dir = get_corpus_index_dir(name);
    let mut footprint = CorpusDiskFootprint::default();
    if !index_dir.exists() {
        return footprint;
    }

    // SQLite meta.db + WAL + SHM
    for file in &["meta.db", "meta.db-wal", "meta.db-shm"] {
        if let Ok(meta) = std::fs::metadata(index_dir.join(file)) {
            footprint.meta_db_bytes += meta.len();
        }
    }

    // Tantivy inverted index directory
    let tantivy_dir = index_dir.join("tantivy");
    if tantivy_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&tantivy_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        footprint.tantivy_bytes += meta.len();
                    }
                }
            }
        }
    }

    // Vectors
    for file in &["vectors.json", "vectors.bin"] {
        if let Ok(meta) = std::fs::metadata(index_dir.join(file)) {
            footprint.vectors_bytes += meta.len();
        }
    }

    // Graph
    if let Ok(meta) = std::fs::metadata(index_dir.join("graph.bin")) {
        footprint.graph_bytes += meta.len();
    }

    footprint.total_bytes = footprint.meta_db_bytes
        + footprint.tantivy_bytes
        + footprint.vectors_bytes
        + footprint.graph_bytes;

    footprint
}

/// Format raw byte count into a human-readable string (e.g. `4.2 MB`, `850 KB`).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Path to the daemon PID file: `${GROUNDCONTROL_CACHE_DIR}/daemon.pid`.
pub fn get_daemon_pid_path() -> PathBuf {
    get_cache_dir().join("daemon.pid")
}

/// Information recorded in the daemon PID file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonPidInfo {
    /// Process ID of the background daemon.
    pub pid: u32,
    /// Bound socket address (e.g. `127.0.0.1:9090`).
    pub bind: String,
    /// UNIX timestamp when daemon process was detached.
    pub started_at: u64,
    /// Optional daemon API token for authentication.
    #[serde(default)]
    pub token: Option<String>,
}

/// Read and parse active daemon PID file if present.
pub fn read_daemon_pid() -> Option<DaemonPidInfo> {
    let path = get_daemon_pid_path();
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write daemon PID file to disk upon daemon detachment.
pub fn write_daemon_pid(info: &DaemonPidInfo) -> std::io::Result<()> {
    let path = get_daemon_pid_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(info)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, content)
}

/// Remove daemon PID file upon graceful daemon shutdown.
pub fn remove_daemon_pid() -> std::io::Result<()> {
    let path = get_daemon_pid_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_corpus_config() {
        let toml_str = r#"
            name = "test-wiki"
            path = "./wiki"
        "#;
        let config: CorpusConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "test-wiki");
        assert_eq!(config.mode, CorpusMode::ReadWrite);
        assert_eq!(config.chunking.target_tokens, 512);
    }

    #[test]
    fn parse_full_corpus_config() {
        let toml_str = r#"
            name = "engineering"
            path = "./docs"
            mode = "read-only"
            templates_dir = ".schemas"

            [chunking]
            strategy = "heading"
            target_tokens = 1024
            max_tokens = 2048

            [embedding]
            model = "BAAI/bge-small-en-v1.5"

            [[graph.edge_types]]
            name = "Wikilink"
            source = "wikilink"
            weight = 1.0

            [[graph.edge_types]]
            name = "Implements"
            source = "frontmatter"
            field = "implements"
            weight = 0.8
            direction = "outbound"
        "#;
        let config: CorpusConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.mode, CorpusMode::ReadOnly);
        assert_eq!(config.chunking.strategy, ChunkingStrategy::Heading);
        assert_eq!(config.embedding.model, "BAAI/bge-small-en-v1.5");
        assert_eq!(config.graph.edge_types.len(), 2);
        assert_eq!(config.graph.edge_types[1].name, "Implements");
        assert_eq!(config.templates_dir, Some(".schemas".to_string()));
    }

    #[test]
    fn parse_corpus_config_embedder_alias_and_default() {
        // Test omitting [embedding] completely inherits global default
        let toml_minimal = r#"
            name = "default-embedder"
            path = "./vault"
        "#;
        let config_min: CorpusConfig = toml::from_str(toml_minimal).unwrap();
        assert_eq!(config_min.templates_dir, None);
        assert_eq!(config_min.embedding.model, "jina-embeddings-v2-base-code-int8");

        // Test using [embedder] instead of [embedding]
        let toml_alias = r#"
            name = "alias-embedder"
            path = "./vault"

            [embedder]
            model = "custom-model"
        "#;
        let config_alias: CorpusConfig = toml::from_str(toml_alias).unwrap();
        assert_eq!(config_alias.embedding.model, "custom-model");
    }

    #[test]
    fn parse_corpus_config_index_mode() {
        let toml_fast = r#"
            name = "fast-corpus"
            path = "./src"
            index_mode = "fast"
        "#;
        let config_fast: CorpusConfig = toml::from_str(toml_fast).unwrap();
        assert_eq!(config_fast.index_mode, IndexMode::Fast);

        let toml_full = r#"
            name = "full-corpus"
            path = "./src"
            index_mode = "full"
        "#;
        let config_full: CorpusConfig = toml::from_str(toml_full).unwrap();
        assert_eq!(config_full.index_mode, IndexMode::Full);

        let toml_default = r#"
            name = "default-corpus"
            path = "./src"
        "#;
        let config_def: CorpusConfig = toml::from_str(toml_default).unwrap();
        assert_eq!(config_def.index_mode, IndexMode::Full);
    }

    #[test]
    fn parse_corpus_config_with_exclude() {
        let toml_str = r#"
            name = "exclude-test"
            path = "./repo"

            [exclude]
            patterns = ["custom_dir/**", "*.custom"]
        "#;
        let config: CorpusConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.exclude.patterns, vec!["custom_dir/**", "*.custom"]);
    }

    #[test]
    fn corpus_config_default_exclude() {
        let toml_str = r#"
            name = "default-exclude"
            path = "./repo"
        "#;
        let config: CorpusConfig = toml::from_str(toml_str).unwrap();
        assert!(config.exclude.patterns.contains(&"tests/".to_string()));
        assert!(config.exclude.patterns.contains(&"node_modules/".to_string()));
        assert!(config.exclude.patterns.contains(&"target/".to_string()));
    }

    #[test]
    fn test_ensure_global_config_bootstraps_keys() {
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("CTXV_CACHE_DIR", temp.path());

        let cfg = ensure_global_config();
        assert_eq!(cfg.server.bind, "127.0.0.1:9090");
        assert!(cfg.auth.daemon_key.is_some());
        assert_eq!(cfg.auth.daemon_key, cfg.graphview.daemon_key);
        assert!(!cfg.auth.clients.is_empty());
        assert!(cfg.auth.clients.iter().any(|c| c.id == "antigravity" && c.key.is_some()));

        let cfg_path = get_config_path();
        assert!(cfg_path.exists());

        // Verify re-loading loads the exact same config from disk
        let loaded = load_global_config();
        assert_eq!(loaded.auth.daemon_key, cfg.auth.daemon_key);
    }
}
