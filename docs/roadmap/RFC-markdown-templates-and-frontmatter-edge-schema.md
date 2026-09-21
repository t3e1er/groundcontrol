# RFC: Markdown Templates & Frontmatter Edge Schema Standard

**Status**: Proposed  
**Author**: Architecture Team  
**Scope**: `groundcontrol-core`, `groundcontrol-common`, `groundcontrol-mcp`, `docs`  
**Date**: September 2026  
**Target Version**: `0.1.0`+  
**Related Documents**: [coderoadmap.md](file:///c:/dev/ctx/groundcontrol/docs/roadmap/coderoadmap.md), [GEMINI.md](file:///c:/dev/ctx/groundcontrol/GEMINI.md), [RFC-cross-corpus-graph-federation.md](file:///c:/dev/ctx/groundcontrol/docs/roadmap/RFC-cross-corpus-graph-federation.md)

---

## 1. Executive Summary & Problem Statement

`groundcontrol` relies on continuous knowledge crystallization (Principle 3): ephemeral agent interactions, architectural choices, and investigation traces are distilled into permanent, schema-validated notes.

Currently, template definition and edge configuration suffer from an architectural **split-brain**:
1. **Frontmatter & Section Schemas** are declared in TOML files located in `.templates/*.toml` ([`crates/groundcontrol-core/src/template.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/template.rs)).
2. **Graph Edge Relationships** derived from frontmatter (e.g., `supersedes`, `specifies`, `parent_of`) are configured separately in the global corpus configuration ([`CorpusConfig.graph.edge_types`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs#L159)).
3. **The Actual Notes** written by humans and agents are authored in Markdown (`.md`).

This tripartite split creates severe ergonomic friction:
* **No Authoring Scaffold**: `.toml` template files define abstract schemas, but provide no sample markdown content, heading boilerplate, or guidance comments. An agent calling `list_templates` receives disjoint field lists and must guess how to assemble the markdown body, frequently failing post-write validation.
* **Disconnected Edge Governance**: Creating a new note category (e.g., `adr`, `incident`, `rfc`) requires coordinating changes across `.templates/*.toml` and `gc.toml`. Adding an edge relationship to a template requires editing global corpus configuration.
* **Invisible in Tooling**: `.toml` template files cannot be rendered, previewed, or edited as markdown documents in IDEs (VS Code, Cursor, Antigravity) or PKM tools (Obsidian, Logseq, GitHub preview).

This RFC specifies a **unified `.templates/*.md` standard**: every template becomes a self-contained markdown document containing its own frontmatter schema, edge type declarations, and markdown body scaffold.

---

## 2. Invariant Constraints & Core Principles

Conforming to `groundcontrol`'s architectural invariants ([`GEMINI.md`](file:///c:/dev/ctx/groundcontrol/GEMINI.md)):

1. **Markdown on disk is authoritative ground truth (Invariant 1)**: Templates should themselves be markdown files on disk, readable by humans, agents, and IDEs alike.
2. **Deterministic graph topology (Invariant 2)**: Edges defined in template schemas are compiled directly into deterministic graph extraction rules—never inferred via stochastic LLM calls.
3. **No backwards compatibility shims (Greenfield Principle)**: Replace the legacy `.toml` template parser outright. All templates are `.md` files.
4. **Hexagonal architecture**: `Template` parsing and validation live behind domain ports; storage and MCP layers interact only through typed contracts.
5. **Open-world flexibility**: The schema must provide strict validation for declared structural fields and graph edges without artificially prohibiting arbitrary ad-hoc frontmatter keys or natural prose expansion.

---

## 3. The Unified `.templates/*.md` Standard

Each template lives in `.templates/<name>.md` (e.g., `.templates/adr.md`, `.templates/rfc.md`, `.templates/postmortem.md`).

### 3.1 Structural Anatomy

A template file consists of two sections separated by standard markdown boundaries:
1. **YAML Frontmatter (`--- ... ---`)**: Declares template identity, field schemas, edge extraction rules, section rules, and word count constraints.
2. **Markdown Body**: The exact starter scaffold, containing headings, guidance comments (`<!-- ... -->`), and placeholder text.

```markdown
---
# =============================================================================
# 1. Template Identity & Routing
# =============================================================================
template:
  name: adr
  description: "Architecture Decision Record"
  target_dir: "docs/architecture/adr"

# =============================================================================
# 2. Frontmatter Validation Schema
# =============================================================================
schema:
  fields:
    status:
      type: enum
      required: true
      values: [proposed, accepted, rejected, deprecated, superseded]
    date:
      type: date
      required: true
    deciders:
      type: list
      required: false

  # ===========================================================================
  # 3. Edge Topology Declarations (Self-Contained Edge Rules)
  # ===========================================================================
  edges:
    - field: supersedes
      type: Supersedes
      class: structural
      direction: outbound
      bidirectional: false
      target_template: adr
      required: false
      description: "Previous ADR superseded by this decision"
    - field: implements
      type: ImplementsSpec
      class: crossmodal
      direction: outbound
      bidirectional: false
      target_kind: code_symbol
      required: false
      description: "Code symbol or spec implemented by this ADR"

  # ===========================================================================
  # 4. Content Structure Rules
  # ===========================================================================
  sections:
    required: ["Context", "Decision", "Consequences"]
  min_words: 50
---
# ADR-{id}: {Title}

<!--
Guidance:
Explain the architectural context, evaluated alternatives, and rationale.
-->

## Context
{Describe the architectural context, forces, and constraints}

## Decision
{State the decision clearly and outline the architectural mechanism}

## Consequences
{Document positive, negative, and neutral trade-offs}
```

---

## 4. Architectural Design

```
.templates/*.md (Disk Ground Truth)
      │
      ▼
Template::load_from_dir
      │
      ├───► TemplateRegistry (In-Memory Port)
      │           │
      │           ├───► list_templates (MCP Tool) -> Exposes Schema + Markdown Scaffold
      │           │
      │           ├───► validate (MCP Tool) -> Checks Fields + Headings + Graph Edges
      │           │
      │           └───► write_note (MCP Tool) -> 1-Shot Hydration & Write
      │
      └───► Dynamic Edge Synthesis
                  │
                  ▼
            KnowledgeGraph (Edge Extraction)
            - Automatically extracts edges declared in template frontmatter
            - Enforces target_template & target_kind constraints
```

### 4.1 Domain Types in `groundcontrol-common`

Add template edge schema declarations to [`groundcontrol-common::config`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs):

```rust
/// Declarative edge rule defined directly inside a markdown template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateEdgeSchema {
    /// Frontmatter field name containing the link target(s).
    pub field: String,
    /// Edge type name emitted in the knowledge graph (e.g. "Supersedes").
    pub r#type: String,
    /// Edge class: structural, semantic, code, or crossmodal.
    #[serde(default)]
    pub class: EdgeClass,
    /// Traversal direction relative to this note: outbound or inbound.
    #[serde(default = "default_outbound")]
    pub direction: EdgeDirection,
    /// Whether to insert reverse edge automatically.
    #[serde(default)]
    pub bidirectional: bool,
    /// Target must follow a specific template (e.g. "adr").
    pub target_template: Option<String>,
    /// Target must be a specific kind (e.g. "code_symbol", "file", "doc").
    pub target_kind: Option<String>,
    /// Whether this edge field is required in frontmatter.
    #[serde(default)]
    pub required: bool,
    /// Human-readable documentation for this edge.
    pub description: Option<String>,
}
```

### 4.2 Template Model in `groundcontrol-core`

Extend [`Template`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/template.rs) to own the edge schemas and the raw markdown scaffold:

```rust
pub struct Template {
    pub name: String,
    pub description: Option<String>,
    pub target_dir: Option<String>,
    pub required_fields: Vec<FieldSchema>,
    pub optional_fields: Vec<FieldSchema>,
    pub edges: Vec<TemplateEdgeSchema>,
    pub required_sections: Vec<String>,
    pub min_word_count: Option<usize>,
    /// The unparsed markdown body below the YAML frontmatter.
    pub scaffold: String,
}
```

### 4.3 Ingestion Pipeline: `Template::load_from_dir`

1. Scans `templates_dir` for all `*.md` files.
2. Extracts YAML frontmatter and markdown body via `groundcontrol_core::parser::split_frontmatter_and_content`.
3. Deserializes frontmatter into the template schema struct.
4. Stores the body as `scaffold`.
5. Converts `TemplateEdgeSchema` entries into standard [`EdgeTypeConfig`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs#L164) items.

### 4.4 Dynamic Edge Registration in Graph Building

When the indexing pipeline builds edges for a document ([`build_edges_for_document`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/graph/mod.rs#L472)):
1. It applies the corpus-level `config.graph.edge_types`.
2. If `doc.template` is present and matches a loaded template in `TemplateRegistry`, it additionally evaluates the template's declared `edges`.
3. Graph edges are inserted with the configured `class`, `direction`, and provenance [`EdgeProvenance::Frontmatter`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/types.rs#L557).

---

## 5. Topological Graph Validation

Validation extends beyond field presence and word counts to **graph-level structural integrity**:

1. **Target Existence**: When note A declares `supersedes: "docs/adr/001.md"`, validation verifies that `docs/adr/001.md` exists in the metadata catalog or graph node index.
2. **Template Type Constraint (`target_template`)**: If `target_template = "adr"`, validation checks that the resolved target note has `template: adr`.
3. **Cross-Modal Target Constraint (`target_kind = "code_symbol"`)**: If targeting a code symbol, validation checks that the identifier resolves to a known symbol in the SQLite catalog.
4. **Actionable Diagnostics**: Emits clear `ValidationIssue` entries with severity, field name, and remediation guidance.

---

## 6. MCP Tool Surface Improvements

### 6.1 `list_templates`
Now returns the complete template specification, including the **scaffold markdown**:
```json
{
  "name": "adr",
  "description": "Architecture Decision Record",
  "scaffold": "# ADR-{id}: {Title}\n\n## Context\n...\n\n## Decision\n...\n\n## Consequences\n...",
  "fields": [...],
  "edges": [
    {
      "field": "supersedes",
      "type": "Supersedes",
      "class": "structural",
      "target_template": "adr"
    }
  ]
}
```
**Agent Advantage**: An agent can fetch `.templates/adr.md` and immediately hydrate the scaffold in a single turn without hallucinating headings or frontmatter keys.

### 6.2 `validate`
Accepts `path` and checks:
- Scalar/Enum field schemas (`FieldSchema`).
- Content sections and word counts.
- **Topological edge rules (`TemplateEdgeSchema`)**.
- Graph taxonomy checks (cycles, orphans, broken links).

---

## 7. Migration & Non-Goals

### 7.1 Non-Goals
* Supporting backwards compatibility for `.templates/*.toml`. Per greenfield principles, `.toml` templates will be deprecated and removed outright.
* Forcing loose/unstructured notes into templates. Untemplated notes remain 100% supported and continue to use open wikilinks and `#tags`.

### 7.2 Migration Plan
1. Convert any test `.toml` templates to `.md`.
2. Update `Template::load_from_dir` to read `.md` files.
3. Wire template edge definitions into graph indexing and validation.
4. Update `list_templates` output serialization to include scaffolds.
5. Ship standard starter templates: `.templates/adr.md`, `.templates/rfc.md`, `.templates/guide.md`.
