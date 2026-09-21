# Cursor AI Steering Rules (.cursorrules)

Place the following content into `.cursorrules` (or `.cursor/rules/groundcontrol.mdc`) in your project root to instruct Cursor AI when using `groundcontrol` MCP server tools.

```markdown
# GroundControl Knowledge & Codebase Protocol

You have access to the `groundcontrol` MCP server (17 authoritative tools). Follow these rules when querying or modifying knowledge and source code in this repository:

1. Retrieval Strategy & Turn 1 Snippets:
   - Use `search` with `mode="hybrid"` as your default exploratory query tool.
   - `search` automatically returns Turn 1 inline text and code snippets (`snippets: 3` by default) along with graph affordances (`calls_in`, `calls_out`, `implements`, `imports`, `wikilinks_in`). Work directly from these snippets whenever possible to avoid unnecessary round-trips.
   - Use `search` with `mode="bm25"` when searching for exact identifier names, error strings, struct symbols, or CLI flags.
   - Use `search` with `mode="semantic"` for abstract natural-language concepts.
   - Use `search_related` for Personalized PageRank expansion around known seed notes or symbols.

2. Bounded Code & Document Inspection:
   - Tier 1: Survey results via `search` (inspect handles, snippets, and graph affordances).
   - Tier 2: Fetch exact symbol definitions or bounded doc chunks with `get_snippet(symbol="...")` or `get_snippet(path="...", chunk_index=0)`.
   - Tier 3: Call `read_file` with line bounds (`start_line`, `end_line`) only when exhaustive context is required. Do NOT dump entire large files into context.

3. Graph Navigation with Cypher-Lite (`graph_match`):
   - To explore relationships, call `graph_match` using linear Cypher-Lite ASCII patterns:
     - Trace call graphs: `(:CodeSymbol {name: "MyFunction"})-[:calls*1..2]->(target)`
     - Trace implementations: `(source)-[:implements]->(target)`
     - Trace doc ancestry: `(:DocNode {path: "adrs/001.md"})-[:derived_from*1..3]->(target)`
   - Filter by `edge_class`: `"code"`, `"structural"`, `"semantic"`, `"crossmodal"`, or `"hybrid"`.
   - Use `graph_communities(view="architecture")` for high-level architectural component maps.

4. Note Creation & Schema Discipline:
   - Before authoring a new document or ADR, call `list_templates` to discover available schemas.
   - Author or update notes via `write_note(path="...", mode="create"|"overwrite"|"append")`.
   - Always run `validate(path="...")` immediately after creating or modifying a note to ensure zero schema errors.

5. Principle 3 Knowledge Crystallization:
   - When resolving complex architectural questions, subtle bugs, or incident resolutions, crystallize the findings into a permanent note using `write_note` (with `derived_from` frontmatter) and verify lineage via `graph_match`.
```
