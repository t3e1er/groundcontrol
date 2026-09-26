# GroundControl Agent Steering Rules (Antigravity & Gemini IDE)

Copy this file into `.agents/rules/groundcontrol-rules.md` or as `GEMINI.md` in your repository root to guide your AI pair programmer. Synchronized with `.kiro/steering/`.

---

## Directives

1. **Markdown & Source Authority**: Files on disk are the ultimate ground truth. Indices are disposable retrieval caches.
2. **Modal Search Selection & Turn 1 Snippets** (`search` tool):
   - `mode="hybrid"`: Default for general research and broad technical queries (3-way RRF).
   - `mode="bm25"`: For exact function names, struct fields, error messages, and verbatim tokens.
   - `mode="semantic"`: For exploring abstract concepts and semantic analogies.
   - `mode="graph"`: For relationship and dependency queries across typed edges (`calls`, `implements`, `defines`, `imports`, `wikilink`, `parent_child`, `supersedes`).
   - `snippets: 3`: Directly leverage top snippets in Turn 1 without issuing secondary read calls unless deeper context is necessary.
3. **Progressive Disclosure**:
   - Tier 1: Query `search` (inspect candidate handles, snippets, and graph affordances).
   - Tier 2: Use `get_snippet` for bounded single symbols/chunks and caller/callee signatures.
   - Tier 3: Use `read_file` with line bounds (`[start_line, end_line]`) when exhaustive file context is necessary.
4. **Cypher-Lite Graph Traversal** (`graph_match` tool):
   - Traversal syntax: `(:CodeSymbol {name: "foo"})-[:calls*1..2]->(target)` or `(:DocNode {path: "adrs/001.md"})-[:derived_from*1..3]->(target)`.
   - Filter by `edge_class`: `"code"`, `"structural"`, `"semantic"`, `"crossmodal"`, or `"hybrid"`.
5. **Schema Discipline & Validation**:
   - Before creating notes, call `list_templates`.
   - Author via `write_note(path="...", mode="create"|"overwrite"|"append")`.
   - Verify compliance immediately with `validate(path="...")`.
6. **Knowledge Crystallization (Principle 3)**:
   - Transform valuable debugging outcomes and architectural resolutions into durable notes via `write_note` (setting `derived_from`) and trace lineage with `graph_match`.
7. **Workspace MCP Configuration Exclusivity**:
   - Exclusively configure workspace MCP servers in `.agents/mcp_config.json`. Never modify global machine configuration files.
