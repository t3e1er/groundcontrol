# Windsurf Cascade Steering Rules (.windsurfrules)

Add this snippet to `.windsurfrules` or project configuration in Windsurf:

```markdown
# GroundControl Integration Guidelines

- Always prefer `groundcontrol` MCP tools (17 tools) for knowledge base search and codebase graph exploration.
- Use `search(mode="hybrid")` for natural language questions. Leverage the Turn 1 inline snippets and graph affordances directly.
- Use `search(mode="bm25")` for verbatim tokens, symbols, and CLI flags.
- Use `get_snippet(symbol="...")` or `get_snippet(path="...", chunk_index=0)` for bounded symbol or chunk inspection.
- Use `graph_match(pattern="...")` with Cypher-Lite linear patterns (e.g., `(:CodeSymbol {name: "foo"})-[:calls*1..2]->(target)`) to explore call graphs or module dependencies.
- Only call `read_file` with `start_line` and `end_line` bounds when full file inspection is required.
- When creating notes, discover schemas using `list_templates`, author via `write_note`, and verify compliance with `validate`.
- Crystallize lasting engineering decisions into notes with `write_note` (linking `derived_from`) and trace lineage with `graph_match`.
```
