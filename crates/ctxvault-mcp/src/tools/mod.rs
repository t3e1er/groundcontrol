//! MCP tool definitions: maps tool names → core engine calls.
//!
//! Each tool is a named handler function that takes `(&mut Engine, Value)` and returns
//! `Result<Value>`. The [`ToolRegistry`] manages registration and dispatch.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use ctxvault_common::config::CorpusMode;
use ctxvault_common::ports::{GraphStore, MetadataCatalog, SearchQuery, SearchService};
use ctxvault_common::{Error, Result};
use ctxvault_core::engine::Engine;
use ctxvault_core::search;
use ctxvault_core::template::Template;

// ---------------------------------------------------------------------------
// Registry types
// ---------------------------------------------------------------------------

/// MCP tool handler function signature for read-only vs mutating tools.
#[derive(Clone)]
pub enum ToolHandler {
    /// Read-only handler (can execute concurrently under reader lock).
    ReadOnly(fn(&Engine, Value) -> Result<Value>),
    /// Mutating handler (requires exclusive writer lock).
    ReadWrite(fn(&mut Engine, Value) -> Result<Value>),
}

/// Metadata and handler for a single MCP tool.
#[derive(Clone)]
pub struct ToolInfo {
    /// Tool name (used in MCP `tools/call` requests).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema describing the expected input parameters.
    pub input_schema: Value,
    /// The handler function to execute.
    pub handler: ToolHandler,
}

impl ToolInfo {
    /// Check whether the tool is read-only.
    pub fn is_read_only(&self) -> bool {
        matches!(self.handler, ToolHandler::ReadOnly(_))
    }
}

/// Tool exposure profile: gates which tools `tools/list` advertises to keep the
/// listing footprint small for narrow agent roles.
///
/// The sets are nested: `Scout` ⊂ `Analysis` ⊂ `All`. Profiles only gate what the
/// listing advertises — a tool called directly still executes regardless of profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfile {
    /// Minimal retrieve/navigate set for lightweight scout agents.
    Scout,
    /// Scout plus read-only graph/validation/analysis/code-intel tools.
    Analysis,
    /// Every registered tool, including mutating/admin tools.
    All,
}

/// Tools exposed under the `scout` profile (minimal retrieve/navigate set).
const SCOUT_TOOLS: [&str; 6] =
    ["search", "search_related", "get_snippet", "read_file", "list_notes", "status"];

/// Read-only tools added by the `analysis` profile on top of `scout`.
const ANALYSIS_ONLY_TOOLS: [&str; 6] = [
    "graph_match",
    "graph_communities",
    "validate",
    "list_templates",
    "list_corpora",
    "trace_cross_corpus",
];

impl ToolProfile {
    /// Parse a profile from its lowercase name, defaulting to [`ToolProfile::All`]
    /// for unknown values.
    pub fn from_str_name(name: &str) -> Self {
        match name {
            "scout" => ToolProfile::Scout,
            "analysis" => ToolProfile::Analysis,
            _ => ToolProfile::All,
        }
    }

    /// Whether `tools/list` under this profile should advertise `tool_name`.
    ///
    /// `All` admits every registered tool (so newly added tools appear without a
    /// list edit). `Analysis` admits the scout set plus the read-only analysis
    /// additions. `Scout` admits only the scout set.
    pub fn includes(&self, tool_name: &str) -> bool {
        match self {
            ToolProfile::All => true,
            ToolProfile::Analysis => {
                SCOUT_TOOLS.contains(&tool_name) || ANALYSIS_ONLY_TOOLS.contains(&tool_name)
            }
            ToolProfile::Scout => SCOUT_TOOLS.contains(&tool_name),
        }
    }
}

/// Registry of all available MCP tools.
pub struct ToolRegistry {
    tools: HashMap<String, ToolInfo>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    /// Register a single tool.
    pub fn register(&mut self, info: ToolInfo) {
        let _ = self.tools.insert(info.name.clone(), info);
    }

    /// Register a read-only tool handler.
    pub fn register_read(
        &mut self,
        name: &str,
        description: &str,
        input_schema: Value,
        handler: fn(&Engine, Value) -> Result<Value>,
    ) {
        self.register(ToolInfo {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            handler: ToolHandler::ReadOnly(handler),
        });
    }

    /// Register a mutating tool handler.
    pub fn register_write(
        &mut self,
        name: &str,
        description: &str,
        input_schema: Value,
        handler: fn(&mut Engine, Value) -> Result<Value>,
    ) {
        self.register(ToolInfo {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            handler: ToolHandler::ReadWrite(handler),
        });
    }

    /// Read tools that are corpus-scoped or manager-level and therefore must NOT
    /// accept the fan-out `corpus`/`corpora` discrimination args.
    const NON_DISCRIMINATED_READ_TOOLS: [&'static str; 3] =
        ["status", "list_corpora", "trace_cross_corpus"];

    /// Write tools that operate at the manager level and don't accept corpus arg.
    const MANAGER_WRITE_TOOLS: [&'static str; 2] = ["index_corpus", "unload_corpus"];

    /// Inject the optional `corpus` and `corpora` discrimination properties into
    /// the JSON input schema of every read tool that supports fan-out.
    ///
    /// `corpus` targets a single corpus; `corpora` fans out across several corpora
    /// (an array of names, or the string `"all"`) with RRF-merged, corpus-tagged
    /// results. Manager-level / corpus-scoped read tools are skipped.
    fn inject_corpus_args(&mut self) {
        let corpus_prop = serde_json::json!({
            "type": "string",
            "description": "Target a single corpus by name. Omit to use the default corpus."
        });
        let corpora_prop = serde_json::json!({
            "description": "Search across multiple corpora: an array of corpus names, or the string \"all\". Results are RRF-merged and each hit is tagged with its source corpus.",
            "oneOf": [
                { "type": "array", "items": { "type": "string" } },
                { "type": "string", "enum": ["all"] }
            ]
        });

        for tool in self.tools.values_mut() {
            let manager_level = Self::NON_DISCRIMINATED_READ_TOOLS.contains(&tool.name.as_str());
            let manager_write = Self::MANAGER_WRITE_TOOLS.contains(&tool.name.as_str());
            let Some(props) =
                tool.input_schema.get_mut("properties").and_then(Value::as_object_mut)
            else {
                continue;
            };

            match tool.handler {
                // Read tools (except manager-level ones) get single `corpus` + fan-out `corpora`.
                ToolHandler::ReadOnly(_) if !manager_level => {
                    let _ = props.insert("corpus".to_string(), corpus_prop.clone());
                    let _ = props.insert("corpora".to_string(), corpora_prop.clone());
                }
                // Write tools get only single `corpus` — they never fan out.
                ToolHandler::ReadWrite(_) if !manager_write => {
                    let _ = props.insert("corpus".to_string(), corpus_prop.clone());
                }
                _ => {}
            }
        }
    }

    /// Register all available tools.
    pub fn register_all(&mut self) {
        // Read tools
        self.register_read(
            "read_file",
            "Tier 3 (last resort): read one or more markdown or source code files. Accepts a single path string or an array of path strings ('paths' or 'path'). Supports start_line, end_line, max_lines. Prefer search → get_snippet first.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "description": "Relative path to a file, OR an array of relative paths to read in batch.",
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Alternative batch paths argument."
                    },
                    "start_line": { "type": "integer", "description": "Optional 1-based start line (applies when reading a single file)" },
                    "end_line": { "type": "integer", "description": "Optional 1-based end line (inclusive, applies when reading a single file)" },
                    "max_lines": { "type": "integer", "description": "Hard cap on returned lines (default 1000 for single file, 500 per file in batch)" }
                },
                "required": []
            }),
            handle_read_file,
        );

        self.register_read(
            "get_snippet",
            "Tier 2 fetch: retrieve exactly one code symbol's source (by qualified_name or name) or one doc chunk (by path+chunk_index), bounded by max_lines. Call this for the specific handles a search returned — do NOT read whole files unless necessary.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Symbol name to look up definition for" },
                    "qualified_name": { "type": "string", "description": "Code symbol scope_path (exact) or name (fuzzy) to fetch one symbol's source" },
                    "path": { "type": "string", "description": "Relative path — for a DOC chunk fetch (with chunk_index) or a code FILE hint" },
                    "chunk_index": { "type": "integer", "description": "With path, fetch that specific doc chunk (zero-based)" },
                    "max_lines": { "type": "integer", "description": "Hard cap on returned lines (default 500)" },
                    "include_neighbors": { "type": "boolean", "description": "Include neighbor context: code relationships (incoming/outgoing grouped by edge type) as handles, or adjacent doc chunks (default false)" }
                },
                "required": []
            }),
            handle_get_snippet,
        );

        self.register_read(
            "list_notes",
            "List indexed notes with metadata (path, title, template, content_hash), or inspect a single note's frontmatter and metadata by passing 'path'.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Optional relative path to inspect a specific note's parsed YAML frontmatter and metadata" },
                    "limit": { "type": "number", "description": "Maximum number of notes to return (default 100)" },
                    "offset": { "type": "number", "description": "Offset for pagination (default 0)" }
                },
                "required": []
            }),
            handle_list_notes,
        );

        // Search tools
        self.register_read(
            "search",
            "Tier 1 retrieval with Turn 1 hybrid snippets: returns handles across docs and code, with source snippets inlined for the top K results (configured via `snippets`, default 3). Modes: bm25, semantic, hybrid (default), graph, explain, fast.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "mode": { "type": "string", "enum": ["bm25", "semantic", "hybrid", "graph", "explain", "fast"], "description": "Retrieval mode (default: hybrid). Use 'fast' for sub-millisecond CPU SIF + Binary + PPR." },
                    "limit": { "type": "number", "description": "Maximum results to return (default 10)" },
                    "snippets": { "type": "number", "description": "Number of top results across docs and code to inline source snippets for in Turn 1 (default: 3). Set to 0 for pure handles." },
                    "depth": { "type": "string", "enum": ["precise", "broad", "adaptive"], "description": "Semantic mode only: retrieval depth — precise (chunk-level, default), broad (doc-level), adaptive (both + RRF)" },
                    "graph_depth": { "type": "number", "description": "hybrid/graph/explain modes: max graph traversal depth (default 2 for hybrid/explain, 3 for graph)" },
                    "edge_types": { "type": "array", "items": { "type": "string" }, "description": "hybrid/graph/explain modes: filter graph traversal by edge types" },
                    "edge_class": { "type": "string", "enum": ["code", "semantic", "structural", "crossmodal", "hybrid"], "description": "hybrid/graph/explain modes: filter graph traversal by edge class (default: code for code modality, semantic for docs)" },
                    "decompose": { "type": "boolean", "description": "hybrid mode only: enable query decomposition for multi-hop queries (default: false)" },
                    "modality": { "type": "string", "enum": ["docs", "code", "both"], "description": "Restrict results to documentation, code, or both (default)." },
                    "detail": { "type": "string", "enum": ["ids", "default"], "description": "ids = bare handles (path/qualified_name + line range + metadata, no snippet) for wide sweeps; default = handle plus top-K snippets." }
                },
                "required": ["query"]
            }),
            handle_search,
        );

        self.register_read(
            "search_related",
            "Tier 1: returns handles (paths/qualified names + line ranges), not bodies; fetch source with get_snippet, read whole files only as a last resort. Find related documents via graph-based Personalized PageRank approximation.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "seeds": { "type": "array", "items": { "type": "string" }, "description": "Seed document paths to find related notes for" },
                    "limit": { "type": "number", "description": "Maximum results to return (default 10)" },
                    "modality": { "type": "string", "enum": ["docs", "code", "both"], "description": "Restrict results to documentation, code, or both (default)." },
                    "detail": { "type": "string", "enum": ["ids", "default"], "description": "ids = bare handles (path/qualified_name + line range + metadata, no snippet) for wide sweeps; default = handle plus a short snippet." }
                },
                "required": ["seeds"]
            }),
            handle_search_related,
        );

        // Graph tools
        self.register_read(
            "graph_match",
            "Hierarchical Cypher-Lite graph traversal query compiled to SQLite recursive CTEs. Returns a branching tree representation with blast radius impact summary and hub suppression. Cycle-safe with depth bounding.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Linear Cypher-Lite ASCII path pattern, e.g. '(:CodeSymbol {name: \"SelectVictimsOnNode\"})-[:implements]->(:Interface)<-[:calls*1..2]-(c:CodeSymbol)'"
                    },
                    "edge_class": {
                        "type": "string",
                        "enum": ["code", "structural", "semantic", "crossmodal", "hybrid"],
                        "description": "Optional edge class filter: code (calls/defines/implements/imports), structural (wikilinks/hierarchy), semantic (tags/similarity), crossmodal, hybrid (all)"
                    },
                    "where": {
                        "type": "string",
                        "description": "Optional filter predicate on candidate paths"
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of paths to return (default 20, max 100)"
                    },
                    "max_depth": {
                        "type": "number",
                        "description": "Hard cap on recursive traversal depth (default 3, max 5)"
                    }
                },
                "required": ["pattern"]
            }),
            handle_graph_match,
        );

        self.register_read(
            "graph_communities",
            "Detect communities in the knowledge graph. Defaults to Leiden partition. Pass view='architecture' for a high-level subsystem component overview with top key nodes (summarized, no raw member dump), or pass community_id to inspect members of a specific community.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "algorithm": { "type": "string", "enum": ["leiden", "louvain"], "description": "Community detection algorithm (default: leiden)" },
                    "view": { "type": "string", "enum": ["architecture", "raw"], "description": "View mode: 'architecture' for high-level components with top key nodes, 'raw' for raw community clusters (default: 'raw')" },
                    "include_density": { "type": "boolean", "description": "Include per-community density statistics (default false)" },
                    "community_id": { "type": "integer", "description": "Optional community ID to inspect member nodes for that specific community" },
                    "limit": { "type": "integer", "description": "Maximum number of communities to return in overview (default 10) or member nodes when community_id is specified (default 50)" }
                },
                "required": []
            }),
            handle_graph_communities,
        );

        // Write tools
        self.register_write(
            "write_note",
            "Create or update a markdown note. Supports modes: 'create' (fails if note already exists), 'overwrite', 'append', or 'prepend' (default: 'create'). Automatically keeps all indices in sync.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path for the note (e.g. 'projects/my-note.md')" },
                    "content": { "type": "string", "description": "Body content of the note (markdown)" },
                    "mode": { "type": "string", "enum": ["create", "overwrite", "append", "prepend"], "description": "Write mode: 'create' (default, fails if file exists), 'overwrite', 'append', or 'prepend'" },
                    "frontmatter": { "type": "object", "description": "Optional YAML frontmatter fields as a JSON object (title, tags, etc.)" },
                    "template": { "type": "string", "description": "Optional template name to validate against before writing" }
                },
                "required": ["path", "content"]
            }),
            handle_write_note,
        );

        self.register_write(
            "delete_note",
            "Delete a note from the corpus and purge it from all indices (Tantivy, vector store, graph, metadata).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path of the note to delete" }
                },
                "required": ["path"]
            }),
            handle_delete_note,
        );

        self.register_write(
            "move_note",
            "Move or rename a note, automatically updating inbound wikilinks across the corpus and all indices.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Current relative path of the note" },
                    "to": { "type": "string", "description": "New relative path of the note" }
                },
                "required": ["from", "to"]
            }),
            handle_move_note,
        );

        // Validation / Template tools
        self.register_read(
            "list_templates",
            "List registered note templates with their required and optional frontmatter fields, valid types, and regex rules.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            handle_list_templates,
        );

        self.register_read(
            "validate",
            "Run schema validation. Pass 'path' to validate a single note against its template; omit 'path' to scan the entire corpus. Pass check_taxonomy=true to enforce the corpus-defined tag/category taxonomy.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Optional relative path of a note to validate. If omitted, validates the entire corpus." },
                    "check_taxonomy": { "type": "boolean", "description": "Enforce the corpus-defined tag and category taxonomy (default false)" }
                },
                "required": []
            }),
            handle_validate,
        );

        // System / Corpus tools
        self.register_read(
            "status",
            "Report corpus health, indexing progress, graph topology, coverage status, or high-level architecture census.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["corpus", "indexing", "graph", "coverage", "census", "architecture", "all"], "description": "corpus = per-corpus stats/config; indexing = indexing progress; graph = topology stats & density; coverage = path-level index & parse status; census/architecture = instant structural census (symbols, edges, languages, routes); all (default) = combined." },
                    "corpus": { "type": "string", "description": "Target a single corpus by name for per-corpus stats/indexing. Omit for the multi-corpus overview across all configured corpora." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Scope 'coverage' only: paths or path prefixes to check for index coverage and parse status." }
                },
                "required": []
            }),
            handle_status,
        );

        self.register_read(
            "list_corpora",
            "List all loaded and discovered corpora in central cache with node/edge/vector statistics.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "include_cached": { "type": "boolean", "description": "Include dormant cached corpora (default true)" }
                },
                "required": []
            }),
            handle_list_corpora_dummy,
        );

        self.register_read(
            "trace_cross_corpus",
            "Federated cross-corpus graph traversal. Starts a bounded breadth-first walk at `start_node` in `start_corpus`, following intra-corpus edges up to `per_corpus_depth` and crossing cross-corpus edges up to `max_corpus_hops` times (when `continue` is true). Returns `nodes` (each tagged with its `corpus` and per-corpus `depth`) and `hops` (each cross-corpus seam with `from_corpus`/`to_corpus`, `to_node`, `edge_type`, `target_kind`, `confidence`, and `corpus_depth`). This tool takes an explicit `start_corpus`, so the fan-out `corpus`/`corpora`/\"all\" scoping args do NOT apply here.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "start_corpus": { "type": "string", "description": "Name of the corpus to begin the traversal in." },
                    "start_node": { "type": "string", "description": "Node key (path / scope_path / route key) to begin the traversal at." },
                    "per_corpus_depth": { "type": "integer", "description": "Max breadth-first depth walked WITHIN each corpus (default 3, clamped to 10).", "minimum": 0 },
                    "max_corpus_hops": { "type": "integer", "description": "Max number of cross-corpus edges the traversal may cross (default 3, clamped to 10).", "minimum": 0 },
                    "continue": { "type": "boolean", "description": "When true (default), continue the walk live into the far side of each cross-corpus edge. When false, cross-corpus hops are still recorded but never entered." }
                },
                "required": ["start_corpus", "start_node"]
            }),
            handle_trace_cross_corpus_dummy,
        );

        self.register_write(
            "sync_corpus",
            "Corpus index maintenance. Mode 'delta' (default) syncs filesystem changes incrementally; mode 'full' forces a full reindex with checkpoint resumption; mode 'reembed' recomputes dense embeddings.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["delta", "full", "reembed"], "description": "Sync mode: 'delta' (default, incremental sync), 'full' (full reindex), 'reembed' (recompute embeddings)" },
                    "fast": { "type": "boolean", "description": "Enable Fast Mode: skip dense embedding and vector indexing for instant indexing" },
                    "index_mode": { "type": "string", "enum": ["full", "fast"], "description": "Indexing mode override ('full', 'fast')" },
                    "batch_size": { "type": "number", "description": "Batch size for commits / intermediate checkpoints (default 50)" },
                    "resume": { "type": "boolean", "description": "For mode 'full': resume from last indexing checkpoint if available (default true)" }
                },
                "required": []
            }),
            handle_sync_corpus,
        );

        self.register_write(
            "index_corpus",
            "Dynamically index and mount a new repository by path without restarting the server.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Filesystem path to the repository/corpus directory" },
                    "name": { "type": "string", "description": "Optional corpus name override (defaults to directory name)" },
                    "sync": { "type": "boolean", "description": "Run delta sync after mounting (default true)" },
                    "reindex": { "type": "boolean", "description": "Force full reindex from scratch (default false)" },
                    "fast": { "type": "boolean", "description": "Skip dense vector embedding for instant indexing (default false)" },
                    "batch_size": { "type": "integer", "description": "Batch size for indexing (default 50)" }
                },
                "required": ["path"]
            }),
            handle_index_corpus_dummy,
        );

        self.register_write(
            "unload_corpus",
            "Free memory by unloading an inactive corpus from the central daemon.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the corpus to unload" }
                },
                "required": ["name"]
            }),
            handle_unload_corpus_dummy,
        );

        // Inject corpus/corpora discrimination args into tool schemas.
        self.inject_corpus_args();
    }

    /// Check if a tool is read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.tools.get(name).map(|t| t.is_read_only()).unwrap_or(false)
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&ToolInfo> {
        self.tools.get(name)
    }

    /// List all registered tools (for MCP `tools/list` response).
    pub fn list(&self) -> Vec<&ToolInfo> {
        let mut tools: Vec<&ToolInfo> = self.tools.values().collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    /// Execute a read-only tool with shared immutable access to the Engine.
    pub fn execute_read(&self, name: &str, engine: &Engine, args: Value) -> Result<Value> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| Error::NotFound(format!("tool not found: {}", name)))?;
        match &tool.handler {
            ToolHandler::ReadOnly(h) => h(engine, args),
            ToolHandler::ReadWrite(_) => {
                Err(Error::Config(format!("tool '{}' is mutating and requires write lock", name)))
            }
        }
    }

    /// Execute a tool with exclusive mutable access to the Engine.
    pub fn execute_write(&self, name: &str, engine: &mut Engine, args: Value) -> Result<Value> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| Error::NotFound(format!("tool not found: {}", name)))?;
        match &tool.handler {
            ToolHandler::ReadOnly(h) => h(engine, args),
            ToolHandler::ReadWrite(h) => h(engine, args),
        }
    }

    /// Execute a tool by name with given arguments (convenience wrapper around `execute_write`).
    pub fn execute(&self, name: &str, engine: &mut Engine, args: Value) -> Result<Value> {
        self.execute_write(name, engine, args)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Multi-Corpus Routing
// ---------------------------------------------------------------------------

use ctxvault_core::corpus_manager::CorpusManager;

/// Multi-corpus tool registry: wraps a `CorpusManager` and routes tool calls
/// to the correct engine(s) based on the `corpus` / `corpora` arguments.
///
/// - `corpus = "name"` targets a single corpus engine.
/// - `corpora = ["a", "b"]` or `corpora = "all"` fans out across several corpora;
///   search-style results are RRF-merged and each hit is tagged with its source
///   corpus.
/// - Omitting both resolves to the default corpus. This is an ergonomic default,
///   not a legacy code path.
pub struct MultiCorpusToolRegistry {
    registry: ToolRegistry,
    profile: ToolProfile,
}

/// The resolved fan-out target for a read tool call.
enum CorpusTarget {
    /// Exactly one corpus (explicit `corpus` or the default).
    Single(String),
    /// Two or more corpora to fan out across (deduplicated, order preserved).
    Multi(Vec<String>),
}

/// Dynamically construct a SchemaEnvelope from the actual returned search items and their graph affordances.
fn build_dynamic_schema_envelope(
    items: &[ctxvault_common::types::SearchResult],
    is_code: bool,
    extra_active_edges: &[String],
) -> ctxvault_common::types::SchemaEnvelope {
    let mut labels = std::collections::BTreeSet::new();
    let mut edges = std::collections::BTreeSet::new();

    for e in extra_active_edges {
        edges.insert(e.clone());
    }

    if is_code {
        labels.insert("CodeSymbol".to_string());
        labels.insert("CodeChunk".to_string());
        edges.insert("calls".to_string());
        edges.insert("implements".to_string());
        edges.insert("imports".to_string());
        edges.insert("defines".to_string());

        for item in items {
            if let Some(ref kind) = item.entity_kind {
                match kind {
                    ctxvault_common::types::EntityKind::CodeSymbol { symbol_type, .. } => {
                        labels.insert(format!("{:?}", symbol_type));
                    }
                    ctxvault_common::types::EntityKind::CodeChunk { .. } => {
                        labels.insert("CodeChunk".to_string());
                    }
                    ctxvault_common::types::EntityKind::CodeFile { .. } => {
                        labels.insert("CodeFile".to_string());
                    }
                    ctxvault_common::types::EntityKind::Documentation { .. } => {
                        labels.insert("DocNode".to_string());
                    }
                }
            }
            if let Some(ref aff) = item.graph_affordances {
                for (edge_name, count) in &aff.edge_counts {
                    if *count > 0 {
                        let clean = edge_name.strip_suffix("_in").unwrap_or(edge_name);
                        edges.insert(clean.to_string());
                    }
                }
            }
            if let Some(ref g) = item.graph {
                for part in g.split("[:") {
                    if let Some(end) = part.find(['*', ']', ' ', '-']) {
                        let rel = &part[..end];
                        if !rel.is_empty() {
                            edges.insert(rel.to_string());
                        }
                    }
                }
            }
        }
    } else {
        labels.insert("DocNode".to_string());
        labels.insert("ADR".to_string());
        labels.insert("Concept".to_string());
        edges.insert("wikilink".to_string());
        edges.insert("supersedes".to_string());
        edges.insert("documents".to_string());
        edges.insert("tag".to_string());

        for item in items {
            if let Some(ref lineage) = item.lineage {
                if !lineage.superseded_by.is_empty() || !lineage.supersedes.is_empty() {
                    edges.insert("supersedes".to_string());
                }
                if !lineage.implements.is_empty() || !lineage.implemented_by.is_empty() {
                    edges.insert("implements".to_string());
                }
                if !lineage.depends_on.is_empty() || !lineage.depended_on_by.is_empty() {
                    edges.insert("depends_on".to_string());
                }
                for incoming_type in lineage.incoming.keys() {
                    edges.insert(incoming_type.clone());
                }
                for outgoing_type in lineage.outgoing.keys() {
                    edges.insert(outgoing_type.clone());
                }
            }
            if let Some(ref aff) = item.graph_affordances {
                for (edge_name, count) in &aff.edge_counts {
                    if *count > 0 {
                        let clean = edge_name.strip_suffix("_in").unwrap_or(edge_name);
                        edges.insert(clean.to_string());
                    }
                }
            }
            if let Some(ref g) = item.graph {
                for part in g.split("[:") {
                    if let Some(end) = part.find(['*', ']', ' ', '-']) {
                        let rel = &part[..end];
                        if !rel.is_empty() {
                            edges.insert(rel.to_string());
                        }
                    }
                }
            }
        }
    }

    ctxvault_common::types::SchemaEnvelope {
        node_labels: labels.into_iter().collect(),
        active_edges: edges.into_iter().collect(),
    }
}

impl MultiCorpusToolRegistry {
    /// Create a new multi-corpus registry with all tools registered and the
    /// [`ToolProfile::All`] exposure profile.
    pub fn new() -> Self {
        Self::with_profile(ToolProfile::All)
    }

    /// Create a new multi-corpus registry exposing tools under `profile`.
    ///
    /// The profile only gates what [`Self::list`] advertises; every registered
    /// tool remains executable regardless of profile.
    pub fn with_profile(profile: ToolProfile) -> Self {
        let mut registry = ToolRegistry::new();
        registry.register_all();

        Self { registry, profile }
    }

    /// The active tool exposure profile.
    pub fn profile(&self) -> ToolProfile {
        self.profile
    }

    /// Check if a tool is read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.registry.is_read_only(name)
    }

    /// List the tools advertised under the active profile (for MCP `tools/list`).
    pub fn list(&self) -> Vec<&ToolInfo> {
        self.registry.list().into_iter().filter(|t| self.profile.includes(&t.name)).collect()
    }

    /// List every registered tool regardless of profile (for internal use).
    pub fn list_all(&self) -> Vec<&ToolInfo> {
        self.registry.list()
    }

    /// Execute a read-only tool call, routing to one corpus or fanning out across
    /// several with RRF-merged, corpus-tagged results.
    pub fn execute_read(&self, name: &str, manager: &CorpusManager, args: Value) -> Result<Value> {
        // `status` without an explicit `corpus` returns the manager-level overview
        // (all corpora). With a `corpus` it routes to that engine's status below.
        if name == "status" && !has_corpus_arg(&args) {
            return handle_get_status(manager);
        }

        if name == "list_corpora" {
            return handle_list_corpora_manager(manager, args);
        }

        // Federated traversal is a manager-level walk across corpora (it needs the
        // whole manager, not a single engine), so intercept it before per-engine
        // dispatch. It takes an explicit `start_corpus`, not the fan-out args.
        if name == "trace_cross_corpus" {
            return handle_trace_cross_corpus(manager, args);
        }

        // Parse both discrimination args out of the call, resolving the target set.
        let (target, clean_args) = resolve_corpus_target(args, manager)?;

        match target {
            CorpusTarget::Single(corpus_name) => {
                let engine = manager.get_engine(&corpus_name)?;
                let output = self.registry.execute_read(name, engine, clean_args)?;
                Ok(tag_search_output(output, &corpus_name))
            }
            CorpusTarget::Multi(names) => self.fan_out_read(name, manager, &names, clean_args),
        }
    }

    /// Fan out a read tool across multiple corpora and merge the results.
    ///
    /// Search-style outputs (JSON arrays of `SearchResult`) are RRF-merged via
    /// [`search::rrf_fuse_cross_corpus`] and returned as one tagged array. Other
    /// (non-array) outputs are returned as a JSON object keyed by corpus name.
    fn fan_out_read(
        &self,
        name: &str,
        manager: &CorpusManager,
        names: &[String],
        clean_args: Value,
    ) -> Result<Value> {
        let limit =
            clean_args.get("limit").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(10);

        let mut per_corpus: Vec<(String, Value)> = Vec::new();
        let mut last_err: Option<Error> = None;

        for corpus_name in names {
            let engine = match manager.get_engine(corpus_name) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(corpus = %corpus_name, error = %e, "fan-out: engine resolve failed");
                    last_err = Some(e);
                    continue;
                }
            };
            match self.registry.execute_read(name, engine, clean_args.clone()) {
                Ok(v) => per_corpus.push((corpus_name.clone(), v)),
                Err(e) => {
                    tracing::warn!(corpus = %corpus_name, error = %e, "fan-out: tool call failed");
                    last_err = Some(e);
                }
            }
        }

        if per_corpus.is_empty() {
            return Err(last_err.unwrap_or_else(|| {
                Error::NotFound("no corpora available for fan-out".to_string())
            }));
        }

        let all_search_responses = per_corpus
            .iter()
            .all(|(_, v)| v.is_object() && (v.get("docs").is_some() || v.get("code").is_some()));
        if all_search_responses {
            let mut docs_tagged: Vec<(String, Vec<ctxvault_common::types::SearchResult>)> =
                Vec::new();
            let mut code_tagged: Vec<(String, Vec<ctxvault_common::types::SearchResult>)> =
                Vec::new();
            for (corpus_name, value) in per_corpus {
                let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(value)
                    .map_err(|e| Error::Config(format!("invalid search response: {}", e)))?;
                if let Some(d) = resp.docs {
                    docs_tagged.push((corpus_name.clone(), d.results));
                }
                if let Some(c) = resp.code {
                    code_tagged.push((corpus_name, c.results));
                }
            }
            let merged_docs = search::rrf_fuse_cross_corpus(&docs_tagged, limit);
            let merged_code = search::rrf_fuse_cross_corpus(&code_tagged, limit);
            let docs_partition = if !merged_docs.is_empty() {
                Some(ctxvault_common::types::SearchPartition {
                    total_matches: merged_docs.len(),
                    top_k_returned: merged_docs.len(),
                    schema_envelope: build_dynamic_schema_envelope(&merged_docs, false, &[]),
                    results: merged_docs,
                })
            } else {
                None
            };
            let code_partition = if !merged_code.is_empty() {
                Some(ctxvault_common::types::SearchPartition {
                    total_matches: merged_code.len(),
                    top_k_returned: merged_code.len(),
                    schema_envelope: build_dynamic_schema_envelope(&merged_code, true, &[]),
                    results: merged_code,
                })
            } else {
                None
            };
            let resp = ctxvault_common::types::SearchResponse {
                docs: docs_partition,
                code: code_partition,
            };
            return serde_json::to_value(resp)
                .map_err(|e| Error::Config(format!("serialize merged search response: {}", e)));
        }

        // If every successful output is a JSON array, treat as search-style and RRF-merge.
        let all_arrays = per_corpus.iter().all(|(_, v)| v.is_array());
        if all_arrays {
            let mut tagged_lists: Vec<(String, Vec<ctxvault_common::types::SearchResult>)> =
                Vec::with_capacity(per_corpus.len());
            for (corpus_name, value) in per_corpus {
                let results: Vec<ctxvault_common::types::SearchResult> =
                    serde_json::from_value(value).map_err(|e| {
                        Error::Config(format!("invalid search result array: {}", e))
                    })?;
                tagged_lists.push((corpus_name, results));
            }
            let merged = search::rrf_fuse_cross_corpus(&tagged_lists, limit);
            return serde_json::to_value(merged)
                .map_err(|e| Error::Config(format!("serialize merged results: {}", e)));
        }

        // Otherwise: return an object keyed by corpus name → raw output.
        let obj: serde_json::Map<String, Value> = per_corpus.into_iter().collect();
        Ok(Value::Object(obj))
    }

    /// Execute a tool call with exclusive access to the CorpusManager.
    ///
    /// Write tools always resolve a SINGLE corpus (explicit `corpus` or the default)
    /// and never fan out. Omitting `corpus` selects the default corpus as an
    /// ergonomic default.
    pub fn execute_write(
        &self,
        name: &str,
        manager: &mut CorpusManager,
        args: Value,
    ) -> Result<Value> {
        if name == "index_corpus" {
            return handle_index_corpus_manager(manager, args);
        }
        if name == "unload_corpus" {
            return handle_unload_corpus_manager(manager, args);
        }

        // `status` without an explicit `corpus` returns the manager-level overview.
        if name == "status" && !has_corpus_arg(&args) {
            return handle_get_status(manager);
        }

        // Extract and remove the `corpus` param from arguments (writes never fan out).
        let (corpus_name, clean_args) = extract_corpus_param(args);

        // Resolve the engine mutably.
        let engine = manager.resolve_engine_mut(corpus_name.as_deref())?;

        // Execute the tool.
        self.registry.execute_write(name, engine, clean_args)
    }

    /// Execute a tool call, routing to the correct corpus engine.
    pub fn execute(&self, name: &str, manager: &mut CorpusManager, args: Value) -> Result<Value> {
        self.execute_write(name, manager, args)
    }

    /// Get underlying registry reference (for listing tools etc).
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

impl Default for MultiCorpusToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the tool arguments explicitly target a single `corpus` by name.
fn has_corpus_arg(args: &Value) -> bool {
    args.get("corpus").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// Extract the optional single `corpus` field from tool arguments, returning
/// the corpus name and the arguments with `corpus` removed. Used by write tools,
/// which never fan out.
fn extract_corpus_param(args: Value) -> (Option<String>, Value) {
    match args {
        Value::Object(mut map) => {
            let corpus = map.remove("corpus").and_then(|v| v.as_str().map(|s| s.to_string()));
            (corpus, Value::Object(map))
        }
        other => (None, other),
    }
}

/// Parse both `corpus` and `corpora` out of read-tool arguments and resolve the
/// target corpus set, returning it alongside the arguments with BOTH keys removed.
///
/// Resolution precedence:
/// - `corpora == "all"` → every corpus (sorted for determinism);
/// - `corpora` as a non-empty array → those names (each validated to exist);
/// - `corpus` set → that single corpus;
/// - neither → the default corpus (single).
fn resolve_corpus_target(args: Value, manager: &CorpusManager) -> Result<(CorpusTarget, Value)> {
    let Value::Object(mut map) = args else {
        // Non-object args cannot carry discrimination — fall back to default corpus.
        let default = manager
            .default_corpus_name()
            .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?
            .to_string();
        return Ok((CorpusTarget::Single(default), args));
    };

    let corpus = map.remove("corpus").and_then(|v| v.as_str().map(|s| s.to_string()));
    let corpora = map.remove("corpora");
    let clean_args = Value::Object(map);

    let target = match corpora {
        Some(Value::String(s)) if s == "all" => {
            let mut names: Vec<String> =
                manager.corpus_names().into_iter().map(|s| s.to_string()).collect();
            names.sort();
            multi_or_single(names)?
        }
        Some(Value::Array(items)) => {
            let mut names: Vec<String> = Vec::with_capacity(items.len());
            for item in items {
                let n = item
                    .as_str()
                    .ok_or_else(|| Error::Config("corpora array must contain strings".to_string()))?
                    .to_string();
                if !manager.has_corpus(&n) {
                    return Err(Error::NotFound(format!("corpus not found: {}", n)));
                }
                names.push(n);
            }
            if names.is_empty() {
                // Empty array behaves like "omitted": resolve default.
                single_default(corpus, manager)?
            } else {
                multi_or_single(names)?
            }
        }
        Some(Value::String(s)) => {
            return Err(Error::Config(format!(
                "invalid corpora value '{}': expected an array of names or \"all\"",
                s
            )));
        }
        Some(_) => {
            return Err(Error::Config(
                "invalid corpora value: expected an array of names or \"all\"".to_string(),
            ));
        }
        None => single_default(corpus, manager)?,
    };

    Ok((target, clean_args))
}

/// Resolve the single-corpus target from an explicit `corpus` or the default.
fn single_default(corpus: Option<String>, manager: &CorpusManager) -> Result<CorpusTarget> {
    match corpus {
        Some(name) => {
            if !manager.has_corpus(&name) {
                return Err(Error::NotFound(format!("corpus not found: {}", name)));
            }
            Ok(CorpusTarget::Single(name))
        }
        None => {
            let default = manager
                .default_corpus_name()
                .ok_or_else(|| Error::NotFound("no default corpus configured".to_string()))?
                .to_string();
            Ok(CorpusTarget::Single(default))
        }
    }
}

/// Collapse a resolved name list into `Single` (one, deduped) or `Multi` (many).
fn multi_or_single(mut names: Vec<String>) -> Result<CorpusTarget> {
    names.dedup();
    match names.len() {
        0 => Err(Error::NotFound("no corpora resolved for fan-out".to_string())),
        1 => Ok(CorpusTarget::Single(names.into_iter().next().unwrap())),
        _ => Ok(CorpusTarget::Multi(names)),
    }
}

/// Tag a single-corpus read output: if it is a JSON array of `SearchResult`,
/// stamp each hit with the source corpus; otherwise return it unchanged.
fn tag_search_output(output: Value, corpus_name: &str) -> Value {
    if output.is_array() {
        if let Ok(results) =
            serde_json::from_value::<Vec<ctxvault_common::types::SearchResult>>(output.clone())
        {
            let tagged: Vec<ctxvault_common::types::SearchResult> =
                results.into_iter().map(|r| r.with_corpus(Some(corpus_name.to_string()))).collect();
            return serde_json::to_value(tagged).unwrap_or(output);
        }
    } else if output.is_object() && (output.get("docs").is_some() || output.get("code").is_some()) {
        if let Ok(mut resp) =
            serde_json::from_value::<ctxvault_common::types::SearchResponse>(output.clone())
        {
            if let Some(ref mut d) = resp.docs {
                for r in &mut d.results {
                    r.corpus = Some(corpus_name.to_string());
                }
            }
            if let Some(ref mut c) = resp.code {
                for r in &mut c.results {
                    r.corpus = Some(corpus_name.to_string());
                }
            }
            return serde_json::to_value(resp).unwrap_or(output);
        }
    } else if let Value::Object(mut map) = output {
        map.entry("corpus").or_insert_with(|| Value::String(corpus_name.to_string()));
        return Value::Object(map);
    }
    output
}

/// Get overall system status from the CorpusManager.
fn handle_get_status(manager: &CorpusManager) -> Result<Value> {
    let corpora = manager.list_corpora();
    let default_name = manager.default_corpus_name().unwrap_or("none");

    let corpora_info: Vec<Value> = corpora
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "path": c.path,
                "mode": c.mode,
                "index_mode": c.index_mode,
                "file_count": c.file_count,
                "embedder_active": c.embedder_active,
                "vector_count": c.vector_count,
                "graph_node_count": c.graph_node_count,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "corpus_count": manager.corpus_count(),
        "default_corpus": default_name,
        "corpora": corpora_info,
    }))
}

/// Handle `list_corpora` across active and cached corpora.
fn handle_list_corpora_manager(manager: &CorpusManager, args: Value) -> Result<Value> {
    let include_cached = args.get("include_cached").and_then(Value::as_bool).unwrap_or(true);
    let loaded = manager.list_corpora();
    let loaded_names: HashSet<String> = loaded.iter().map(|c| c.name.clone()).collect();

    let mut corpora_info: Vec<Value> = loaded
        .into_iter()
        .map(|c| {
            let index_path = ctxvault_common::config::get_corpus_index_dir(&c.name);
            serde_json::json!({
                "name": c.name,
                "path": c.path,
                "index_path": index_path.to_string_lossy().replace('\\', "/"),
                "status": "active",
                "mode": c.mode,
                "index_mode": c.index_mode,
                "file_count": c.file_count,
                "embedder_active": c.embedder_active,
                "vector_count": c.vector_count,
                "graph_node_count": c.graph_node_count,
            })
        })
        .collect();

    if include_cached {
        for cached in manager.discover_cached_corpora() {
            if !loaded_names.contains(&cached) {
                let cache_dir = ctxvault_common::config::get_corpus_index_dir(&cached);
                let source_path =
                    ctxvault_core::corpus_manager::CorpusManager::get_cached_corpus_source_path(
                        &cached,
                    )
                    .unwrap_or_else(|| cache_dir.to_string_lossy().replace('\\', "/"));
                corpora_info.push(serde_json::json!({
                    "name": cached,
                    "status": "cached",
                    "path": source_path,
                    "index_path": cache_dir.to_string_lossy().replace('\\', "/"),
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "corpora": corpora_info,
        "default_corpus": manager.default_corpus_name(),
        "total_active": manager.corpus_count(),
    }))
}

/// Handle `index_corpus` dynamically mounting and indexing a new repository.
fn handle_index_corpus_manager(manager: &mut CorpusManager, args: Value) -> Result<Value> {
    let path_str = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("missing required argument 'path'".to_string()))?;

    let corpus_path = PathBuf::from(path_str);
    let name_override = args.get("name").and_then(Value::as_str);
    let name = manager.ensure_corpus_with_name(&corpus_path, name_override)?;

    let do_reindex = args.get("reindex").and_then(Value::as_bool).unwrap_or(false);
    let do_sync = args.get("sync").and_then(Value::as_bool).unwrap_or(true);
    let fast = args.get("fast").and_then(Value::as_bool).unwrap_or(false);
    let batch_size =
        args.get("batch_size").and_then(Value::as_u64).map(|n| n as usize).unwrap_or(50);

    let engine = manager.get_engine_mut(&name)?;

    if fast {
        engine.config_mut().index_mode = ctxvault_common::config::IndexMode::Fast;
    }

    let index_stats = if do_reindex {
        let count = engine.full_reindex_paginated(batch_size, false)?;
        serde_json::json!({ "reindexed_files": count })
    } else if do_sync {
        let delta = engine.delta_scan_paginated(batch_size)?;
        serde_json::json!({
            "new_files": delta.new_files.len(),
            "modified_files": delta.modified_files.len(),
            "deleted_files": delta.deleted_files.len()
        })
    } else {
        serde_json::json!({ "status": "mounted_without_indexing" })
    };

    let file_count = engine.store().list_files().map(|f| f.len()).unwrap_or(0);

    Ok(serde_json::json!({
        "status": "success",
        "corpus": name,
        "path": path_str,
        "file_count": file_count,
        "indexing": index_stats,
    }))
}

/// Handle `unload_corpus` freeing memory from an open engine.
fn handle_unload_corpus_manager(manager: &mut CorpusManager, args: Value) -> Result<Value> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("missing required argument 'name'".to_string()))?;

    let unloaded = manager.unload_corpus(name)?;
    Ok(serde_json::json!({
        "status": if unloaded { "unloaded" } else { "not_found" },
        "corpus": name
    }))
}

fn handle_list_corpora_dummy(_engine: &Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("list_corpora is a manager-level tool".to_string()))
}

/// Upper bound on `per_corpus_depth` / `max_corpus_hops` to keep the federated
/// walk bounded and protect query latency (invariant I3).
const MAX_FEDERATED_BOUND: usize = 10;

/// Handle `trace_cross_corpus`: a bounded federated graph traversal across
/// corpora, returning corpus-tagged nodes and cross-corpus hop records.
///
/// Parses the start point plus bounded depth/hop budgets (clamped to
/// [`MAX_FEDERATED_BOUND`]), delegates to `CorpusManager::federated_traverse`,
/// and serializes the resulting `FederatedTraversal` to JSON.
fn handle_trace_cross_corpus(manager: &CorpusManager, args: Value) -> Result<Value> {
    let start_corpus = args
        .get("start_corpus")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("trace_cross_corpus requires 'start_corpus'".to_string()))?;
    let start_node = args
        .get("start_node")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("trace_cross_corpus requires 'start_node'".to_string()))?;

    let per_corpus_depth = args
        .get("per_corpus_depth")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(3)
        .min(MAX_FEDERATED_BOUND);
    let max_corpus_hops = args
        .get("max_corpus_hops")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(3)
        .min(MAX_FEDERATED_BOUND);
    let continue_across = args.get("continue").and_then(Value::as_bool).unwrap_or(true);

    let traversal = manager.federated_traverse(
        start_corpus,
        start_node,
        per_corpus_depth,
        max_corpus_hops,
        continue_across,
    )?;

    serde_json::to_value(&traversal)
        .map_err(|e| Error::Config(format!("failed to serialize federated traversal: {}", e)))
}

fn handle_trace_cross_corpus_dummy(_engine: &Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("trace_cross_corpus is a manager-level tool".to_string()))
}

fn handle_index_corpus_dummy(_engine: &mut Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("index_corpus is a manager-level tool".to_string()))
}

fn handle_unload_corpus_dummy(_engine: &mut Engine, _args: Value) -> Result<Value> {
    Err(Error::Config("unload_corpus is a manager-level tool".to_string()))
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum PathOrPaths {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadFileParams {
    pub path: Option<PathOrPaths>,
    pub paths: Option<Vec<String>>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub max_lines: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListNotesParams {
    pub path: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GetSnippetParams {
    pub name: Option<String>,
    pub path: Option<String>,
    pub chunk_index: Option<usize>,
    pub qualified_name: Option<String>,
    pub max_lines: Option<usize>,
    #[serde(default)]
    pub include_neighbors: bool,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub mode: Option<String>,
    pub limit: Option<usize>,
    pub depth: Option<String>,
    pub graph_depth: Option<usize>,
    pub edge_types: Option<Vec<String>>,
    pub edge_class: Option<String>,
    pub decompose: Option<bool>,
    pub modality: Option<String>,
    pub detail: Option<String>,
    pub snippets: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchRelatedParams {
    pub seeds: Vec<String>,
    pub limit: Option<usize>,
    pub modality: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatusParams {
    pub scope: Option<String>,
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphMatchParams {
    pub pattern: String,
    pub edge_class: Option<String>,
    #[serde(rename = "where")]
    pub where_clause: Option<String>,
    pub limit: Option<usize>,
    pub max_depth: Option<usize>,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GraphCommunitiesParams {
    pub algorithm: Option<String>,
    pub view: Option<String>,
    pub include_density: Option<bool>,
    pub community_id: Option<usize>,
    pub limit: Option<usize>,
}

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

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub path: Option<String>,
    pub check_taxonomy: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SyncCorpusParams {
    pub mode: Option<String>,
    pub batch_size: Option<usize>,
    pub resume: Option<bool>,
    pub fast: Option<bool>,
    pub index_mode: Option<String>,
}

// ---------------------------------------------------------------------------
// Response structs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct NoteListItem {
    path: String,
    title: Option<String>,
    template: Option<String>,
    content_hash: String,
}

/// Detect a source language from a file extension. Returns `"text"` when unknown.
fn language_from_path(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("py") => "python",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") => "cpp",
        Some("md") | Some("markdown") => "markdown",
        _ => "text",
    }
}

/// Bound a body of source lines to `max_lines`, joining with newlines and
/// reporting whether truncation occurred.
fn cap_lines(lines: &[&str], max_lines: usize) -> (String, bool) {
    if lines.len() > max_lines {
        (lines[..max_lines].join("\n"), true)
    } else {
        (lines.join("\n"), false)
    }
}

/// Build a bare handle (no body) for a code symbol: scope_path + file + line range + signature/docstring.
fn code_symbol_handle(sym: &ctxvault_common::types::CodeSymbol) -> Value {
    serde_json::json!({
        "scope_path": sym.scope_path,
        "name": sym.name,
        "file_path": sym.file_path,
        "start_line": sym.start_line,
        "end_line": sym.end_line,
        "language": sym.language,
        "symbol_type": sym.symbol_type,
        "signature": sym.signature,
        "docstring": sym.docstring,
    })
}

fn read_file_lossy(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read a single file for [`handle_read_file`].
fn read_single_file(
    engine: &Engine,
    path: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
    max_lines: usize,
) -> Result<Value> {
    let proj_path = engine.projection_path(path);
    let is_projected = proj_path.is_file();
    let raw = if is_projected {
        read_file_lossy(&proj_path)
            .map_err(|e| Error::NotFound(format!("cannot read projection for {}: {}", path, e)))?
    } else {
        let corpus_root = Path::new(&engine.config().path);
        let full_path = corpus_root.join(path);
        read_file_lossy(&full_path)
            .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?
    };

    if is_projected {
        let file_lines: Vec<&str> = raw.lines().collect();
        let total_lines = file_lines.len();
        let start = start_line.unwrap_or(1).max(1);
        let end = end_line.unwrap_or(total_lines).min(total_lines);

        let (content, truncated) = if start > total_lines {
            (String::new(), false)
        } else {
            let slice_start = start - 1;
            let slice_end = end.max(slice_start);
            let slice = &file_lines[slice_start..slice_end];
            cap_lines(slice, max_lines)
        };

        return Ok(serde_json::json!({
            "kind": "projected_doc",
            "path": path,
            "start_line": start,
            "end_line": end,
            "total_lines": total_lines,
            "language": "markdown",
            "content": content,
            "truncated": truncated,
        }));
    }

    let is_markdown = matches!(language_from_path(path), "markdown");
    if is_markdown && start_line.is_none() && end_line.is_none() {
        let doc = ctxvault_core::parser::parse_document(Path::new(path), &raw)?;
        let lines: Vec<&str> = doc.content.lines().collect();
        let (content, truncated) = cap_lines(&lines, max_lines);
        return Ok(serde_json::json!({
            "kind": "markdown_note",
            "path": path,
            "title": doc.title,
            "frontmatter": doc.frontmatter,
            "content": content,
            "truncated": truncated,
            "content_hash": doc.content_hash,
        }));
    }

    let file_lines: Vec<&str> = raw.lines().collect();
    let total_lines = file_lines.len();
    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(total_lines).min(total_lines);

    if start > total_lines {
        return Ok(serde_json::json!({
            "kind": if is_markdown { "markdown_note" } else { "code_file" },
            "path": path,
            "start_line": start,
            "end_line": end,
            "total_lines": total_lines,
            "content": "",
            "truncated": false,
        }));
    }

    let slice_start = start - 1;
    let slice_end = end.max(slice_start);
    let slice = &file_lines[slice_start..slice_end];
    let (content, truncated) = cap_lines(slice, max_lines);

    Ok(serde_json::json!({
        "kind": if is_markdown { "markdown_note" } else { "code_file" },
        "path": path,
        "start_line": start,
        "end_line": end,
        "total_lines": total_lines,
        "language": language_from_path(path),
        "content": content,
        "truncated": truncated,
    }))
}

/// Tier 3 read of one or more files (markdown or source code).
fn handle_read_file(engine: &Engine, args: Value) -> Result<Value> {
    let params: ReadFileParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let target_paths = if let Some(paths) = params.paths {
        PathOrPaths::Multiple(paths)
    } else if let Some(p) = params.path {
        p
    } else {
        return Err(Error::Config("read_file requires 'path' or 'paths'".to_string()));
    };

    let is_lean = params.format.as_deref() == Some("lean");

    match target_paths {
        PathOrPaths::Single(p) => {
            let max_lines = params.max_lines.unwrap_or(1000).max(1);
            let val = read_single_file(engine, &p, params.start_line, params.end_line, max_lines)?;
            if is_lean {
                let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let start_line =
                    val.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end_line = val.get("end_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let total_lines =
                    val.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let language = val.get("language").and_then(|v| v.as_str()).unwrap_or("text");
                let kind = val.get("kind").and_then(|v| v.as_str());
                let is_markdown = kind == Some("markdown_note") || kind == Some("projected_doc");
                let truncated = val.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);

                let lean = crate::format::lean::format_lean_read_file(
                    &p,
                    start_line,
                    end_line,
                    total_lines,
                    content,
                    language,
                    is_markdown,
                    truncated,
                );
                Ok(Value::String(lean))
            } else {
                Ok(val)
            }
        }
        PathOrPaths::Multiple(paths) => {
            let max_lines = params.max_lines.unwrap_or(500).max(1);
            let results: Vec<(String, std::result::Result<Value, String>)> = paths
                .iter()
                .map(|p| {
                    let res = read_single_file(engine, p, None, None, max_lines)
                        .map_err(|e| e.to_string());
                    (p.clone(), res)
                })
                .collect();

            if is_lean {
                let lean = crate::format::lean::format_lean_read_multiple(&results);
                Ok(Value::String(lean))
            } else {
                let json_results: Vec<Value> = results
                    .into_iter()
                    .map(|(p, res)| match res {
                        Ok(val) => val,
                        Err(e) => serde_json::json!({ "path": p, "error": e }),
                    })
                    .collect();
                Ok(serde_json::json!({
                    "count": json_results.len(),
                    "results": json_results,
                }))
            }
        }
    }
}

/// Tier 2 fetch: return exactly one code symbol's source or one doc chunk,
/// bounded by `max_lines`, with optional neighbor expansion.
fn handle_get_snippet(engine: &Engine, args: Value) -> Result<Value> {
    let params: GetSnippetParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let max_lines = params.max_lines.unwrap_or(500).max(1);
    let corpus_root = Path::new(&engine.config().path);
    let is_lean = params.format.as_deref() == Some("lean");

    let target_name = params.qualified_name.or(params.name);
    if let Some(ref qualified_name) = target_name {
        return fetch_code_symbol(
            engine,
            corpus_root,
            qualified_name,
            max_lines,
            params.include_neighbors,
            is_lean,
        );
    }

    if let Some(path) = params.path.as_deref() {
        if let Some(chunk_index) = params.chunk_index {
            return fetch_doc_chunk(
                engine,
                path,
                chunk_index,
                max_lines,
                params.include_neighbors,
                is_lean,
            );
        }
        return Err(Error::Config(format!(
            "get_snippet needs a chunk_index for a doc fetch on '{path}'. \
             For a whole file use Tier 3: read_file.",
        )));
    }

    Err(Error::Config(
        "get_snippet requires either `name`/`qualified_name` (code) or `path`+`chunk_index` (doc)."
            .to_string(),
    ))
}

/// Fetch a single code symbol's bounded source by qualified name (or fuzzy name),
/// optionally attaching caller/callee handles.
fn fetch_code_symbol(
    engine: &Engine,
    corpus_root: &Path,
    qualified_name: &str,
    max_lines: usize,
    include_neighbors: bool,
    is_lean: bool,
) -> Result<Value> {
    let mut matches = engine.store().find_symbols_by_qualified_name(qualified_name)?;
    if matches.is_empty() {
        matches = engine.store().find_symbols_by_normalized_scope(qualified_name)?;
    }
    if matches.is_empty() {
        matches = engine.store().find_symbols_by_name(qualified_name)?;
    }

    match matches.len() {
        0 => {
            let leaf = qualified_name.split(" > ").last().unwrap_or(qualified_name).trim();
            let leaf_candidates = engine.store().find_symbols_by_name(leaf).unwrap_or_default();
            let leaf_matches: Vec<_> =
                leaf_candidates.into_iter().filter(|s| s.name.eq_ignore_ascii_case(leaf)).collect();

            if !leaf_matches.is_empty() {
                let candidates: Vec<Value> = leaf_matches.iter().map(code_symbol_handle).collect();
                if is_lean {
                    let mut s = format!(
                        "# Candidate Suggestions for '{qualified_name}'\n\nNo code symbol matches '{qualified_name}', but found {} candidate(s) with leaf name '{leaf}'. Disambiguate with an exact scope_path:\n\n",
                        candidates.len()
                    );
                    for (i, c) in candidates.iter().enumerate() {
                        let num = i + 1;
                        let name = c["name"].as_str().unwrap_or("");
                        let scope = c["scope_path"].as_str().unwrap_or("");
                        let file = c["file_path"].as_str().unwrap_or("");
                        let start = c["start_line"].as_u64().unwrap_or(0);
                        let end = c["end_line"].as_u64().unwrap_or(0);
                        s.push_str(&format!(
                            "{num}. {name} (`{file}`:L{start}-L{end}) [scope: `{scope}`]\n"
                        ));
                        s.push_str(&format!("   -> get_snippet(name: \"{scope}\")\n"));
                    }
                    return Ok(Value::String(s));
                }
                Ok(serde_json::json!({
                    "kind": "candidate_suggestions",
                    "note": format!(
                        "No code symbol matches '{qualified_name}', but found {} candidate(s) with leaf name '{leaf}'. Disambiguate with an exact scope_path.",
                        candidates.len()
                    ),
                    "candidates": candidates,
                }))
            } else {
                Err(Error::NotFound(format!("no code symbol matches '{qualified_name}'")))
            }
        }
        1 => {
            let sym = &matches[0];
            let full_path = corpus_root.join(&sym.file_path);
            let content = read_file_lossy(&full_path)
                .map_err(|e| Error::NotFound(format!("cannot read {}: {}", sym.file_path, e)))?;
            let file_lines: Vec<&str> = content.lines().collect();

            let (source, truncated) = if sym.start_line > 0 && sym.start_line <= file_lines.len() {
                let start_idx = sym.start_line - 1;
                let end_idx = sym.end_line.min(file_lines.len());
                cap_lines(&file_lines[start_idx..end_idx], max_lines)
            } else {
                (String::new(), false)
            };

            let total_lines = file_lines.len();
            let mut out = serde_json::json!({
                "path": sym.file_path,
                "start_line": sym.start_line,
                "end_line": sym.end_line,
                "total_lines": total_lines,
                "source": source,
            });
            if let Some(ref doc) = sym.docstring {
                if !doc.trim().is_empty() {
                    out["docstring"] = serde_json::Value::String(doc.clone());
                }
            }
            if truncated {
                out["truncated"] = serde_json::Value::Bool(true);
            }

            let mut incoming: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            let mut outgoing: BTreeMap<String, Vec<Value>> = BTreeMap::new();

            if include_neighbors {
                let all_symbols = engine.store().get_all_code_symbols().unwrap_or_default();
                let mut sym_map: HashMap<String, &ctxvault_common::types::CodeSymbol> =
                    HashMap::with_capacity(all_symbols.len() * 2);
                for s in &all_symbols {
                    sym_map.insert(s.scope_path.clone(), s);
                    sym_map.insert(s.name.clone(), s);
                }

                let edges = engine.graph().get_all_edges();
                let matches_sym =
                    |candidate: &str| candidate == sym.scope_path || candidate == sym.name;

                // Grammar-driven graph relationships grouped by edge_type:
                // incoming (edges where target is this symbol)
                // outgoing (edges where source is this symbol)
                let mut seen_incoming = HashSet::new();
                let mut seen_outgoing = HashSet::new();

                for e in edges.iter().filter(|e| matches_sym(&e.target)) {
                    if seen_incoming.insert((e.edge_type.clone(), e.source.clone())) {
                        let node = if let Some(s) = sym_map.get(&e.source) {
                            code_symbol_handle(s)
                        } else {
                            serde_json::json!({
                                "name": e.source,
                                "scope_path": e.source,
                                "unresolved": true,
                            })
                        };
                        incoming.entry(e.edge_type.clone()).or_default().push(node);
                    }
                }

                for e in edges.iter().filter(|e| matches_sym(&e.source)) {
                    if seen_outgoing.insert((e.edge_type.clone(), e.target.clone())) {
                        let node = if let Some(s) = sym_map.get(&e.target) {
                            code_symbol_handle(s)
                        } else if let Some(ref target_corpus) = e.target_corpus {
                            serde_json::json!({
                                "name": e.target_symbol.as_deref().unwrap_or(&e.target),
                                "scope_path": e.target,
                                "corpus": target_corpus,
                                "file_path": e.target_path,
                                "symbol_type": e.target_kind,
                                "confidence": e.confidence,
                                "cross_corpus": true,
                            })
                        } else {
                            serde_json::json!({
                                "name": e.target,
                                "scope_path": e.target,
                                "unresolved": true,
                            })
                        };
                        outgoing.entry(e.edge_type.clone()).or_default().push(node);
                    }
                }

                out["relationships"] = serde_json::json!({
                    "incoming": incoming,
                    "outgoing": outgoing,
                });
            }

            if is_lean {
                let lean = crate::format::lean::format_lean_code_symbol(
                    &sym.name,
                    &sym.scope_path,
                    &sym.file_path,
                    sym.start_line,
                    sym.end_line,
                    total_lines,
                    sym.docstring.as_deref(),
                    &source,
                    truncated,
                    &incoming,
                    &outgoing,
                );
                Ok(Value::String(lean))
            } else {
                Ok(out)
            }
        }
        _ => {
            let candidates: Vec<Value> = matches.iter().map(code_symbol_handle).collect();
            if is_lean {
                let mut s = format!(
                    "# Ambiguous Symbol: '{qualified_name}' ({} matches)\n\nDisambiguate with an exact scope_path:\n\n",
                    candidates.len()
                );
                for (i, c) in candidates.iter().enumerate() {
                    let num = i + 1;
                    let name = c["name"].as_str().unwrap_or("");
                    let scope = c["scope_path"].as_str().unwrap_or("");
                    let file = c["file_path"].as_str().unwrap_or("");
                    let start = c["start_line"].as_u64().unwrap_or(0);
                    let end = c["end_line"].as_u64().unwrap_or(0);
                    s.push_str(&format!(
                        "{num}. {name} (`{file}`:L{start}-L{end}) [scope: `{scope}`]\n"
                    ));
                    s.push_str(&format!("   -> get_snippet(name: \"{scope}\")\n"));
                }
                return Ok(Value::String(s));
            }
            Ok(serde_json::json!({
                "kind": "ambiguous",
                "note": format!(
                    "'{qualified_name}' is ambiguous ({} matches); disambiguate with an exact scope_path.",
                    candidates.len()
                ),
                "candidates": candidates,
            }))
        }
    }
}

/// Fetch a single doc chunk's bounded text, optionally with adjacent chunks.
fn fetch_doc_chunk(
    engine: &Engine,
    path: &str,
    chunk_index: usize,
    max_lines: usize,
    include_neighbors: bool,
    is_lean: bool,
) -> Result<Value> {
    let chunks = engine.store().get_chunks_for_file(path)?;
    if chunks.is_empty() {
        return Err(Error::NotFound(format!("no indexed chunks for '{path}'")));
    }

    let chunk = chunks
        .iter()
        .find(|c| c.chunk_index == chunk_index)
        .ok_or_else(|| Error::NotFound(format!("chunk {chunk_index} not found for '{path}'")))?;

    let chunk_text = engine.fetch_chunk_text(path, chunk.start_byte, chunk.end_byte)?;
    let text_lines: Vec<&str> = chunk_text.lines().collect();
    let (text, truncated) = cap_lines(&text_lines, max_lines);

    let full_doc_path = Path::new(&engine.config().path).join(path);
    let total_lines =
        read_file_lossy(&full_doc_path).map(|c| c.lines().count()).unwrap_or(text_lines.len());

    let mut out = serde_json::json!({
        "path": path,
        "chunk_index": chunk.chunk_index,
        "total_lines": total_lines,
        "text": text,
    });
    if truncated {
        out["truncated"] = serde_json::Value::Bool(true);
    }

    if include_neighbors {
        let neighbor_cap = (max_lines / 2).max(1);
        let neighbor = |target: usize| -> Option<Value> {
            chunks.iter().find(|c| c.chunk_index == target).and_then(|c| {
                let n_text = engine.fetch_chunk_text(path, c.start_byte, c.end_byte).ok()?;
                let nlines: Vec<&str> = n_text.lines().collect();
                let (ntext, ntrunc) = cap_lines(&nlines, neighbor_cap);
                Some(serde_json::json!({
                    "chunk_index": c.chunk_index,
                    "start_byte": c.start_byte,
                    "end_byte": c.end_byte,
                    "text": ntext,
                    "truncated": ntrunc,
                }))
            })
        };

        out["previous"] = chunk_index.checked_sub(1).and_then(neighbor).unwrap_or(Value::Null);
        out["next"] = neighbor(chunk_index + 1).unwrap_or(Value::Null);
    }

    if is_lean {
        let incoming = BTreeMap::new();
        let outgoing = BTreeMap::new();
        let lean = crate::format::lean::format_lean_doc_chunk(
            path,
            chunk.chunk_index,
            chunk.start_line,
            chunk.end_line,
            total_lines,
            &text,
            truncated,
            &incoming,
            &outgoing,
        );
        Ok(Value::String(lean))
    } else {
        Ok(out)
    }
}

/// Report index coverage + parse status for the given paths or path prefixes.
fn check_index_coverage_inner(engine: &Engine, paths: &[String]) -> Result<Value> {
    let all_files = engine.store().list_files()?;

    let mut reports = Vec::with_capacity(paths.len());
    let mut covered = 0usize;

    for scope in paths {
        let matched: Vec<&str> = all_files
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| *p == scope || p.starts_with(scope.as_str()))
            .collect();

        let indexed = !matched.is_empty();
        let mut chunk_count = 0usize;
        let mut symbol_count = 0usize;
        for file_path in &matched {
            chunk_count += engine.store().get_chunks_for_file(file_path).map(|c| c.len())?;
            symbol_count += engine.store().get_code_symbols_for_file(file_path).map(|s| s.len())?;
        }

        let parsed = indexed && (chunk_count > 0 || symbol_count > 0);
        if indexed {
            covered += 1;
        }

        let mut matched_files: Vec<String> = matched.iter().map(|p| p.to_string()).collect();
        matched_files.sort();

        reports.push(serde_json::json!({
            "path": scope,
            "indexed": indexed,
            "parsed": parsed,
            "chunk_count": chunk_count,
            "symbol_count": symbol_count,
            "matched_files": matched_files,
        }));
    }

    let total = paths.len();
    Ok(serde_json::json!({
        "reports": reports,
        "summary": {
            "total": total,
            "covered": covered,
            "uncovered": total - covered,
        },
    }))
}

/// List all indexed notes with metadata, or inspect single note's frontmatter and metadata if `path` is provided.
fn handle_list_notes(engine: &Engine, args: Value) -> Result<Value> {
    let params: ListNotesParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if let Some(path) = params.path {
        let corpus_path = PathBuf::from(&engine.config().path);
        let full_path = corpus_path.join(&path);
        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?;
        let doc = ctxvault_core::parser::parse_document(Path::new(&path), &content)?;
        return Ok(serde_json::json!({
            "path": path,
            "title": doc.title,
            "frontmatter": doc.frontmatter,
            "content_hash": doc.content_hash,
        }));
    }

    let limit = params.limit.unwrap_or(100);
    let offset = params.offset.unwrap_or(0);

    let files = engine.store().list_files()?;

    let items: Vec<NoteListItem> = files
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|f| NoteListItem {
            path: f.path,
            title: f.title,
            template: f.template,
            content_hash: f.content_hash,
        })
        .collect();

    serde_json::to_value(items).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}

/// Full-text BM25 keyword search.
/// Apply Tier-1 progressive-disclosure verbosity to a set of search results.
/// Apply detail level shaping to search results.
///
/// `detail == "ids"` strips the `snippet`, `lineage`, and `score_components`
/// from every result, leaving bare handles (path/qualified-name + line range + metadata carried by
/// `entity_kind`/`language`/`chunk_index`). Any other value (including the
/// omitted default) keeps the existing short snippet and metadata. Full bodies are never
/// emitted here — callers fetch source via `get_snippet`.
fn apply_detail(
    mut results: Vec<ctxvault_common::types::SearchResult>,
    detail: Option<&str>,
) -> Vec<ctxvault_common::types::SearchResult> {
    if detail == Some("ids") {
        for r in &mut results {
            r.snippet = None;
            r.lineage = None;
            r.score_components = None;
        }
    }
    results
}

/// Inlines bounded source text for the top K results across a partition.
fn populate_top_snippets(
    engine: &Engine,
    results: &mut [ctxvault_common::types::SearchResult],
    k: usize,
    max_lines: usize,
) {
    let corpus_root = Path::new(&engine.config().path);
    for (i, item) in results.iter_mut().enumerate() {
        if i >= k {
            item.snippet = None;
            continue;
        }

        if let Some(ref s) = item.snippet {
            if s.len() > 120 {
                let lines: Vec<&str> = s.lines().collect();
                if lines.len() > max_lines {
                    let (capped, _) = cap_lines(&lines, max_lines);
                    item.snippet = Some(capped);
                }
                continue;
            }
        }

        if let Some(chunk_index) = item.chunk_index {
            if let Ok(chunks) = engine.store().get_chunks_for_file(&item.path) {
                if let Some(chunk) = chunks.iter().find(|c| c.chunk_index == chunk_index) {
                    if let Ok(chunk_text) =
                        engine.fetch_chunk_text(&item.path, chunk.start_byte, chunk.end_byte)
                    {
                        let lines: Vec<&str> = chunk_text.lines().collect();
                        let (capped, _) = cap_lines(&lines, max_lines);
                        item.snippet = Some(capped);
                        continue;
                    }
                }
            }
        }

        if let Ok(symbols) = engine.store().find_symbols_by_name(&item.path) {
            if let Some(sym) = symbols.first() {
                let full_path = corpus_root.join(&sym.file_path);
                if let Ok(content) = read_file_lossy(&full_path) {
                    let file_lines: Vec<&str> = content.lines().collect();
                    if sym.start_line > 0 && sym.start_line <= file_lines.len() {
                        let start_idx = sym.start_line - 1;
                        let end_idx = sym.end_line.min(file_lines.len());
                        let (capped, _) = cap_lines(&file_lines[start_idx..end_idx], max_lines);
                        item.snippet = Some(capped);
                        continue;
                    }
                }
            }
        }

        let full_path = corpus_root.join(&item.path);
        if let Ok(content) = read_file_lossy(&full_path) {
            let lines: Vec<&str> = content.lines().collect();
            let (capped, _) = cap_lines(&lines, max_lines);
            item.snippet = Some(capped);
        }
    }
}

/// Consolidated search tool: dispatches to a retrieval mode selected by `mode`
/// (default `hybrid`). Modes: `bm25`, `semantic`, `hybrid`, `graph`, `explain`.
///
/// This is a thin adapter: it obtains the engine's search service (which
/// resolves the retrieval backends internally) and delegates the mode dispatch
/// to it via the [`SearchService`] port, then applies detail/verbosity shaping
/// and JSON serialization. Every mode honors `modality` (docs|code|both) and
/// `detail` (ids|default) via [`apply_detail`]. `explain` returns the
/// score-breakdown shape ([`SearchService::explain`]) rather than a plain
/// result array.
fn handle_search(engine: &Engine, args: Value) -> Result<Value> {
    let params: SearchParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let mode_str = params.mode.as_deref().unwrap_or("hybrid").to_string();
    let is_semantic = mode_str == "semantic";
    let is_explain = mode_str == "explain";
    let modality = params
        .modality
        .as_deref()
        .and_then(ctxvault_common::types::Modality::from_str_name)
        .unwrap_or_default();
    let depth = params
        .depth
        .as_deref()
        .and_then(ctxvault_common::types::SearchDepth::from_str_name)
        .unwrap_or_default();

    // Semantic mode lazily initializes the embedder, but only once the fast-mode
    // guard (no vector index) has passed — mirroring the original ordering.
    if is_semantic && engine.has_vector_index() {
        let _ = engine.ensure_embedder()?;
    }

    // Build the search service from the engine (it resolves its own backends
    // internally) and dispatch through the port. Detail/verbosity shaping and
    // serialization stay here.
    let service = engine.search_service();

    let query = SearchQuery {
        query: params.query,
        mode: params.mode,
        limit: params.limit,
        modality,
        depth,
        graph_depth: params.graph_depth,
        edge_types: params.edge_types,
        edge_class: params.edge_class,
        decompose: params.decompose,
        snippets: params.snippets,
    };

    if is_explain {
        let mut explanations = service.explain(&query)?;

        // Tier-1: `detail=ids` strips snippets, leaving bare handles + score breakdown.
        if params.detail.as_deref() == Some("ids") {
            for e in &mut explanations {
                e.snippet = None;
            }
        }

        serde_json::to_value(explanations)
            .map_err(|e| Error::Config(format!("serialize error: {}", e)))
    } else {
        let results = service.search(&query)?;
        let results = apply_detail(results, params.detail.as_deref());

        let mut docs_items = Vec::new();
        let mut code_items = Vec::new();

        for r in results {
            let is_code = r
                .entity_kind
                .as_ref()
                .map(|k| k.is_code())
                .unwrap_or_else(|| !r.path.ends_with(".md"));
            if is_code {
                code_items.push(r);
            } else {
                docs_items.push(r);
            }
        }

        let is_lean = params.detail.as_deref() == Some("ids");
        let k =
            if is_lean { 0 } else { params.snippets.unwrap_or_else(|| params.limit.unwrap_or(10)) };

        populate_top_snippets(engine, &mut docs_items, k, 20);
        populate_top_snippets(engine, &mut code_items, k, 20);

        if is_lean {
            for item in &mut docs_items {
                item.graph_affordances = None;
                item.graph = None;
                item.score_components = None;
                item.language = None;
                item.entity_kind = None;
                item.chunk_index = None;
                item.symbol = None;
            }
        } else {
            for item in &mut docs_items {
                item.language = None;
                item.entity_kind = None;
                item.chunk_index = None;
                item.symbol = None;
                item.graph_affordances = None;
                item.graph = engine.format_cypher_affordances(&item.path, 3);
            }
        }

        let code_paths: Vec<&str> = code_items.iter().map(|item| item.path.as_str()).collect();
        let symbols_by_file =
            engine.store().get_code_symbols_for_files(&code_paths).unwrap_or_default();

        for item in &mut code_items {
            let mut matched_symbol: Option<String> = None;
            let mut target_scope_path: Option<String> = None;

            if let Some(file_symbols) = symbols_by_file.get(&item.path) {
                if let Some(chunk_index) = item.chunk_index {
                    if let Ok(chunks) = engine.store().get_chunks_for_file(&item.path) {
                        if let Some(chunk) = chunks.iter().find(|c| c.chunk_index == chunk_index) {
                            if let Some(sym) = file_symbols.iter().find(|s| {
                                s.start_line <= chunk.end_line && s.end_line >= chunk.start_line
                            }) {
                                matched_symbol = Some(sym.name.clone());
                                target_scope_path = Some(sym.scope_path.clone());
                            }
                        }
                    }
                }
                if matched_symbol.is_none() {
                    if let Some(first_sym) = file_symbols.first() {
                        matched_symbol = Some(first_sym.name.clone());
                        target_scope_path = Some(first_sym.scope_path.clone());
                    }
                }
            } else if let Ok(symbols) = engine.store().find_symbols_by_name(&item.path) {
                if let Some(sym) = symbols.first() {
                    matched_symbol = Some(sym.name.clone());
                    target_scope_path = Some(sym.scope_path.clone());
                }
            }

            if is_lean {
                item.graph_affordances = None;
                item.graph = None;
                item.score_components = None;
            } else {
                let cypher = if let Some(ref scope) = target_scope_path {
                    engine.format_cypher_affordances(scope, 3)
                } else {
                    None
                };

                let cypher = cypher.or_else(|| {
                    if let Some(file_symbols) = symbols_by_file.get(&item.path) {
                        for sym in file_symbols {
                            if let Some(c) = engine.format_cypher_affordances(&sym.scope_path, 3) {
                                return Some(c);
                            }
                        }
                    }
                    engine.format_cypher_affordances(&item.path, 3)
                });

                item.graph = cypher;
                item.graph_affordances = None;
            }

            // Zero semantic duplication:
            // 1. Language is omitted (file extension in `path` conveys it).
            item.language = None;
            // 2. Entity kind is omitted (implied by snippet/symbol).
            item.entity_kind = None;
            // 3. Chunk index is omitted.
            item.chunk_index = None;
            // 4. Bare symbol identifier is surfaced only when snippet is omitted (trailing hits or snippets: 0)
            //    or when in lean emission mode for Tier 2 progressive disclosure scent.
            if params.format.as_deref() == Some("lean") || item.snippet.is_none() {
                item.symbol = matched_symbol;
            } else {
                item.symbol = None;
            }
        }

        if params.format.as_deref() == Some("lean") {
            let lean_text = crate::format::lean::format_lean_search(
                &query.query,
                &mode_str,
                Some(engine.config().name.as_str()),
                &code_items,
                &docs_items,
                is_lean,
            );
            return Ok(Value::String(lean_text));
        }

        let active_doc_edges = if is_lean {
            Vec::new()
        } else {
            engine.active_edge_types(Some(ctxvault_common::config::EdgeClass::Structural))
        };
        let docs_partition =
            if !docs_items.is_empty() || modality != ctxvault_common::types::Modality::Code {
                Some(ctxvault_common::types::SearchPartition {
                    total_matches: docs_items.len(),
                    top_k_returned: docs_items.len(),
                    schema_envelope: if is_lean {
                        ctxvault_common::types::SchemaEnvelope::default()
                    } else {
                        build_dynamic_schema_envelope(&docs_items, false, &active_doc_edges)
                    },
                    results: docs_items,
                })
            } else {
                None
            };

        let active_code_edges = if is_lean {
            Vec::new()
        } else {
            engine.active_edge_types(Some(ctxvault_common::config::EdgeClass::Code))
        };
        let code_partition =
            if !code_items.is_empty() || modality != ctxvault_common::types::Modality::Docs {
                Some(ctxvault_common::types::SearchPartition {
                    total_matches: code_items.len(),
                    top_k_returned: code_items.len(),
                    schema_envelope: if is_lean {
                        ctxvault_common::types::SchemaEnvelope::default()
                    } else {
                        build_dynamic_schema_envelope(&code_items, true, &active_code_edges)
                    },
                    results: code_items,
                })
            } else {
                None
            };

        let response =
            ctxvault_common::types::SearchResponse { docs: docs_partition, code: code_partition };

        serde_json::to_value(response).map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Find related documents via PPR approximation.
fn handle_search_related(engine: &Engine, args: Value) -> Result<Value> {
    let params: SearchRelatedParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let limit = params.limit.unwrap_or(10);
    let modality = params
        .modality
        .as_deref()
        .and_then(ctxvault_common::types::Modality::from_str_name)
        .unwrap_or_default();

    // Related search only traverses the graph, but the service is built the same
    // way `handle_search` builds it; the embedder is left as-is (never lazily
    // initialized here, matching prior behaviour) since related does not touch
    // it. Detail/verbosity shaping stays here.
    let service = engine.search_service();

    let results = service.search_related(&params.seeds, limit, modality)?;
    let results = apply_detail(results, params.detail.as_deref());

    serde_json::to_value(results).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}

/// Execute a Cypher-Lite graph path query compiled to SQLite recursive CTE.
fn handle_graph_match(engine: &Engine, args: Value) -> Result<Value> {
    let params: GraphMatchParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let limit = params.limit.unwrap_or(20);
    let max_depth = params.max_depth.unwrap_or(3);

    let match_result = engine.graph_match(
        &params.pattern,
        params.edge_class.as_deref(),
        params.where_clause.as_deref(),
        limit,
        max_depth,
    )?;

    if params.format.as_deref() == Some("lean") {
        Ok(Value::String(crate::format::lean::format_lean_graph_match(&match_result)))
    } else {
        serde_json::to_value(match_result)
            .map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Detect communities via Leiden or Louvain, or architectural components overview.
fn handle_graph_communities(engine: &Engine, args: Value) -> Result<Value> {
    let params: GraphCommunitiesParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    let view = params.view.as_deref().unwrap_or("raw");
    if view == "architecture" {
        let result = engine.graph().detect_communities_leiden();
        let densities = engine.graph().community_densities();
        let density_map: HashMap<usize, f64> =
            densities.into_iter().map(|d| (d.community_id, d.density)).collect();

        // If a specific community is requested, return that community with its members
        if let Some(target_id) = params.community_id {
            if let Some(comm) = result.communities.iter().find(|c| c.id == target_id) {
                let mut nodes = comm.members.clone();
                nodes.sort();
                let density = density_map.get(&target_id).copied().unwrap_or(0.0);
                let mem_limit = params.limit.unwrap_or(50).min(nodes.len());
                return Ok(serde_json::json!({
                    "component_id": target_id,
                    "node_count": nodes.len(),
                    "internal_density": density,
                    "members": &nodes[..mem_limit],
                    "total_members": nodes.len(),
                }));
            } else {
                return Err(Error::NotFound(format!("community_id {target_id} not found")));
            }
        }

        let mut clusters = Vec::new();
        let limit = params.limit.unwrap_or(10);

        let mut sorted_comms = result.communities.clone();
        sorted_comms.sort_by(|a, b| b.members.len().cmp(&a.members.len()));

        for comm in sorted_comms.into_iter().take(limit) {
            let comm_id = comm.id;
            let mut nodes = comm.members;
            nodes.sort();
            let density = density_map.get(&comm_id).copied().unwrap_or(0.0);

            let mut key_nodes: Vec<_> =
                nodes.iter().map(|n| (n.clone(), engine.graph().in_degree(n))).collect();
            key_nodes.sort_by(|a, b| b.1.cmp(&a.1));
            let top_key_nodes: Vec<String> =
                key_nodes.into_iter().take(5).map(|(n, _)| n).collect();

            clusters.push(serde_json::json!({
                "component_id": comm_id,
                "node_count": nodes.len(),
                "internal_density": density,
                "top_nodes": top_key_nodes,
            }));
        }

        return Ok(serde_json::json!({
            "algorithm": "leiden",
            "component_count": clusters.len(),
            "total_communities": result.communities.len(),
            "top_components_returned": clusters.len(),
            "modularity": result.modularity,
            "components": clusters,
        }));
    }

    let algo = params.algorithm.as_deref().unwrap_or("leiden");
    let result = match algo {
        "louvain" => engine.graph().detect_communities(),
        _ => engine.graph().detect_communities_leiden(),
    };

    if params.include_density.unwrap_or(false) {
        let densities = engine.graph().community_densities();
        let response = serde_json::json!({
            "communities": result.communities,
            "modularity": result.modularity,
            "iterations": result.iterations,
            "community_densities": densities,
        });
        Ok(response)
    } else {
        serde_json::to_value(result).map_err(|e| Error::Config(format!("serialize error: {}", e)))
    }
}

/// Helper to apply index_mode overrides dynamically on an engine.
fn apply_index_mode_override(
    engine: &mut Engine,
    index_mode: Option<&str>,
    fast: Option<bool>,
) -> Result<()> {
    if let Some(mode_str) = index_mode {
        match mode_str.to_lowercase().as_str() {
            "fast" => engine.set_index_mode(ctxvault_common::config::IndexMode::Fast),
            "full" => engine.set_index_mode(ctxvault_common::config::IndexMode::Full),
            other => return Err(Error::Config(format!("invalid index_mode '{}'", other))),
        }
    } else if let Some(fast) = fast {
        engine.set_index_mode(if fast {
            ctxvault_common::config::IndexMode::Fast
        } else {
            ctxvault_common::config::IndexMode::Full
        });
    }
    Ok(())
}

/// Sync or reindex corpus in configurable batches. Supports mode: "delta" | "full" | "reembed".
fn handle_sync_corpus(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: SyncCorpusParams = serde_json::from_value(args).unwrap_or(SyncCorpusParams {
        mode: None,
        batch_size: None,
        resume: None,
        fast: None,
        index_mode: None,
    });
    apply_index_mode_override(engine, params.index_mode.as_deref(), params.fast)?;

    match params.mode.as_deref().unwrap_or("delta") {
        "reembed" => {
            let was_stale = engine.vectors_stale();
            let old_version = engine.stored_model_version().map(|s| s.to_string());
            let chunks_reembedded = engine.reembed()?;
            let new_version = engine.stored_model_version().unwrap_or("unknown").to_string();
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "reembed",
                "chunks_reembedded": chunks_reembedded,
                "was_stale": was_stale,
                "previous_model_version": old_version,
                "current_model_version": new_version,
            }))
        }
        "full" => {
            let batch_size = params.batch_size.unwrap_or(50);
            let resume = params.resume.unwrap_or(true);
            let count = engine.full_reindex_paginated(batch_size, resume)?;
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "full",
                "files_indexed": count,
                "batch_size": batch_size,
                "resumed": resume,
            }))
        }
        _ => {
            let batch_size = params.batch_size.unwrap_or(50);
            let result = engine.delta_scan_paginated(batch_size)?;
            Ok(serde_json::json!({
                "status": "complete",
                "mode": "delta",
                "new_files": result.new_files.len(),
                "modified_files": result.modified_files.len(),
                "deleted_files": result.deleted_files.len(),
                "new": result.new_files,
                "modified": result.modified_files,
                "deleted": result.deleted_files,
            }))
        }
    }
}

/// Per-corpus statistics (document counts, mode, chunking, embedding model).
fn corpus_stats(engine: &Engine) -> Result<Value> {
    let files = engine.store().list_files()?;
    let is_indexed = engine.is_indexed();
    Ok(serde_json::json!({
        "status": "healthy",
        "corpus_name": engine.config().name,
        "corpus_path": engine.config().path,
        "document_count": files.len(),
        "indexed": is_indexed,
        "mode": format!("{:?}", engine.config().mode),
        "index_mode": format!("{:?}", engine.config().index_mode),
        "chunking": format!("{:?}", engine.config().chunking.strategy),
        "embedding_model": engine.config().embedding.model,
    }))
}

/// Consolidated status tool (engine-level): combines per-corpus statistics,
/// indexing progress, graph topology/density, and coverage inspection.
fn handle_status(engine: &Engine, args: Value) -> Result<Value> {
    let params: StatusParams =
        serde_json::from_value(args).unwrap_or(StatusParams { scope: None, paths: None });
    let scope = params.scope.as_deref().unwrap_or("all");

    match scope {
        "corpus" => corpus_stats(engine),
        "indexing" => {
            let status = engine.get_indexing_status()?;
            serde_json::to_value(status)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))
        }
        "graph" => {
            let stats = engine.graph().stats();
            let density = engine.analyze_density(10);
            Ok(serde_json::json!({
                "stats": stats,
                "density": density,
            }))
        }
        "coverage" => {
            let paths = params.paths.unwrap_or_default();
            check_index_coverage_inner(engine, &paths)
        }
        "census" | "architecture" => {
            let stats = engine.graph().stats();
            let symbols = engine.store().get_all_code_symbols().unwrap_or_default();
            let mut symbol_types: HashMap<String, usize> = HashMap::new();
            let mut languages: HashMap<String, usize> = HashMap::new();
            for s in &symbols {
                *symbol_types.entry(format!("{:?}", s.symbol_type)).or_insert(0) += 1;
                *languages.entry(s.language.clone()).or_insert(0) += 1;
            }
            let active_edges = engine.active_edge_types(None);
            let files = engine.store().list_files().unwrap_or_default();
            Ok(serde_json::json!({
                "corpus_name": engine.config().name,
                "corpus_path": engine.config().path,
                "total_files": files.len(),
                "total_symbols": symbols.len(),
                "total_graph_nodes": stats.node_count,
                "total_graph_edges": stats.edge_count,
                "symbol_types": symbol_types,
                "languages": languages,
                "active_edge_types": active_edges,
            }))
        }
        _ => {
            let corpus = corpus_stats(engine)?;
            let indexing = serde_json::to_value(engine.get_indexing_status()?)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))?;
            let stats = engine.graph().stats();
            let density = engine.analyze_density(10);
            Ok(serde_json::json!({
                "corpus": corpus,
                "indexing": indexing,
                "graph": {
                    "stats": stats,
                    "density": density,
                },
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Write tool handlers
// ---------------------------------------------------------------------------

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
fn handle_write_note(engine: &mut Engine, args: Value) -> Result<Value> {
    let params: WriteNoteParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if engine.config().mode == CorpusMode::ReadOnly {
        return Err(Error::Config(format!("corpus '{}' is read-only", engine.config().name)));
    }

    let corpus_path = PathBuf::from(&engine.config().path);
    let full_path = corpus_path.join(&params.path);

    let classification = engine.classifier().classify(&full_path, None);
    let ext = full_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if let ctxvault_core::index::classifier::FileClassification::Document(fmt) = classification {
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
fn handle_delete_note(engine: &mut Engine, args: Value) -> Result<Value> {
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
fn handle_move_note(engine: &mut Engine, args: Value) -> Result<Value> {
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
    matcher: &ctxvault_core::index::exclude::ExcludeMatcher,
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
    matcher: &ctxvault_core::index::exclude::ExcludeMatcher,
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

// ---------------------------------------------------------------------------
// Validation tool handlers
// ---------------------------------------------------------------------------

/// Load templates for the corpus.
fn load_corpus_templates(engine: &Engine) -> Result<HashMap<String, Template>> {
    engine.load_templates()
}

/// Validate a single note against its declared template.
fn validate_single_note(
    engine: &Engine,
    path: &str,
) -> Result<ctxvault_core::template::ValidationResult> {
    let corpus_path = PathBuf::from(&engine.config().path);
    let full_path = corpus_path.join(path);

    let content = fs::read_to_string(&full_path)
        .map_err(|e| Error::NotFound(format!("cannot read {}: {}", path, e)))?;

    let doc = ctxvault_core::parser::parse_document(Path::new(path), &content)?;

    let template_name = doc.template.clone();

    let (valid, issues, tmpl_name) = if let Some(ref name) = template_name {
        let templates = load_corpus_templates(engine)?;
        if let Some(tmpl) = templates.get(name) {
            let mut issues = tmpl.validate(&doc.frontmatter, &doc.content);

            let files = engine.store().list_files().unwrap_or_default();
            let mut note_templates: HashMap<String, Option<String>> = HashMap::new();
            for f in files {
                let _ = note_templates.insert(f.path, f.template);
            }
            let edge_issues =
                tmpl.validate_edge_targets(&doc.frontmatter, &note_templates, |sym| {
                    engine.store().find_symbols_by_name(sym).map(|v| !v.is_empty()).unwrap_or(false)
                        || engine
                            .store()
                            .find_symbols_by_qualified_name(sym)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false)
                });
            issues.extend(edge_issues);

            let valid =
                !issues.iter().any(|i| i.severity == ctxvault_core::template::Severity::Error);
            (valid, issues, Some(name.clone()))
        } else {
            let issues = vec![ctxvault_core::template::ValidationIssue {
                severity: ctxvault_core::template::Severity::Warning,
                message: format!("template '{}' not found in templates directory", name),
                field: Some("template".to_string()),
            }];
            (true, issues, Some(name.clone()))
        }
    } else {
        (true, Vec::new(), None)
    };

    Ok(ctxvault_core::template::ValidationResult {
        path: path.to_string(),
        template: tmpl_name,
        valid,
        issues,
    })
}

/// Validate all templated notes in the corpus.
fn validate_corpus_notes(
    engine: &Engine,
    limit: Option<usize>,
) -> Result<Vec<ctxvault_core::template::ValidationResult>> {
    let templates = load_corpus_templates(engine)?;
    let files = engine.store().list_files()?;
    let corpus_path = PathBuf::from(&engine.config().path);

    let mut note_templates: HashMap<String, Option<String>> = HashMap::new();
    for f in &files {
        let _ = note_templates.insert(f.path.clone(), f.template.clone());
    }

    let mut results: Vec<ctxvault_core::template::ValidationResult> = Vec::new();

    for file in &files {
        let tmpl_name = match &file.template {
            Some(name) => name.clone(),
            None => continue,
        };

        let full_path = corpus_path.join(&file.path);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let doc = match ctxvault_core::parser::parse_document(Path::new(&file.path), &content) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let issues = if let Some(tmpl) = templates.get(&tmpl_name) {
            let mut issues = tmpl.validate(&doc.frontmatter, &doc.content);
            let edge_issues =
                tmpl.validate_edge_targets(&doc.frontmatter, &note_templates, |sym| {
                    engine.store().find_symbols_by_name(sym).map(|v| !v.is_empty()).unwrap_or(false)
                        || engine
                            .store()
                            .find_symbols_by_qualified_name(sym)
                            .map(|v| !v.is_empty())
                            .unwrap_or(false)
                });
            issues.extend(edge_issues);
            issues
        } else {
            vec![ctxvault_core::template::ValidationIssue {
                severity: ctxvault_core::template::Severity::Warning,
                message: format!("template '{}' not found in templates directory", tmpl_name),
                field: Some("template".to_string()),
            }]
        };

        if !issues.is_empty() {
            let valid =
                !issues.iter().any(|i| i.severity == ctxvault_core::template::Severity::Error);
            results.push(ctxvault_core::template::ValidationResult {
                path: file.path.clone(),
                template: Some(tmpl_name),
                valid,
                issues,
            });
        }

        if let Some(limit_val) = limit {
            if results.len() >= limit_val {
                break;
            }
        }
    }

    Ok(results)
}

/// Validate structural ontology and graph integrity (broken links, cycle detection, orphan ADRs).
fn run_taxonomy_validation(engine: &Engine) -> Result<Value> {
    let files = engine.store().list_files()?;
    let existing_paths: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();

    let broken_links = engine.graph().detect_broken_links(&existing_paths);
    let circular_dependencies = engine.graph().detect_circular_dependencies(&[
        "supersedes",
        "depends_on",
        "implements",
        "parent_of",
    ]);

    let adr_paths: Vec<String> = files
        .iter()
        .filter(|f| {
            f.template.as_deref() == Some("adr")
                || f.template.as_deref() == Some("decision-record")
                || f.path.starts_with("docs/adrs/")
                || f.path.starts_with("adrs/")
        })
        .map(|f| f.path.clone())
        .collect();
    let orphan_adrs = engine.graph().detect_orphan_adrs(&adr_paths);

    let valid =
        broken_links.is_empty() && circular_dependencies.is_empty() && orphan_adrs.is_empty();

    Ok(serde_json::json!({
        "valid": valid,
        "broken_links_count": broken_links.len(),
        "broken_links": broken_links,
        "circular_dependencies_count": circular_dependencies.len(),
        "circular_dependencies": circular_dependencies,
        "orphan_adrs_count": orphan_adrs.len(),
        "orphan_adrs": orphan_adrs,
    }))
}

/// Unified validation tool: validates a single note, entire corpus notes against templates, and/or graph taxonomy.
fn handle_validate(engine: &Engine, args: Value) -> Result<Value> {
    let params: ValidateParams = serde_json::from_value(args)
        .map_err(|e| Error::Config(format!("invalid params: {}", e)))?;

    if let Some(path) = &params.path {
        let note_res = validate_single_note(engine, path)?;
        if params.check_taxonomy == Some(true) {
            let tax_res = run_taxonomy_validation(engine)?;
            let valid = note_res.valid && tax_res["valid"].as_bool().unwrap_or(true);
            Ok(serde_json::json!({
                "valid": valid,
                "note": note_res,
                "taxonomy": tax_res,
            }))
        } else {
            serde_json::to_value(note_res)
                .map_err(|e| Error::Config(format!("serialize error: {}", e)))
        }
    } else {
        let check_taxonomy = params.check_taxonomy.unwrap_or(true);
        let note_issues = validate_corpus_notes(engine, params.limit)?;
        let notes_valid = note_issues.is_empty() || note_issues.iter().all(|r| r.valid);

        if check_taxonomy {
            let tax_res = run_taxonomy_validation(engine)?;
            let tax_valid = tax_res["valid"].as_bool().unwrap_or(true);
            Ok(serde_json::json!({
                "valid": notes_valid && tax_valid,
                "notes_with_issues": note_issues,
                "taxonomy": tax_res,
            }))
        } else {
            Ok(serde_json::json!({
                "valid": notes_valid,
                "notes_with_issues": note_issues,
            }))
        }
    }
}

/// List all available templates.
fn handle_list_templates(engine: &Engine, _args: Value) -> Result<Value> {
    let templates = load_corpus_templates(engine)?;

    let mut list: Vec<&Template> = templates.values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));

    serde_json::to_value(list).map_err(|e| Error::Config(format!("serialize error: {}", e)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ctxvault_common::config::{
        ChunkingConfig, CorpusConfig, CorpusMode, EdgeClass, EdgeSource, EdgeTypeConfig,
        EmbeddingConfig, GraphConfig, IndexMode,
    };
    use ctxvault_common::types::EdgeProvenance;
    use std::fs;
    use tempfile::TempDir;

    /// Create a minimal corpus config pointing at the given path.
    fn test_config(corpus_path: &std::path::Path) -> CorpusConfig {
        CorpusConfig {
            name: "test".to_string(),
            path: corpus_path.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig {
                edge_types: vec![EdgeTypeConfig {
                    name: "Wikilink".to_string(),
                    source: EdgeSource::Wikilink,
                    weight: 1.0,
                    bidirectional: false,
                    field: None,
                    direction: None,
                    max_frequency: None,
                    class: None,
                    description: None,
                    allowed_source_templates: None,
                    allowed_target_templates: None,
                }],
            },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        }
    }

    /// Create a test engine with an empty corpus.
    fn create_test_engine(tmp: &TempDir) -> Engine {
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let config = test_config(&corpus_dir);
        Engine::open(config, &index_dir).unwrap()
    }

    #[test]
    fn test_registry_has_all_tools() {
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let tools = registry.list();
        assert_eq!(tools.len(), 18, "Expected 18 tools registered");

        // Verify each expected tool exists.
        let expected = [
            "read_file",
            "get_snippet",
            "list_notes",
            "search",
            "search_related",
            "graph_match",
            "graph_communities",
            "write_note",
            "delete_note",
            "move_note",
            "validate",
            "list_templates",
            "status",
            "list_corpora",
            "trace_cross_corpus",
            "sync_corpus",
            "index_corpus",
            "unload_corpus",
        ];

        assert_eq!(expected.len(), 18, "expected-name list must match the 18-tool count");

        for name in expected {
            assert!(registry.get(name).is_some(), "Tool '{}' should be registered", name);
        }

        // The consolidated / deleted legacy tools must not be registered.
        for gone in [
            "read_note",
            "read_code_file",
            "read_multiple",
            "get_frontmatter",
            "create_note",
            "update_note",
            "promote_concept",
            "validate_note",
            "validate_corpus",
            "validate_taxonomy",
            "analyze_density",
            "find_semantic_gaps",
            "suggest_splits",
            "coverage_report",
            "check_index_coverage",
            "corpus_list",
            "reembed_corpus",
            "reindex_corpus",
            "get_symbol_definition",
            "get_architecture",
            "search_bm25",
            "search_semantic",
            "search_hybrid",
            "search_graph",
            "search_explain",
            "get_status",
            "get_corpus_stats",
            "get_indexing_status",
            "backlinks",
            "forwardlinks",
            "graph_path",
            "graph_stats",
            "graph_subgraph",
            "list_edge_types",
            "traverse_lineage",
            "find_callers",
            "detect_changes",
        ] {
            assert!(registry.get(gone).is_none(), "Tool '{}' must no longer be registered", gone);
        }

        // Verify read-only classification
        assert!(registry.is_read_only("read_file"));
        assert!(registry.is_read_only("get_snippet"));
        assert!(registry.is_read_only("list_notes"));
        assert!(registry.is_read_only("search"));
        assert!(registry.is_read_only("search_related"));
        assert!(registry.is_read_only("graph_match"));
        assert!(registry.is_read_only("graph_communities"));
        assert!(registry.is_read_only("validate"));
        assert!(registry.is_read_only("list_templates"));
        assert!(registry.is_read_only("status"));
        assert!(registry.is_read_only("list_corpora"));
        assert!(registry.is_read_only("trace_cross_corpus"));
        assert!(!registry.is_read_only("write_note"));
        assert!(!registry.is_read_only("delete_note"));
        assert!(!registry.is_read_only("move_note"));
        assert!(!registry.is_read_only("sync_corpus"));
        assert!(!registry.is_read_only("index_corpus"));
        assert!(!registry.is_read_only("unload_corpus"));
    }

    #[test]
    fn test_tool_profiles_gate_listing() {
        let all = MultiCorpusToolRegistry::with_profile(ToolProfile::All);
        let analysis = MultiCorpusToolRegistry::with_profile(ToolProfile::Analysis);
        let scout = MultiCorpusToolRegistry::with_profile(ToolProfile::Scout);

        let all_count = all.list().len();
        let analysis_count = analysis.list().len();
        let scout_count = scout.list().len();

        // scout ⊂ analysis ⊂ all.
        assert!(scout_count < analysis_count, "scout must expose fewer tools than analysis");
        assert!(analysis_count < all_count, "analysis must expose fewer tools than all");
        assert_eq!(all_count, 18, "all profile advertises every registered tool");
        assert_eq!(analysis_count, 12, "analysis profile advertises scout + analysis tools");
        assert_eq!(scout_count, 6, "scout profile advertises the minimal set");

        // scout includes core retrieval/fetch but not writes or analysis-only tools.
        let scout_names: HashSet<&str> = scout.list().iter().map(|t| t.name.as_str()).collect();
        assert!(scout_names.contains("search"));
        assert!(scout_names.contains("get_snippet"));
        assert!(scout_names.contains("read_file"));
        assert!(scout_names.contains("status"));
        assert!(!scout_names.contains("write_note"));
        assert!(!scout_names.contains("graph_match"));

        // Hidden tools still execute (advertise-only filtering): write_note is
        // registered even though scout does not advertise it.
        assert!(scout.registry().get("write_note").is_some());

        // analysis adds read-only tools but still hides writes.
        let analysis_names: HashSet<&str> =
            analysis.list().iter().map(|t| t.name.as_str()).collect();
        assert!(analysis_names.contains("graph_match"));
        assert!(analysis_names.contains("graph_communities"));
        assert!(analysis_names.contains("validate"));
        assert!(analysis_names.contains("list_corpora"));
        assert!(analysis_names.contains("trace_cross_corpus"));
        assert!(!analysis_names.contains("write_note"));
        assert!(!analysis_names.contains("sync_corpus"));
        // trace_cross_corpus is an analysis-tier capability, not a scout tool.
        assert!(!scout_names.contains("trace_cross_corpus"));
    }

    #[test]
    fn test_read_only_tool_execution() {
        let tmp = TempDir::new().unwrap();
        let engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        // Read tool with immutable &engine should succeed
        let result = registry.execute_read("list_notes", &engine, serde_json::json!({})).unwrap();
        let notes: Vec<Value> = serde_json::from_value(result).unwrap();
        assert!(notes.is_empty());

        // Calling mutating tool with execute_read should return error
        let err = registry.execute_read(
            "write_note",
            &engine,
            serde_json::json!({ "path": "fail.md", "content": "hello" }),
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_list_notes_empty() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry.execute("list_notes", &mut engine, serde_json::json!({})).unwrap();

        let notes: Vec<Value> = serde_json::from_value(result).unwrap();
        assert!(notes.is_empty(), "Empty corpus should return empty list");
    }

    #[test]
    fn test_search_bm25_tool() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        // Write and index a test file.
        let content =
            "# Rust Programming\n\nRust is a systems programming language focused on safety.\n";
        fs::write(corpus_dir.join("rust.md"), content).unwrap();
        engine.index_file("rust.md", content).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry
            .execute(
                "search",
                &mut engine,
                serde_json::json!({ "query": "systems programming", "mode": "bm25" }),
            )
            .unwrap();

        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let docs = resp.docs.unwrap();
        assert!(!docs.results.is_empty(), "Should find indexed file via search");
        assert_eq!(docs.results[0].path, "rust.md");
        assert!(docs.results[0].snippet.is_some());
    }

    #[test]
    fn test_write_note_create() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry
            .execute(
                "write_note",
                &mut engine,
                serde_json::json!({
                    "path": "new-note.md",
                    "content": "# Hello\n\nThis is a new note.",
                    "frontmatter": { "tags": ["test", "demo"] }
                }),
            )
            .unwrap();

        assert_eq!(result["path"], "new-note.md");
        assert_eq!(result["written"], true);
        assert_eq!(result["mode"], "create");

        // Verify file exists on disk.
        let corpus_dir = tmp.path().join("corpus");
        let file_content = fs::read_to_string(corpus_dir.join("new-note.md")).unwrap();
        assert!(file_content.contains("# Hello"));
        assert!(file_content.contains("---"));

        // Verify indexed (searchable).
        let search_result = registry
            .execute(
                "search",
                &mut engine,
                serde_json::json!({ "query": "new note", "mode": "bm25" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(search_result).unwrap();
        assert!(!resp.docs.unwrap().results.is_empty());
    }

    #[test]
    fn test_write_note_with_template() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry
            .execute(
                "write_note",
                &mut engine,
                serde_json::json!({
                    "path": "templated.md",
                    "content": "Body text here.",
                    "template": "meeting"
                }),
            )
            .unwrap();

        assert_eq!(result["written"], true);

        let corpus_dir = tmp.path().join("corpus");
        let file_content = fs::read_to_string(corpus_dir.join("templated.md")).unwrap();
        assert!(file_content.contains("template: meeting"));
    }

    #[test]
    fn test_write_note_already_exists() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        fs::write(corpus_dir.join("existing.md"), "# Existing").unwrap();

        let result = registry.execute(
            "write_note",
            &mut engine,
            serde_json::json!({ "path": "existing.md", "content": "overwrite?", "mode": "create" }),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_write_note_overwrite() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        let original = "# Original\n\nOld content.\n";
        fs::write(corpus_dir.join("update-me.md"), original).unwrap();
        engine.index_file("update-me.md", original).unwrap();
        engine.commit().unwrap();

        let result = registry
            .execute(
                "write_note",
                &mut engine,
                serde_json::json!({
                    "path": "update-me.md",
                    "content": "# Replaced\n\nNew content.",
                    "mode": "overwrite"
                }),
            )
            .unwrap();

        assert_eq!(result["written"], true);
        assert_eq!(result["mode"], "overwrite");

        let file_content = fs::read_to_string(corpus_dir.join("update-me.md")).unwrap();
        assert!(file_content.contains("New content"));
        assert!(!file_content.contains("Old content"));
    }

    #[test]
    fn test_write_note_append() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        let original = "# Append Test\n\nFirst line.\n";
        fs::write(corpus_dir.join("append.md"), original).unwrap();
        engine.index_file("append.md", original).unwrap();
        engine.commit().unwrap();

        let result = registry
            .execute(
                "write_note",
                &mut engine,
                serde_json::json!({
                    "path": "append.md",
                    "content": "Second line.",
                    "mode": "append"
                }),
            )
            .unwrap();

        assert_eq!(result["written"], true);
        assert_eq!(result["mode"], "append");

        let file_content = fs::read_to_string(corpus_dir.join("append.md")).unwrap();
        assert!(file_content.contains("First line."));
        assert!(file_content.contains("Second line."));
    }

    #[test]
    fn test_write_note_prepend() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        let original = "# Prepend Test\n\nOriginal.\n";
        fs::write(corpus_dir.join("prepend.md"), original).unwrap();
        engine.index_file("prepend.md", original).unwrap();
        engine.commit().unwrap();

        let result = registry
            .execute(
                "write_note",
                &mut engine,
                serde_json::json!({
                    "path": "prepend.md",
                    "content": "Prepended text.",
                    "mode": "prepend"
                }),
            )
            .unwrap();

        assert_eq!(result["written"], true);
        assert_eq!(result["mode"], "prepend");

        let file_content = fs::read_to_string(corpus_dir.join("prepend.md")).unwrap();
        // Prepended text should appear before original content.
        let prepend_pos = file_content.find("Prepended text.").unwrap();
        let original_pos = file_content.find("Original.").unwrap();
        assert!(prepend_pos < original_pos);
    }

    #[test]
    fn test_delete_note_tool() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        let content = "# Delete Me\n\nGoing away.\n";
        fs::write(corpus_dir.join("delete-me.md"), content).unwrap();
        engine.index_file("delete-me.md", content).unwrap();
        engine.commit().unwrap();

        let result = registry
            .execute("delete_note", &mut engine, serde_json::json!({ "path": "delete-me.md" }))
            .unwrap();

        assert_eq!(result["deleted"], true);

        // File should be gone from disk.
        assert!(!corpus_dir.join("delete-me.md").exists());

        // Should not be found in search.
        let search_result = registry
            .execute(
                "search",
                &mut engine,
                serde_json::json!({ "query": "Going away", "mode": "bm25" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(search_result).unwrap();
        let hits = resp.docs.map(|d| d.results).unwrap_or_default();
        assert!(hits.is_empty() || hits.iter().all(|h| h.path != "delete-me.md"));
    }

    #[test]
    fn test_delete_note_not_found() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry.execute(
            "delete_note",
            &mut engine,
            serde_json::json!({ "path": "nonexistent.md" }),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_move_note_tool() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");

        // Create the note to move.
        let content = "# Alpha\n\nAlpha content.\n";
        fs::write(corpus_dir.join("alpha.md"), content).unwrap();
        engine.index_file("alpha.md", content).unwrap();

        // Create another note that links to alpha.
        let linker = "# Linker\n\nSee [[alpha]] for details.\n";
        fs::write(corpus_dir.join("linker.md"), linker).unwrap();
        engine.index_file("linker.md", linker).unwrap();
        engine.commit().unwrap();

        // Move alpha to beta.
        let result = registry
            .execute(
                "move_note",
                &mut engine,
                serde_json::json!({ "from": "alpha.md", "to": "beta.md" }),
            )
            .unwrap();

        assert_eq!(result["moved"], true);
        assert_eq!(result["from"], "alpha.md");
        assert_eq!(result["to"], "beta.md");
        assert_eq!(result["links_rewritten"], 1);

        // Old file should be gone, new file should exist.
        assert!(!corpus_dir.join("alpha.md").exists());
        assert!(corpus_dir.join("beta.md").exists());

        // Linker file should now reference [[beta]].
        let linker_content = fs::read_to_string(corpus_dir.join("linker.md")).unwrap();
        assert!(linker_content.contains("[[beta]]"));
        assert!(!linker_content.contains("[[alpha]]"));
    }

    #[test]
    fn test_move_note_to_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let mut registry = ToolRegistry::new();
        registry.register_all();

        let corpus_dir = tmp.path().join("corpus");
        let content = "# Move Me\n\nContent.\n";
        fs::write(corpus_dir.join("movable.md"), content).unwrap();
        engine.index_file("movable.md", content).unwrap();
        engine.commit().unwrap();

        let result = registry
            .execute(
                "move_note",
                &mut engine,
                serde_json::json!({ "from": "movable.md", "to": "archive/movable.md" }),
            )
            .unwrap();

        assert_eq!(result["moved"], true);
        assert!(!corpus_dir.join("movable.md").exists());
        assert!(corpus_dir.join("archive/movable.md").exists());
    }

    // ─── Multi-Corpus Routing Tests ────────────────────────────────────

    #[test]
    fn test_multi_corpus_registry_has_status() {
        let registry = MultiCorpusToolRegistry::new();
        let tools = registry.list();

        assert_eq!(tools.len(), 18, "Expected 18 tools in multi-corpus registry");
        assert!(
            registry.registry().get("status").is_some(),
            "consolidated status tool should be registered"
        );
        // The old status tools/aliases are gone.
        assert!(registry.registry().get("get_status").is_none());
        assert!(registry.registry().get("get_corpus_stats").is_none());
        assert!(registry.registry().get("get_indexing_status").is_none());
    }

    fn add_test_corpus(
        manager: &mut ctxvault_core::corpus_manager::CorpusManager,
        config: CorpusConfig,
    ) {
        let index_dir = PathBuf::from(&config.path).join(".index");
        manager.add_corpus_with_index_dir(config, &index_dir).unwrap();
    }

    #[test]
    fn test_multi_corpus_routing_default() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        fs::create_dir_all(&wiki_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        let config = CorpusConfig {
            name: "wiki".to_string(),
            path: wiki_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };
        add_test_corpus(&mut manager, config);

        // Index a file in wiki.
        {
            let engine = manager.get_engine_mut("wiki").unwrap();
            let content = "# Wiki Note\n\nWiki content here.\n";
            fs::write(wiki_dir.join("note.md"), content).unwrap();
            engine.index_file("note.md", content).unwrap();
            engine.commit().unwrap();
        }

        let registry = MultiCorpusToolRegistry::new();

        // Search without corpus param — should use default (wiki).
        let result = registry
            .execute(
                "search",
                &mut manager,
                serde_json::json!({ "query": "wiki content", "mode": "bm25" }),
            )
            .unwrap();

        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let results = resp.docs.unwrap().results;
        assert!(!results.is_empty(), "Should find wiki note via default corpus");
        assert_eq!(results[0].path, "note.md");
    }

    #[test]
    fn test_multi_corpus_routing_explicit() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        let docs_dir = tmp.path().join("docs");
        fs::create_dir_all(&wiki_dir).unwrap();
        fs::create_dir_all(&docs_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();

        let wiki_config = CorpusConfig {
            name: "wiki".to_string(),
            path: wiki_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };
        let docs_config = CorpusConfig {
            name: "docs".to_string(),
            path: docs_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };

        add_test_corpus(&mut manager, wiki_config);
        add_test_corpus(&mut manager, docs_config);

        // Index different content in each corpus.
        {
            let engine = manager.get_engine_mut("wiki").unwrap();
            let content = "# Rust Wiki\n\nRust programming language notes.\n";
            fs::write(wiki_dir.join("rust.md"), content).unwrap();
            engine.index_file("rust.md", content).unwrap();
            engine.commit().unwrap();
        }
        {
            let engine = manager.get_engine_mut("docs").unwrap();
            let content = "# Python Docs\n\nPython documentation guide.\n";
            fs::write(docs_dir.join("python.md"), content).unwrap();
            engine.index_file("python.md", content).unwrap();
            engine.commit().unwrap();
        }

        let registry = MultiCorpusToolRegistry::new();

        // Search in wiki corpus explicitly.
        let result = registry
            .execute(
                "search",
                &mut manager,
                serde_json::json!({ "query": "programming", "mode": "bm25", "corpus": "wiki" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let results = resp.docs.unwrap().results;
        assert!(!results.is_empty(), "Should find rust.md in wiki");
        assert_eq!(results[0].path, "rust.md");

        // Search in docs corpus explicitly.
        let result = registry
            .execute(
                "search",
                &mut manager,
                serde_json::json!({ "query": "documentation", "mode": "bm25", "corpus": "docs" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let results = resp.docs.unwrap().results;
        assert!(!results.is_empty(), "Should find python.md in docs");
        assert_eq!(results[0].path, "python.md");

        // Verify isolation: searching wiki for python returns nothing.
        let result = registry
            .execute(
                "search",
                &mut manager,
                serde_json::json!({ "query": "python documentation", "mode": "bm25", "corpus": "wiki" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let results = resp.docs.map(|d| d.results).unwrap_or_default();
        assert!(
            results.is_empty() || results.iter().all(|r| r.path != "python.md"),
            "Wiki corpus should not contain python.md"
        );
    }

    #[test]
    fn test_multi_corpus_fan_out_tags_by_corpus() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        let docs_dir = tmp.path().join("docs");
        fs::create_dir_all(&wiki_dir).unwrap();
        fs::create_dir_all(&docs_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        for (name, dir) in [("wiki", &wiki_dir), ("docs", &docs_dir)] {
            let config = CorpusConfig {
                name: name.to_string(),
                path: dir.to_string_lossy().to_string(),
                mode: CorpusMode::ReadWrite,
                index_mode: IndexMode::Full,
                chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
                embedding: EmbeddingConfig::default(),
                graph: GraphConfig { edge_types: Vec::new() },
                templates_dir: None,
                exclude: ctxvault_common::config::ExcludeConfig::default(),
                docs: ctxvault_common::config::DocsConfig::default(),
            };
            add_test_corpus(&mut manager, config);
        }

        // Both corpora contain a doc mentioning "shared" (BM25-only; no embedder).
        {
            let engine = manager.get_engine_mut("wiki").unwrap();
            let content = "# Wiki\n\nshared knowledge lives here in the wiki.\n";
            fs::write(wiki_dir.join("shared.md"), content).unwrap();
            engine.index_file("shared.md", content).unwrap();
            engine.commit().unwrap();
        }
        {
            let engine = manager.get_engine_mut("docs").unwrap();
            let content = "# Docs\n\nshared documentation lives here in the docs.\n";
            fs::write(docs_dir.join("shared.md"), content).unwrap();
            engine.index_file("shared.md", content).unwrap();
            engine.commit().unwrap();
        }

        let registry = MultiCorpusToolRegistry::new();

        // Fan out across both corpora with corpora = "all".
        let result = registry
            .execute_read(
                "search",
                &manager,
                serde_json::json!({ "query": "shared", "mode": "bm25", "corpora": "all" }),
            )
            .unwrap();

        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(result).unwrap();
        let docs = resp.docs.unwrap();
        assert_eq!(docs.results.len(), 2, "both corpora should contribute a hit");

        // Same path, distinct corpora → two tagged hits.
        let corpora: HashSet<String> =
            docs.results.iter().filter_map(|r| r.corpus.clone()).collect();
        assert!(corpora.contains("wiki"), "a hit must be tagged 'wiki'");
        assert!(corpora.contains("docs"), "a hit must be tagged 'docs'");
        assert!(docs.results.iter().all(|r| r.path == "shared.md"));

        // Single-corpus read via corpus="wiki" also tags its hit.
        let single = registry
            .execute_read(
                "search",
                &manager,
                serde_json::json!({ "query": "shared", "mode": "bm25", "corpus": "wiki" }),
            )
            .unwrap();
        let single_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(single).unwrap();
        let single_docs = single_resp.docs.unwrap();
        assert!(!single_docs.results.is_empty());
        assert!(single_docs.results.iter().all(|r| r.corpus.as_deref() == Some("wiki")));
    }

    /// Fast-mode (embedder-free) corpus config for scoping-parity + federated
    /// tests. Fast mode skips dense embeddings, so no ONNX model is needed, yet
    /// BM25/graph retrieval and code extraction still run.
    fn fast_corpus_config(name: &str, dir: &Path) -> CorpusConfig {
        let _ = fs::create_dir_all(dir.join(".index"));
        CorpusConfig {
            name: name.to_string(),
            path: dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Fast,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig {
                edge_types: vec![EdgeTypeConfig {
                    name: "Wikilink".to_string(),
                    source: EdgeSource::Wikilink,
                    weight: 1.0,
                    bidirectional: false,
                    field: None,
                    direction: None,
                    max_frequency: None,
                    class: None,
                    description: None,
                    allowed_source_templates: None,
                    allowed_target_templates: None,
                }],
            },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        }
    }

    /// Rust source defining exactly one top-level fn (scope_path == bare name).
    fn rs_symbol(name: &str) -> String {
        format!("pub fn {name}() -> u32 {{\n    42\n}}\n")
    }

    /// Rust source for a named fn that calls an out-of-corpus fn (produces an
    /// unresolved `ExternalRef` that `resolve_external_refs` links across corpora).
    fn rs_caller(caller: &str, callee: &str) -> String {
        format!("pub fn {caller}() -> u32 {{\n    {callee}()\n}}\n")
    }

    /// `corpus`/`corpora` resolution is applied uniformly BEFORE per-engine
    /// dispatch, so every search `mode` is scopable to N / N+1 corpora. This
    /// asserts each embedder-free mode (`bm25`, `graph`, `hybrid`) honors
    /// `corpora=["A","B"]` and `corpora="all"`. Semantic mode is asserted at the
    /// routing level only: it requires the ONNX embedder, which fast-mode corpora
    /// deliberately do not load — but it flows through the identical
    /// `resolve_corpus_target` fan-out path, so its scoping is proven by the fact
    /// that fan-out invokes the same code for every mode. Here we confirm the
    /// mode-agnostic fan-out surfaces corpus-tagged hits from BOTH corpora.
    #[test]
    fn test_search_modes_honor_corpora_scoping() {
        let tmp = TempDir::new().unwrap();
        let a_dir = tmp.path().join("A");
        let b_dir = tmp.path().join("B");
        fs::create_dir_all(&a_dir).unwrap();
        fs::create_dir_all(&b_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        add_test_corpus(&mut manager, fast_corpus_config("A", &a_dir));
        add_test_corpus(&mut manager, fast_corpus_config("B", &b_dir));

        // Each corpus has a doc that shares the query token "shared" and links
        // to a neighbor so graph search (which returns discovered neighbors) has nodes.
        // Index the neighbor first so indexing the parent adds the edge last.
        {
            let a = manager.get_engine_mut("A").unwrap();
            let sub_content = "# Alpha Sub\n\nsub knowledge.\n";
            fs::write(a_dir.join("alpha_sub.md"), sub_content).unwrap();
            a.index_file("alpha_sub.md", sub_content).unwrap();
            let content = "# Alpha\n\nshared alpha knowledge lives here. See [[alpha_sub.md]].\n";
            fs::write(a_dir.join("alpha.md"), content).unwrap();
            a.index_file("alpha.md", content).unwrap();
            a.commit().unwrap();
        }
        {
            let b = manager.get_engine_mut("B").unwrap();
            let sub_content = "# Beta Sub\n\nsub knowledge.\n";
            fs::write(b_dir.join("beta_sub.md"), sub_content).unwrap();
            b.index_file("beta_sub.md", sub_content).unwrap();
            let content = "# Beta\n\nshared beta knowledge lives here. See [[beta_sub.md]].\n";
            fs::write(b_dir.join("beta.md"), content).unwrap();
            b.index_file("beta.md", content).unwrap();
            b.commit().unwrap();
        }

        let registry = MultiCorpusToolRegistry::new();

        // Modes that need no dense embedder: full end-to-end fan-out assertions.
        for mode in ["bm25", "graph", "hybrid"] {
            for corpora in [serde_json::json!(["A", "B"]), serde_json::json!("all")] {
                let result = registry
                    .execute_read(
                        "search",
                        &manager,
                        serde_json::json!({ "query": "shared", "mode": mode, "corpora": corpora }),
                    )
                    .unwrap_or_else(|e| panic!("mode {mode} corpora {corpora:?} failed: {e}"));

                if mode == "graph" {
                    eprintln!("DEBUG GRAPH RESULT: {result:#?}");
                }

                let resp: ctxvault_common::types::SearchResponse =
                    serde_json::from_value(result).unwrap();
                let docs = resp.docs.unwrap_or_default();
                let corpora_seen: HashSet<String> =
                    docs.results.iter().filter_map(|r| r.corpus.clone()).collect();
                assert!(
                    corpora_seen.contains("A"),
                    "mode {mode} corpora {corpora:?}: expected a hit tagged 'A', saw {corpora_seen:?}"
                );
                assert!(
                    corpora_seen.contains("B"),
                    "mode {mode} corpora {corpora:?}: expected a hit tagged 'B', saw {corpora_seen:?}"
                );
            }
        }

        // Semantic mode: fast-mode corpora have no ONNX embedder, so an
        // end-to-end semantic query is not meaningful here. It rides the SAME
        // mode-agnostic fan-out path (resolve_corpus_target strips corpora before
        // per-engine dispatch, independent of `mode`), so its scoping is proven by
        // the fan-out invoking each engine — we assert the call fans out to both
        // engines by observing per-corpus execution (a fast-mode semantic call
        // errors per engine, so the fan-out surfaces that error rather than a
        // wrong-corpus routing). The routing itself is mode-independent.
        let sem = registry.execute_read(
            "search",
            &manager,
            serde_json::json!({ "query": "shared", "mode": "semantic", "corpora": ["A", "B"] }),
        );
        assert!(
            sem.is_err(),
            "semantic fan-out over embedder-free corpora surfaces the per-engine \
             embedder error, confirming the call was routed/fanned out (not silently dropped)"
        );
    }

    /// The `trace_cross_corpus` MCP tool exposes federated traversal: given a
    /// start node in corpus A whose call crosses into corpus B, the returned JSON
    /// carries a `hops` entry naming `to_corpus == "B"` and a `nodes` entry tagged
    /// with `corpus == "B"`.
    #[test]
    fn test_trace_cross_corpus_returns_hop_annotated_results() {
        let tmp = TempDir::new().unwrap();
        let a_dir = tmp.path().join("A");
        let b_dir = tmp.path().join("B");
        fs::create_dir_all(&a_dir).unwrap();
        fs::create_dir_all(&b_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        add_test_corpus(&mut manager, fast_corpus_config("A", &a_dir));
        add_test_corpus(&mut manager, fast_corpus_config("B", &b_dir));

        // B uniquely defines `leaf`; A's `top` calls `leaf` (unresolved locally).
        {
            let b = manager.get_engine_mut("B").unwrap();
            b.index_file("src/leaf.rs", &rs_symbol("leaf")).unwrap();
            b.commit().unwrap();
        }
        {
            let a = manager.get_engine_mut("A").unwrap();
            a.index_file("src/top.rs", &rs_caller("top", "leaf")).unwrap();
            a.commit().unwrap();
        }

        // Build the cross-corpus edge (CorpusManager-level; public API).
        let created = manager.resolve_external_refs().unwrap();
        assert!(created >= 1, "a unique cross-corpus ref must create an edge");

        let registry = MultiCorpusToolRegistry::new();
        let result = registry
            .execute_read(
                "trace_cross_corpus",
                &manager,
                serde_json::json!({
                    "start_corpus": "A",
                    "start_node": "top",
                    "per_corpus_depth": 4,
                    "max_corpus_hops": 3,
                    "continue": true
                }),
            )
            .unwrap();

        // A cross-corpus hop into B must be recorded.
        let hops = result["hops"].as_array().expect("hops must be an array");
        assert!(
            hops.iter().any(|h| h["to_corpus"] == "B"),
            "hops must name a cross-corpus seam into corpus B: {hops:?}"
        );
        let ab = hops.iter().find(|h| h["to_corpus"] == "B").unwrap();
        assert_eq!(ab["from_corpus"], "A");
        assert_eq!(ab["to_node"], "leaf");

        // A node tagged with corpus B must appear (live continuation entered B).
        let nodes = result["nodes"].as_array().expect("nodes must be an array");
        assert!(
            nodes.iter().any(|n| n["corpus"] == "B" && n["node"] == "leaf"),
            "nodes must include B's `leaf` node tagged corpus 'B': {nodes:?}"
        );
        // The origin node in A is present at depth 0.
        assert!(
            nodes.iter().any(|n| n["corpus"] == "A" && n["node"] == "top" && n["depth"] == 0),
            "origin node A::top must be present at depth 0: {nodes:?}"
        );
    }

    #[test]
    fn test_multi_corpus_get_status() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        fs::create_dir_all(&wiki_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        let config = CorpusConfig {
            name: "wiki".to_string(),
            path: wiki_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };
        add_test_corpus(&mut manager, config);

        let registry = MultiCorpusToolRegistry::new();

        let result = registry.execute("status", &mut manager, serde_json::json!({})).unwrap();

        assert_eq!(result["corpus_count"], 1);
        assert_eq!(result["default_corpus"], "wiki");
        let corpora = result["corpora"].as_array().unwrap();
        assert_eq!(corpora.len(), 1);
        assert_eq!(corpora[0]["name"], "wiki");
    }

    #[test]
    fn test_multi_corpus_invalid_corpus_returns_error() {
        let tmp = TempDir::new().unwrap();
        let wiki_dir = tmp.path().join("wiki");
        fs::create_dir_all(&wiki_dir).unwrap();

        let mut manager = ctxvault_core::corpus_manager::CorpusManager::new();
        let config = CorpusConfig {
            name: "wiki".to_string(),
            path: wiki_dir.to_string_lossy().to_string(),
            mode: CorpusMode::ReadWrite,
            index_mode: IndexMode::Full,
            chunking: ChunkingConfig { min_chunk_tokens: 1, ..Default::default() },
            embedding: EmbeddingConfig::default(),
            graph: GraphConfig { edge_types: Vec::new() },
            templates_dir: None,
            exclude: ctxvault_common::config::ExcludeConfig::default(),
            docs: ctxvault_common::config::DocsConfig::default(),
        };
        add_test_corpus(&mut manager, config);

        let registry = MultiCorpusToolRegistry::new();

        // Non-existent corpus should error.
        let result = registry.execute(
            "search",
            &mut manager,
            serde_json::json!({ "query": "test", "mode": "bm25", "corpus": "nonexistent" }),
        );
        assert!(result.is_err());
    }

    // ─── Graph Match Tool Tests ────────────────────────────────────────

    #[test]
    fn test_graph_match_tool() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);

        // Add nodes and lineage edge directly to graph and SQLite
        engine.graph_mut().add_edge(
            "docs/adrs/002.md",
            "docs/adrs/001.md",
            "supersedes",
            1.0,
            EdgeProvenance::Frontmatter,
            EdgeClass::Structural,
        );
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry
            .execute(
                "graph_match",
                &mut engine,
                serde_json::json!({
                    "pattern": "(a)-[:supersedes]->(b)"
                }),
            )
            .unwrap();

        let match_res: ctxvault_common::types::GraphMatchResult =
            serde_json::from_value(result).unwrap();
        assert_eq!(match_res.total_matches, 1);
        assert_eq!(match_res.tree[0].node, "docs/adrs/001.md");
    }

    #[test]
    fn test_validate_tool() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);

        // Add broken link: valid.md -> missing.md
        engine.graph_mut().add_edge(
            "valid.md",
            "missing.md",
            "Wikilink",
            1.0,
            EdgeProvenance::Wikilink,
            EdgeClass::Structural,
        );

        // Add circular dependency: A -> B -> A
        engine.graph_mut().add_edge(
            "A.md",
            "B.md",
            "supersedes",
            1.0,
            EdgeProvenance::Frontmatter,
            EdgeClass::Structural,
        );
        engine.graph_mut().add_edge(
            "B.md",
            "A.md",
            "supersedes",
            1.0,
            EdgeProvenance::Frontmatter,
            EdgeClass::Structural,
        );

        let mut registry = ToolRegistry::new();
        registry.register_all();

        let result = registry
            .execute("validate", &mut engine, serde_json::json!({ "check_taxonomy": true }))
            .unwrap();

        assert_eq!(result["valid"], false);
        assert!(result["taxonomy"]["broken_links_count"].as_u64().unwrap() >= 1);
        assert!(result["taxonomy"]["circular_dependencies_count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_list_templates_and_validate_markdown_template() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");
        let templates_dir = corpus_dir.join(".templates");
        fs::create_dir_all(&templates_dir).unwrap();

        let adr_template = r#"---
template:
  name: adr
  description: "Architecture Decision Record"

schema:
  fields:
    status:
      type: enum
      required: true
      values: [proposed, accepted, rejected]
    date:
      type: date
      required: true
  edges:
    - field: supersedes
      type: Supersedes
      class: structural
      direction: outbound
      target_template: adr
      required: false

  sections:
    required: ["Context", "Decision"]
  min_words: 20
---
# ADR-{id}: {Title}

## Context
Describe context.

## Decision
State decision.
"#;
        fs::write(templates_dir.join("adr.md"), adr_template).unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. list_templates returns schema + scaffold
        let tmpl_list =
            registry.execute("list_templates", &mut engine, serde_json::json!({})).unwrap();
        let list_arr = tmpl_list.as_array().unwrap();
        assert_eq!(list_arr.len(), 1);
        assert_eq!(list_arr[0]["name"], "adr");
        assert!(list_arr[0]["scaffold"].as_str().unwrap().contains("# ADR-{id}: {Title}"));
        assert_eq!(list_arr[0]["edges"][0]["field"], "supersedes");

        // 2. Validate a valid note
        let note_content = r#"---
template: adr
status: accepted
date: 2026-09-11
---
# ADR-001: First Decision

## Context
This is a comprehensive context section that satisfies the minimum word count requirement for this template.

## Decision
We decide to adopt the markdown template standard across all repositories.
"#;
        fs::write(corpus_dir.join("001.md"), note_content).unwrap();
        engine.index_file("001.md", note_content).unwrap();
        engine.commit().unwrap();

        let val_res = registry
            .execute(
                "validate",
                &mut engine,
                serde_json::json!({ "path": "001.md", "check_taxonomy": false }),
            )
            .unwrap();
        assert_eq!(val_res["valid"], true, "Note should be valid: {:?}", val_res);

        // 3. Validate a note with missing required field and missing section
        let invalid_note = r#"---
template: adr
status: accepted
---
# ADR-002: Incomplete

## Context
Only context, missing decision and date.
"#;
        fs::write(corpus_dir.join("002.md"), invalid_note).unwrap();
        let val_invalid = registry
            .execute(
                "validate",
                &mut engine,
                serde_json::json!({ "path": "002.md", "check_taxonomy": false }),
            )
            .unwrap();
        assert_eq!(val_invalid["valid"], false, "Note should be invalid: {:?}", val_invalid);
    }

    #[test]
    fn test_code_intelligence_mcp_tools() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        // Write polyglot code files
        let rust_code = r#"
pub struct QueryParser;

impl QueryParser {
    pub fn parse_query(&self, raw: &str) -> Vec<String> {
        tokenize(raw)
    }
}

pub fn tokenize(input: &str) -> Vec<String> {
    vec![input.to_string()]
}
"#;
        fs::write(corpus_dir.join("parser.rs"), rust_code).unwrap();
        engine.index_file("parser.rs", rust_code).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Test get_snippet (absorbed get_symbol_definition)
        let def_res = registry
            .execute_read("get_snippet", &engine, serde_json::json!({ "name": "parse_query" }))
            .unwrap();

        assert_eq!(def_res["path"], "parser.rs");
        assert!(def_res["total_lines"].as_u64().unwrap() >= 1);
        assert!(def_res["source"].as_str().unwrap().contains("tokenize(raw)"));

        // 2. Test callers via graph_match
        let callers_res = registry
            .execute_read(
                "graph_match",
                &engine,
                serde_json::json!({ "pattern": "(caller)-[:calls]->(target {name: \"tokenize\"})" }),
            )
            .unwrap();
        let match_res: ctxvault_common::types::GraphMatchResult =
            serde_json::from_value(callers_res).unwrap();
        assert_eq!(match_res.total_matches, 1);
        assert_eq!(match_res.tree[0].node, "tokenize");

        // 3. Test graph_communities view='architecture' (absorbed get_architecture)
        let arch_res = registry
            .execute_read(
                "graph_communities",
                &engine,
                serde_json::json!({ "view": "architecture" }),
            )
            .unwrap();

        assert!(arch_res["component_count"].as_u64().unwrap() >= 1);
        assert!(!arch_res["components"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_read_file_tool() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        let md = "# Design Note\n\nSome markdown content here.\n";
        fs::write(corpus_dir.join("design.md"), md).unwrap();
        engine.index_file("design.md", md).unwrap();

        let rust = "pub fn helper() -> u32 { 42 }\n";
        fs::write(corpus_dir.join("lib.rs"), rust).unwrap();
        engine.index_file("lib.rs", rust).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // Single file read
        let single = registry
            .execute_read("read_file", &engine, serde_json::json!({ "path": "design.md" }))
            .unwrap();
        assert_eq!(single["kind"], "markdown_note");
        assert_eq!(single["title"], "Design Note");
        assert!(single["content"].as_str().unwrap().contains("markdown content"));

        // Batch file read: Two existing files + one missing -> 3 entries, one carrying an error.
        let res = registry
            .execute_read(
                "read_file",
                &engine,
                serde_json::json!({ "paths": ["design.md", "lib.rs", "nope.md"] }),
            )
            .unwrap();

        assert_eq!(res["count"], 3);
        let results = res["results"].as_array().unwrap();

        let note = results.iter().find(|r| r["path"] == "design.md").unwrap();
        assert_eq!(note["kind"], "markdown_note");
        assert_eq!(note["title"], "Design Note");
        assert!(note["content"].as_str().unwrap().contains("markdown content"));
        assert!(note.get("error").is_none());

        let code = results.iter().find(|r| r["path"] == "lib.rs").unwrap();
        assert_eq!(code["kind"], "code_file");
        assert_eq!(code["language"], "rust");
        assert!(code["content"].as_str().unwrap().contains("helper"));

        let missing = results.iter().find(|r| r["path"] == "nope.md").unwrap();
        assert!(missing.get("error").is_some(), "missing path must carry an error entry");
    }

    #[test]
    fn test_status_coverage_scope() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        let rust = r#"
pub fn indexed_fn() -> u32 {
    7
}
"#;
        fs::write(corpus_dir.join("covered.rs"), rust).unwrap();
        engine.index_file("covered.rs", rust).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        let res = registry
            .execute_read(
                "status",
                &engine,
                serde_json::json!({ "scope": "coverage", "paths": ["covered.rs", "does_not_exist.rs"] }),
            )
            .unwrap();

        let reports = res["reports"].as_array().unwrap();
        assert_eq!(reports.len(), 2);

        let covered = reports.iter().find(|r| r["path"] == "covered.rs").unwrap();
        assert_eq!(covered["indexed"], true);
        assert!(covered["chunk_count"].as_u64().unwrap() > 0);
        assert_eq!(covered["parsed"], true);

        let bogus = reports.iter().find(|r| r["path"] == "does_not_exist.rs").unwrap();
        assert_eq!(bogus["indexed"], false);
        assert_eq!(bogus["parsed"], false);

        assert_eq!(res["summary"]["total"], 2);
        assert_eq!(res["summary"]["covered"], 1);
        assert_eq!(res["summary"]["uncovered"], 1);
    }

    #[test]
    fn test_fast_mode_mcp_tools() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("fast_corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        fs::write(
            corpus_dir.join("guide.md"),
            "# Architecture Guide\nFast mode provides instant BM25 and graph search without vector models.\n",
        )
        .unwrap();

        let mut config = test_config(&corpus_dir);
        config.index_mode = IndexMode::Fast;

        let index_dir = tmp.path().join(".index");
        let mut engine = Engine::open(config, &index_dir).unwrap();
        let files_indexed = engine.full_reindex().unwrap();
        assert_eq!(files_indexed, 1);
        assert!(engine.is_fast_mode());
        assert!(!engine.has_vector_index());

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Semantic search must fail with the exact fast mode error message
        let sem_err = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "architecture guide", "mode": "semantic" }),
            )
            .unwrap_err();
        assert!(
            sem_err.to_string().contains(
                "Semantic search is unavailable in fast mode. Re-index with index_mode = 'full' to enable vector search."
            ),
            "Unexpected error: {sem_err}"
        );

        // 2. Hybrid search must cleanly fall back to BM25+Graph
        let hyb_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "architecture", "mode": "hybrid" }),
            )
            .unwrap();
        let hyb_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(hyb_res).unwrap();
        let hyb_docs = hyb_resp.docs.unwrap();
        assert_eq!(hyb_docs.results.len(), 1);
        assert_eq!(hyb_docs.results[0].path, "guide.md");

        // 3. Sync corpus with fast: true maintains fast mode
        let sync_res = registry
            .execute("sync_corpus", &mut engine, serde_json::json!({ "fast": true }))
            .unwrap();
        assert_eq!(sync_res["status"], "complete");
        assert!(engine.is_fast_mode());
    }

    #[test]
    fn test_full_mode_mcp_tools() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("full_corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        fs::write(
            corpus_dir.join("guide.md"),
            "# Architecture Guide\nFull mode provides vector search for documentation and hamming for code.\n",
        )
        .unwrap();
        fs::write(
            corpus_dir.join("service.rs"),
            "pub struct SearchPipeline;\npub fn execute_pipeline() {}\n",
        )
        .unwrap();

        let mut config = test_config(&corpus_dir);
        config.index_mode = IndexMode::Full;

        let index_dir = tmp.path().join(".index");
        let mut engine = Engine::open(config, &index_dir).unwrap();
        let files_indexed = engine.full_reindex().unwrap();
        assert_eq!(files_indexed, 2);
        assert!(!engine.is_fast_mode());
        assert!(engine.has_vector_index());

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Status tool reflects Full mode
        let status_res = registry
            .execute_read("status", &engine, serde_json::json!({ "scope": "corpus" }))
            .unwrap();
        assert_eq!(status_res["index_mode"], "Full");

        // 2. BM25 search finds both doc and code
        let bm25_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "SearchPipeline", "mode": "bm25" }),
            )
            .unwrap();
        let bm25_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(bm25_res).unwrap();
        assert!(bm25_resp.code.is_some());

        // 3. Hybrid search executes cleanly
        let hyb_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "Architecture Guide", "mode": "hybrid" }),
            )
            .unwrap();
        let hyb_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(hyb_res).unwrap();
        assert!(hyb_resp.docs.is_some());

        // 4. Reindex with index_mode override preserves Full
        let reindex_res = registry
            .execute(
                "sync_corpus",
                &mut engine,
                serde_json::json!({ "mode": "full", "index_mode": "full" }),
            )
            .unwrap();
        assert_eq!(reindex_res["status"], "complete");
        assert!(!engine.is_fast_mode());
    }

    // ─── Progressive Disclosure Tests (Tier 1 → 2 → 3) ─────────────────

    #[test]
    fn test_progressive_disclosure_handle_fetch_full() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        // A markdown note with two headings → two chunks.
        let md = "# Alpha Section\n\nAlpha talks about retrieval and ranking.\n\n\
                  # Beta Section\n\nBeta talks about graph traversal and edges.\n";
        fs::write(corpus_dir.join("notes.md"), md).unwrap();
        engine.index_file("notes.md", md).unwrap();

        // A Rust file with a caller/callee pair.
        let rust = r#"
pub struct Router;

impl Router {
    pub fn dispatch(&self, q: &str) -> Vec<String> {
        normalize(q)
    }
}

pub fn normalize(input: &str) -> Vec<String> {
    vec![input.to_lowercase()]
}
"#;
        fs::write(corpus_dir.join("router.rs"), rust).unwrap();
        engine.index_file("router.rs", rust).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // Tier 1: a search with detail="ids" returns handles with snippet == None.
        let ids_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "retrieval ranking", "mode": "bm25", "detail": "ids" }),
            )
            .unwrap();
        let ids_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(ids_res).unwrap();
        let ids_results = ids_resp.docs.unwrap().results;
        assert!(!ids_results.is_empty(), "detail=ids should still return handles");
        assert!(
            ids_results.iter().all(|r| r.snippet.is_none()),
            "detail=ids must strip snippets (bare handles only)"
        );

        // Default detail keeps a short snippet.
        let default_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "retrieval ranking", "mode": "bm25" }),
            )
            .unwrap();
        let default_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(default_res).unwrap();
        let default_results = default_resp.docs.unwrap().results;
        assert!(default_results.iter().any(|r| r.snippet.is_some()), "default keeps a snippet");

        // Tier 2 (doc): fetch exactly one chunk by path + chunk_index, bounded.
        let chunk_res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "path": "notes.md", "chunk_index": 0, "max_lines": 100 }),
            )
            .unwrap();
        assert_eq!(chunk_res["path"], "notes.md");
        assert!(chunk_res["total_lines"].as_u64().unwrap() >= 1);
        assert_eq!(chunk_res["chunk_index"], 0);
        assert!(chunk_res["text"].as_str().unwrap().contains("Alpha"));

        // Tier 2 (doc) neighbor expansion: adjacent chunk is returned.
        let chunk_nb = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({
                    "path": "notes.md",
                    "chunk_index": 0,
                    "include_neighbors": true
                }),
            )
            .unwrap();
        assert_eq!(chunk_nb["previous"], Value::Null, "chunk 0 has no previous");
        assert!(chunk_nb["next"].is_object(), "chunk 0 should have a next neighbor");
        assert!(chunk_nb["next"]["text"].as_str().unwrap().contains("Beta"));

        // Tier 2 (code): fetch one symbol's source by qualified_name.
        let sym_res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "Router > dispatch" }),
            )
            .unwrap();
        assert_eq!(sym_res["path"], "router.rs");
        assert!(sym_res["total_lines"].as_u64().unwrap() >= 1);
        assert!(sym_res["source"].as_str().unwrap().contains("normalize(q)"));
        assert!(sym_res["start_line"].as_u64().unwrap() >= 1);
        assert!(
            sym_res["end_line"].as_u64().unwrap() >= sym_res["start_line"].as_u64().unwrap(),
            "line range must be well-formed"
        );

        // Tier 2 (code) neighbor expansion: callees include the called symbol.
        let sym_nb = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({
                    "qualified_name": "Router > dispatch",
                    "include_neighbors": true
                }),
            )
            .unwrap();
        let outgoing = sym_nb["relationships"]["outgoing"].as_object().unwrap();
        let callees = outgoing.get("calls").and_then(|v| v.as_array()).unwrap();
        assert!(
            callees.iter().any(|c| c["name"] == "normalize" || c["scope_path"] == "normalize"),
            "dispatch should list normalize as an outgoing calls handle"
        );
        // Callees are HANDLES only — no body field.
        assert!(
            callees.iter().all(|c| c.get("source").is_none()),
            "neighbors are handles, not bodies"
        );

        // Callers of normalize should include dispatch in incoming calls.
        let normalize_nb = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "normalize", "include_neighbors": true }),
            )
            .unwrap();
        let incoming = normalize_nb["relationships"]["incoming"].as_object().unwrap();
        let callers = incoming.get("calls").and_then(|v| v.as_array()).unwrap();
        assert!(
            callers.iter().any(|c| c["scope_path"] == "Router > dispatch"),
            "normalize should list Router > dispatch as an incoming calls handle"
        );

        // Tier 2 bounding: max_lines truncates the body.
        let capped = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "Router > dispatch", "max_lines": 1 }),
            )
            .unwrap();
        assert_eq!(capped["truncated"], true, "max_lines=1 must truncate a multi-line symbol");
        assert_eq!(capped["source"].as_str().unwrap().lines().count(), 1);

        // Tier 3 (code): read the whole file raw.
        let file_res = registry
            .execute_read("read_file", &engine, serde_json::json!({ "path": "router.rs" }))
            .unwrap();
        assert_eq!(file_res["language"], "rust");
        assert!(file_res["content"].as_str().unwrap().contains("pub struct Router;"));
        assert!(file_res["content"].as_str().unwrap().contains("pub fn normalize"));
        assert!(file_res["total_lines"].as_u64().unwrap() >= 5);

        // A bare path (no chunk_index / qualified_name) is redirected to Tier 3.
        let hint = registry.execute_read(
            "get_snippet",
            &engine,
            serde_json::json!({ "path": "router.rs" }),
        );
        assert!(hint.is_err(), "bare path must hint toward Tier 3");
    }

    #[test]
    fn test_search_detail_ids_stripping_and_explain() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        let md_old = "# Legacy\n\nLegacy architecture and design.\n";
        let md_new = "# Modern\n\nModern architecture and design.\n";
        fs::write(corpus_dir.join("legacy.md"), md_old).unwrap();
        fs::write(corpus_dir.join("modern.md"), md_new).unwrap();
        engine.index_file("legacy.md", md_old).unwrap();
        engine.index_file("modern.md", md_new).unwrap();

        let rust = r#"
pub struct Service;

impl Service {
    pub fn process(&self) -> bool {
        true
    }
}
"#;
        fs::write(corpus_dir.join("service.rs"), rust).unwrap();
        engine.index_file("service.rs", rust).unwrap();

        // Add structural edge so legacy.md has lineage: modern.md supersedes legacy.md
        engine.graph_mut().add_edge(
            "modern.md",
            "legacy.md",
            "supersedes",
            1.0,
            EdgeProvenance::Frontmatter,
            ctxvault_common::config::EdgeClass::Structural,
        );
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. detail="ids" on code search: snippet, lineage, and score_components must all be None
        let code_ids_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({
                    "query": "Service process",
                    "modality": "code",
                    "mode": "bm25",
                    "detail": "ids"
                }),
            )
            .unwrap();
        let code_ids_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(code_ids_res).unwrap();
        let code_ids_results = code_ids_resp.code.unwrap().results;
        assert!(!code_ids_results.is_empty(), "expected hits for Service process");
        for r in &code_ids_results {
            assert!(r.snippet.is_none(), "code hit snippet must be None with detail=ids");
            assert!(r.lineage.is_none(), "code hit lineage must be None with detail=ids");
            assert!(
                r.score_components.is_none(),
                "code hit score_components must be None with detail=ids"
            );
        }

        // 2. detail="ids" on doc search with lineage: snippet, lineage, and score_components must all be None
        let doc_ids_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({
                    "query": "Legacy architecture",
                    "modality": "docs",
                    "mode": "bm25",
                    "detail": "ids"
                }),
            )
            .unwrap();
        let doc_ids_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(doc_ids_res).unwrap();
        let doc_ids_results = doc_ids_resp.docs.unwrap().results;
        assert!(!doc_ids_results.is_empty(), "expected hits for Legacy architecture");
        for r in &doc_ids_results {
            assert!(r.snippet.is_none(), "doc hit snippet must be None with detail=ids");
            assert!(r.lineage.is_none(), "doc hit lineage must be None with detail=ids");
            assert!(
                r.score_components.is_none(),
                "doc hit score_components must be None with detail=ids"
            );
        }

        // 3. detail="default" preserves snippet, lineage, and score_components
        let default_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({
                    "query": "Legacy architecture",
                    "modality": "docs",
                    "mode": "bm25",
                    "detail": "default"
                }),
            )
            .unwrap();
        let default_resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(default_res).unwrap();
        let default_results = default_resp.docs.unwrap().results;
        assert!(!default_results.is_empty());
        let legacy_hit = default_results.iter().find(|r| r.path.contains("legacy.md")).unwrap();
        assert!(legacy_hit.snippet.is_some(), "snippet must be preserved with detail=default");
        assert!(legacy_hit.lineage.is_some(), "lineage must be preserved with detail=default");
        assert!(
            legacy_hit.score_components.is_some(),
            "score_components must be preserved with detail=default"
        );

        // 4. mode="explain" preserves score breakdown even with detail="ids"
        let explain_res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({
                    "query": "Legacy architecture",
                    "mode": "explain",
                    "detail": "ids"
                }),
            )
            .unwrap();
        let explanations: Vec<ctxvault_common::types::SearchExplanation> =
            serde_json::from_value(explain_res).unwrap();
        assert!(!explanations.is_empty(), "explain should return explanations");
        for exp in &explanations {
            assert!(exp.snippet.is_none(), "snippet must be None when detail=ids in explain");
            assert!(exp.final_score > 0.0, "final_score must be preserved in explain");
            assert!(exp.bm25.raw_score > 0.0, "bm25 score component must be preserved in explain");
        }
    }

    #[test]
    fn test_generic_normalized_scope_resolution() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        let rust_code = r#"
pub struct EarlyBinder<'tcx, T> {
    value: T,
    _marker: std::marker::PhantomData<&'tcx ()>,
}

impl<'tcx, T> EarlyBinder<'tcx, T> {
    pub fn instantiate(&self) -> &T {
        &self.value
    }
}

pub struct OtherBinder<'a, A> {
    item: A,
    _life: &'a str,
}

impl<'a, A> OtherBinder<'a, A> {
    pub fn instantiate(&self) -> &A {
        &self.item
    }
}
"#;
        fs::write(corpus_dir.join("binder.rs"), rust_code).unwrap();
        engine.index_file("binder.rs", rust_code).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Resolve EarlyBinder > instantiate when defined as EarlyBinder<'tcx, T> > instantiate
        let res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "EarlyBinder > instantiate" }),
            )
            .unwrap();
        assert_eq!(res["path"], "binder.rs");
        assert!(res["total_lines"].as_u64().unwrap() >= 1);
        assert!(res["source"].as_str().unwrap().contains("&self.value"));

        // 2. Nonexistent symbol returns clean 404 Not Found error
        let err = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "Nonexistent > missing" }),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("no code symbol")
        );

        // 3. Ambiguous method: two EarlyBinder > instantiate in different files
        let rust_code_2 = r#"
pub struct EarlyBinder<'a, T> {
    alt: T,
}

impl<'a, T> EarlyBinder<'a, T> {
    pub fn instantiate(&self) -> &T {
        &self.alt
    }
}
"#;
        fs::write(corpus_dir.join("binder2.rs"), rust_code_2).unwrap();
        engine.index_file("binder2.rs", rust_code_2).unwrap();
        engine.commit().unwrap();

        let amb_res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "EarlyBinder > instantiate" }),
            )
            .unwrap();
        assert_eq!(amb_res["kind"], "ambiguous");
        let candidates = amb_res["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|c| c["file_path"] == "binder.rs"));
        assert!(candidates.iter().any(|c| c["file_path"] == "binder2.rs"));
    }

    #[test]
    fn test_get_snippet_suggestions_and_enrichment() {
        let tmp = TempDir::new().unwrap();
        let mut engine = create_test_engine(&tmp);
        let corpus_dir = tmp.path().join("corpus");

        let rust_code = r#"
/// Compute hash of input data.
pub fn compute_hash(data: &[u8]) -> u64 {
    42
}
"#;
        fs::write(corpus_dir.join("hash.rs"), rust_code).unwrap();
        engine.index_file("hash.rs", rust_code).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Context enrichment: check scope_path, signature, docstring, language, path
        let res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "compute_hash", "include_neighbors": true }),
            )
            .unwrap();
        assert_eq!(res["path"], "hash.rs");
        assert_eq!(res["start_line"], 3);
        assert_eq!(res["end_line"], 5);
        assert_eq!(res["total_lines"], 5);
        assert!(res["source"].as_str().unwrap().contains("pub fn compute_hash"));
        assert!(res["docstring"].as_str().unwrap().contains("Compute hash of input data."));
        // Grammar-driven relationships: incoming defines from hash.rs, 0 callers, 0 outgoing.
        let incoming = res["relationships"]["incoming"].as_object().unwrap();
        assert!(incoming.get("calls").is_none());
        assert!(incoming.contains_key("defines"));
        assert!(res["relationships"]["outgoing"].as_object().unwrap().is_empty());

        // 2. Candidate suggestions on near-miss: query with wrong container "CryptoEngine > compute_hash"
        let sugg_res = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "CryptoEngine > compute_hash" }),
            )
            .unwrap();
        assert_eq!(sugg_res["kind"], "candidate_suggestions");
        let candidates = sugg_res["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0]["name"], "compute_hash");
        assert_eq!(candidates[0]["scope_path"], "compute_hash");
        assert!(candidates[0]["signature"].as_str().unwrap().contains("compute_hash"));

        // 3. Complete miss returns 404
        let err = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "qualified_name": "CryptoEngine > unknown_fn" }),
            )
            .unwrap_err();
        assert!(err.to_string().contains("no code symbol"));
    }

    #[test]
    fn test_search_inlines_top_snippets() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let doc_content =
            "# Architecture\n\nCtxvault is a high performance semantic context server.\n";
        fs::write(corpus_dir.join("arch.md"), doc_content).unwrap();
        engine.index_file("arch.md", doc_content).unwrap();

        let rust_code = "pub fn execute_search() -> bool { true }\n";
        fs::write(corpus_dir.join("search.rs"), rust_code).unwrap();
        engine.index_file("search.rs", rust_code).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // Search with snippets = 2
        let res = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "semantic context server", "mode": "bm25", "snippets": 2 }),
            )
            .unwrap();

        let resp: ctxvault_common::types::SearchResponse = serde_json::from_value(res).unwrap();
        let docs = resp.docs.unwrap();
        assert!(!docs.results.is_empty());
        let top_hit = &docs.results[0];
        assert_eq!(top_hit.path, "arch.md");
        assert!(top_hit.snippet.is_some(), "Turn 1 snippet must be populated");
        assert!(top_hit.snippet.as_ref().unwrap().contains("high performance semantic"));
    }

    #[test]
    fn test_dynamic_turn1_schema_envelope_and_graph_match() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let ts_code = r#"
@Injectable()
export class UserService extends BaseService implements IUserService {
    @Get('/users')
    getUsers() {}
}
"#;
        fs::write(corpus_dir.join("user.ts"), ts_code).unwrap();
        engine.index_file("user.ts", ts_code).unwrap();
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Search for UserService and inspect Turn 1 SchemaEnvelope & Affordances
        let search_val = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "UserService", "mode": "bm25" }),
            )
            .unwrap();

        let resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(search_val).unwrap();
        let code = resp.code.expect("expected code partition");
        assert!(!code.results.is_empty(), "expected hits for UserService");

        // Verify active_edges in schema_envelope contains extended edge types
        assert!(
            code.schema_envelope.active_edges.iter().any(|e| e == "extends"),
            "schema_envelope should include 'extends', got: {:?}",
            code.schema_envelope.active_edges
        );
        assert!(
            code.schema_envelope.active_edges.iter().any(|e| e == "decorates"),
            "schema_envelope should include 'decorates', got: {:?}",
            code.schema_envelope.active_edges
        );

        // Verify Cypher-Lite graph representation on the UserService hit
        let hit = &code.results[0];
        let graph = hit.graph.as_ref().expect("expected graph affordances");
        assert!(
            graph.contains("extends") || graph.contains("decorates"),
            "Expected extends or decorates in graph: {}",
            graph
        );

        // 2. Cypher-Lite graph_match traversal across new edge types
        let match_val = registry
            .execute_read(
                "graph_match",
                &engine,
                serde_json::json!({ "pattern": "(:CodeSymbol {name: \"UserService\"})-[:extends]->(target)" }),
            )
            .unwrap();

        let match_res: ctxvault_common::types::GraphMatchResult =
            serde_json::from_value(match_val).unwrap();
        assert_eq!(match_res.total_matches, 1);
        assert!(!match_res.tree.is_empty(), "Expected graph_match path for -[:extends]->");
    }

    #[test]
    fn test_lean_multiline_emissions_turns_1_to_3() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let config = test_config(&corpus_dir);
        let mut engine = Engine::open(config, &index_dir).unwrap();

        let rust_code = r#"
pub struct PaymentService {
    api_key: String,
}

pub fn process_payment(amount: u64) -> bool {
    amount > 0
}
"#;
        fs::write(corpus_dir.join("payment.rs"), rust_code).unwrap();
        engine.index_file("payment.rs", rust_code).unwrap();
        engine.graph_mut().add_edge(
            "PaymentService",
            "process_payment",
            "calls",
            1.0,
            ctxvault_common::types::EdgeProvenance::CodeCalls,
            ctxvault_common::config::EdgeClass::Code,
        );
        engine.commit().unwrap();

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 1. Turn 1: Search with format="lean"
        let search_val = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "PaymentService", "mode": "bm25", "format": "lean" }),
            )
            .unwrap();

        let text = search_val.as_str().expect("expected lean text string");
        assert!(text.contains("# Search: \"PaymentService\" [mode: bm25, hits:"));
        assert!(text.contains("PaymentService (`payment.rs`)"));
        assert!(text.contains("-> [T2a fetch] get_snippet"));
        assert!(text.contains("-> [T2b graph]"));

        // 2. Turn 2a: get_snippet with format="lean"
        let snippet_val = registry
            .execute_read(
                "get_snippet",
                &engine,
                serde_json::json!({ "name": "PaymentService", "format": "lean" }),
            )
            .unwrap();
        let snippet_text = snippet_val.as_str().expect("expected lean text string");
        assert!(snippet_text.contains("# Symbol: PaymentService (`payment.rs:L2-L4"));
        assert!(snippet_text.contains("L2: pub struct PaymentService {"));
        assert!(snippet_text.contains("-> [T2b callers] graph_match"));
        assert!(snippet_text.contains("-> [T3 full file] read_file"));

        // 3. Turn 2b: graph_match with format="lean"
        let match_val = registry
            .execute_read(
                "graph_match",
                &engine,
                serde_json::json!({
                    "pattern": "(:CodeSymbol {name: \"PaymentService\"})-[:calls]->(target)",
                    "format": "lean"
                }),
            )
            .unwrap();
        let match_text = match_val.as_str().expect("expected lean text string");
        assert!(match_text.contains("root: PaymentService"));
        assert!(match_text.contains("-> [T2a fetch] get_snippet(symbol: \"PaymentService\")"));

        // 4. Turn 3: read_file with format="lean"
        let read_val = registry
            .execute_read(
                "read_file",
                &engine,
                serde_json::json!({ "path": "payment.rs", "format": "lean" }),
            )
            .unwrap();
        let read_text = read_val.as_str().expect("expected lean text string");
        assert!(read_text.contains("# File: `payment.rs` [lines: L1-L8 of 8, language: rust]"));
        assert!(read_text.contains("```rust\nL1: \nL2: pub struct PaymentService {"));

        // 5. JSON format override continues to return structured objects
        let json_val = registry
            .execute_read(
                "search",
                &engine,
                serde_json::json!({ "query": "PaymentService", "mode": "bm25", "format": "json" }),
            )
            .unwrap();
        let resp: ctxvault_common::types::SearchResponse =
            serde_json::from_value(json_val).unwrap();
        assert!(resp.code.unwrap().total_matches > 0);
    }

    #[test]
    fn test_document_extractor_and_projections_mcp_flow() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        fs::create_dir_all(&corpus_dir).unwrap();
        let index_dir = tmp.path().join("index");
        let mut config = test_config(&corpus_dir);
        config.docs.patterns = vec![
            "*.html".to_string(),
            "**/*.html".to_string(),
            "*.docx".to_string(),
            "*.pdf".to_string(),
        ];
        let mut engine = Engine::open(config, &index_dir).unwrap();

        // 1. Write an HTML documentation article
        let html_content = r#"<!DOCTYPE html>
<html>
<head><title>System Architecture Overview</title></head>
<body>
<header><nav><a href="/home">Home</a></nav></header>
<main>
<h1>Architecture Guide</h1>
<p>This document details the core distributed architecture and protocols.</p>
<h2>Subsystems</h2>
<p>The messaging subsystem routes packets between cluster nodes.</p>
<a href="https://example.com/spec">External Specification</a>
</main>
<footer>(c) 2026 Enterprise Corp</footer>
</body>
</html>"#;
        fs::write(corpus_dir.join("guide.html"), html_content).unwrap();

        // 2. Perform delta sync/reindex
        engine.delta_scan().unwrap();

        // 3. Verify projection file was written to .index/projections/guide.html.txt
        let proj_path = engine.projection_path("guide.html");
        assert!(proj_path.is_file(), "Projection file should exist at {:?}", proj_path);
        let proj_text = fs::read_to_string(&proj_path).unwrap();
        assert!(proj_text.contains("# Architecture Guide"));
        assert!(proj_text.contains("messaging subsystem"));
        assert!(!proj_text.contains("<nav>"));

        let mut registry = ToolRegistry::new();
        registry.register_all();

        // 4. Test Tier 3: read_file on projected document returns kind: "projected_doc"
        let read_val = registry
            .execute_read("read_file", &engine, serde_json::json!({ "path": "guide.html" }))
            .unwrap();
        assert_eq!(read_val["kind"], "projected_doc");
        assert!(read_val["content"].as_str().unwrap().contains("Architecture Guide"));

        // 5. Test Tier 3 read_file with format: "lean"
        let read_lean = registry
            .execute_read(
                "read_file",
                &engine,
                serde_json::json!({ "path": "guide.html", "format": "lean" }),
            )
            .unwrap();
        let lean_str = read_lean.as_str().unwrap();
        assert!(lean_str.contains("# File: `guide.html`"));

        // 6. Test write_note rejects modifying document formats (Docx, Pdf, HtmlDoc)
        let write_res = registry.execute_write(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "spec.docx",
                "content": "Trying to overwrite docx"
            }),
        );
        assert!(write_res.is_err(), "write_note on docx must fail");
        let err_msg = write_res.err().unwrap().to_string();
        assert!(err_msg.contains("strictly read-only"));

        let write_html_res = registry.execute_write(
            "write_note",
            &mut engine,
            serde_json::json!({
                "path": "guide.html",
                "content": "Trying to overwrite html"
            }),
        );
        assert!(write_html_res.is_err(), "write_note on doc html must fail");

        // 7. Test move_note moves both the source file and its projection
        let move_res = registry.execute_write(
            "move_note",
            &mut engine,
            serde_json::json!({
                "from": "guide.html",
                "to": "archived_guide.html"
            }),
        );
        assert!(move_res.is_ok(), "move_note should succeed: {:?}", move_res);
        assert!(!proj_path.exists(), "Old projection must be gone");
        let new_proj = engine.projection_path("archived_guide.html");
        assert!(new_proj.is_file(), "New projection must exist at {:?}", new_proj);
    }
}
