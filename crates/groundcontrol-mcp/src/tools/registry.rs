//! Tool registry, handler dispatching, and multi-corpus routing.

use std::collections::HashMap;

use serde_json::Value;

use groundcontrol_common::{Error, Result};
use groundcontrol_core::corpus_manager::CorpusManager;
use groundcontrol_core::engine::Engine;
use groundcontrol_core::search;

use super::graph::{
    handle_graph_communities, handle_graph_match, handle_trace_cross_corpus,
    handle_trace_cross_corpus_dummy,
};
use super::read::{handle_get_snippet, handle_list_notes, handle_read_file};
use super::search::{handle_search, handle_search_related};
use super::system::{
    handle_get_status, handle_index_corpus_dummy, handle_index_corpus_manager,
    handle_list_corpora_dummy, handle_list_corpora_manager, handle_status, handle_sync_corpus,
    handle_unload_corpus_dummy, handle_unload_corpus_manager,
};
use super::template::{handle_list_templates, handle_validate};
use super::write::{handle_delete_note, handle_move_note, handle_write_note};

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
            "Tier 1 retrieval with Turn 1 snippets: returns handles across docs and code with inlined source snippets for top K results (configured via `snippets`, default 3). Modes: hybrid (default: dense ONNX for docs, binary Hamming for code, fused with BM25 + graph), bm25, semantic, graph, explain, fast (pure CPU SIF + Binary + PPR).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "mode": { "type": "string", "enum": ["bm25", "semantic", "hybrid", "graph", "explain", "fast"], "description": "Retrieval mode (default: hybrid). hybrid automatically routes docs through dense ONNX vectors and code through 256-bit binary Hamming fingerprints, fused with BM25 and Petgraph. Use 'fast' for instant sub-millisecond CPU-only search." },

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
pub(crate) fn build_dynamic_schema_envelope(
    items: &[groundcontrol_common::types::SearchResult],
    is_code: bool,
    extra_active_edges: &[String],
) -> groundcontrol_common::types::SchemaEnvelope {
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
                    groundcontrol_common::types::EntityKind::CodeSymbol { symbol_type, .. } => {
                        labels.insert(format!("{:?}", symbol_type));
                    }
                    groundcontrol_common::types::EntityKind::CodeChunk { .. } => {
                        labels.insert("CodeChunk".to_string());
                    }
                    groundcontrol_common::types::EntityKind::CodeFile { .. } => {
                        labels.insert("CodeFile".to_string());
                    }
                    groundcontrol_common::types::EntityKind::Documentation { .. } => {
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

    groundcontrol_common::types::SchemaEnvelope {
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
            let mut docs_tagged: Vec<(String, Vec<groundcontrol_common::types::SearchResult>)> =
                Vec::new();
            let mut code_tagged: Vec<(String, Vec<groundcontrol_common::types::SearchResult>)> =
                Vec::new();
            for (corpus_name, value) in per_corpus {
                let resp: groundcontrol_common::types::SearchResponse =
                    serde_json::from_value(value)
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
                Some(groundcontrol_common::types::SearchPartition {
                    total_matches: merged_docs.len(),
                    top_k_returned: merged_docs.len(),
                    schema_envelope: build_dynamic_schema_envelope(&merged_docs, false, &[]),
                    results: merged_docs,
                })
            } else {
                None
            };
            let code_partition = if !merged_code.is_empty() {
                Some(groundcontrol_common::types::SearchPartition {
                    total_matches: merged_code.len(),
                    top_k_returned: merged_code.len(),
                    schema_envelope: build_dynamic_schema_envelope(&merged_code, true, &[]),
                    results: merged_code,
                })
            } else {
                None
            };
            let resp = groundcontrol_common::types::SearchResponse {
                docs: docs_partition,
                code: code_partition,
            };
            return serde_json::to_value(resp)
                .map_err(|e| Error::Config(format!("serialize merged search response: {}", e)));
        }

        // If every successful output is a JSON array, treat as search-style and RRF-merge.
        let all_arrays = per_corpus.iter().all(|(_, v)| v.is_array());
        if all_arrays {
            let mut tagged_lists: Vec<(String, Vec<groundcontrol_common::types::SearchResult>)> =
                Vec::with_capacity(per_corpus.len());
            for (corpus_name, value) in per_corpus {
                let results: Vec<groundcontrol_common::types::SearchResult> =
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
            serde_json::from_value::<Vec<groundcontrol_common::types::SearchResult>>(output.clone())
        {
            let tagged: Vec<groundcontrol_common::types::SearchResult> =
                results.into_iter().map(|r| r.with_corpus(Some(corpus_name.to_string()))).collect();
            return serde_json::to_value(tagged).unwrap_or(output);
        }
    } else if output.is_object() && (output.get("docs").is_some() || output.get("code").is_some()) {
        if let Ok(mut resp) =
            serde_json::from_value::<groundcontrol_common::types::SearchResponse>(output.clone())
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
