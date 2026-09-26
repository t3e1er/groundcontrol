# GroundControl — Gemini & Antigravity Steering Rules

Place this file at the root of your repository as `GEMINI.md` (or `.agents/rules/groundcontrol.md`) to guide Gemini Code Assist and Google Antigravity agents when working with `groundcontrol`.

---

## Directives

1. **Markdown & Source Authority**: Files on disk are the ultimate ground truth. Indices (Tantivy BM25, HNSW vectors, SQLite metadata, Petgraph) are disposable, 100% rebuildable retrieval caches.
2. **Modal Search Selection & Turn 1 Grounding** (`search` tool):
   - `mode="hybrid"`: Default for general research and broad technical queries (3-way RRF across BM25, ONNX vectors, and graph).
   - `mode="bm25"`: For exact function names, struct fields, error messages, and verbatim tokens.
   - `mode="semantic"`: For exploring abstract concepts and semantic analogies.
   - `mode="graph"`: For relationship and dependency queries across typed edges.
   - `mode="explain"`: Introspect scoring breakdowns (BM25 vs vector vs graph).
   - `snippets: 3`: Search automatically returns inline text/code snippets for top hits directly in Turn 1. Ground your answers on these snippets to avoid unnecessary tool calls.
   - Graph Affordances: Pay attention to `graph_affordances` (`calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`) returned in Turn 1 to assess entity centrality.
3. **Progressive Disclosure (3 Tiers)**:
   - **Tier 1**: Query `search` (inspect partitioned `docs` and `code` hits + snippets + affordances).
   - **Tier 2**: Fetch targeted symbol definitions or doc chunks via `get_snippet`.
   - **Tier 3**: Read full file contents or line slices (`[start_line, end_line]`) via `read_file` only when exhaustive file context is required.
4. **Cypher-Lite Graph Traversal** (`graph_match` tool):
   - Query typed paths using linear Cypher-Lite ASCII patterns:
     - `(:CodeSymbol {name: "Server"})-[:calls*1..2]->(target)`
     - `(:DocNode {path: "adrs/001.md"})-[:derived_from*1..3]->(target)`
     - `(source)-[:implements]->(target)`
   - Filter by `edge_class`: `"code"`, `"structural"`, `"semantic"`, `"crossmodal"`, or `"hybrid"`.
   - Bounded depths (`*1..5`) and cycle guards execute under sub-millisecond SQLite CTEs.
5. **Schema Discipline on Writes**:
   - Query `list_templates` before authoring.
   - Author via `write_note(path="...", mode="create"|"overwrite"|"append"|"prepend")`.
   - Verify immediately with `validate(path="...")` or `validate(check_taxonomy=true)`.
6. **Knowledge Crystallization (Principle 3)**:
   - Distill debugging breakthroughs and architectural consensus into permanent notes using `write_note` (linking `derived_from` in frontmatter).
   - Trace lineage and provenance with `graph_match`.
7. **Greenfield Discipline**: No backwards compatibility shims, no dead code, clippy `-D warnings`.
8. **Workspace MCP Configuration Exclusivity**: Exclusively use `.agents/mcp_config.json` for workspace MCP server configuration and install changes. Never modify global machine configs (`~/.gemini/antigravity-ide/mcp_config.json` or `~/.gemini/config/mcp_config.json`).
