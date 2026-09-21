---
title: "ADR 020: Lean Multiline Text Emission Protocol Across Progressive Disclosure Turns"
category: "architecture"
status: "accepted"
tags: ["adr", "tokens", "multiline-text", "progressive-disclosure", "context-window", "mcp-transport"]
related:
  - "[[docs/architecture/adr/adr-004-progressive-disclosure-token-contract]]"
  - "[[docs/architecture/adr/adr-010-unified-modal-search-tool]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-model]]"
  - "[[docs/concepts/progressive-disclosure/turn-1-affordances]]"
  - "[[docs/roadmap/RFC-lean-multiline-text-emission]]"
  - "[[docs/roadmap/coderoadmap]]"
---

# ADR 020: Lean Multiline Text Emission Protocol Across Progressive Disclosure Turns

## Status
Accepted / Implemented (September 2026)

## Context

In the Model Context Protocol (MCP) specification, tool call results are delivered to clients inside `content: Vec<Content>`, where `Content::Text { text: String }`. When tools serialize structured domain models to JSON (via `serde_json::to_string` or `serde_json::to_string_pretty`), client hosts (such as Claude Desktop, Cursor, and Antigravity) do not re-hydrate this text into structured language objects for the LLM; **they inject the raw serialized string directly into the prompt context**.

### The Multi-Turn Context Tax

Standard JSON serialization imposes a severe context window tax and increases autoregressive perplexity across agentic coding loops:

1. **Repetitive Key Overhead**: In hierarchical graph traversals (`graph_match`) or search hit lists (`search`), every item redundantly emits keys (`"node":`, `"rel":`, `"path":`, `"line":`, `"score":`), quotes, and braces. For a 15-node graph traversal, over 65% of tokens are syntactic scaffolding.
2. **Closing Delimiter Cascades**: Deeply nested JSON objects produce trailing waterfalls of closing brackets (`}]}}, \n }]}}`), consuming token output limits with zero semantic information.
3. **Double Escaping on Turn 3 Code Reads**: When emitting code or whole files (`read_file`, `get_snippet`) inside JSON strings, every newline is escaped as `\n`, every quote as `\"`, and backslashes as `\\`, inflating token count by 15-25% and degrading code understanding in language models.
4. **Attention Dilution**: Autoregressive transformer attention heads are optimized for natural language and indented source code. High-entropy JSON punctuation dilutes attention away from primary file paths, line ranges, and symbols.

Inspired by the compact output philosophy of `codebase-memory-mcp`'s `compact_out`, groundcontrol required an industrial-grade, semantically rich multiline text protocol across all progressive disclosure tiers.

## Decision

Standardize on **Lean Multiline Text Emission** across Turns 1, 2a, 2b, and 3:

1. **Native Wire Emission**: At the MCP transport boundary in [`dispatch.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/transport/dispatch.rs), if a tool handler produces `Value::String(s)`, the dispatch layer emits the raw string directly into `CallToolResponse { content: vec![Content::Text { text: s }] }` without re-wrapping it in JSON string escaping.
2. **Default Wire Protocol**: MCP tool dispatch defaults `arguments.format` to `"lean"` for all external client invocations. Clients needing machine-parseable JSON can explicitly pass `format: "json"`.
3. **Turn 1 (`search`) Lean Protocol**: Emits human/agent-readable Markdown with partitioned hits (`## Code Hits`, `## Doc Hits`), stripped zero score components, bounded Turn 1 snippets in fenced code blocks, and explicit next-turn affordance scents:
   - `-> [T2a fetch] get_snippet(name: "...")`
   - `-> [T2b graph] graph_match("...")`
4. **Turn 2a (`get_snippet`) Lean Protocol**: Emits bounded definition blocks with 1-based prefixed line numbers (`L<num>: `), docstrings in markdown blockquotes (`> **Docstring**:`), grammar-driven incoming/outgoing relationships, and outbound navigation hints (`-> [T2b callers]`, `-> [T3 full file]`). Disambiguation and candidate near-misses are emitted as clear, numbered markdown lists.
5. **Turn 2b (`graph_match`) Lean Protocol**: Emits 2-space indented Cypher-Lite ASCII hierarchy trees (`-[:calls]->`, `<-[:defines]-`), hub node suppression (`... (+N more)`), cycle detection markers (`[CYCLE: -> target]`), and direct jump targets.
6. **Turn 3 (`read_file`) Lean Protocol**: Emits line-numbered markdown blocks for single files or batch arrays (`paths: [...]`) with clear file headers (`# File: \`path\` [lines: L1-L100 of 250, language: rust]`), avoiding all JSON escape sequences.

## Implementation Architecture

The protocol is implemented in the pure Rust formatting engine [`crates/groundcontrol-mcp/src/format/lean.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/format/lean.rs) and wired into tool handlers in [`crates/groundcontrol-mcp/src/tools/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs):

```mermaid
flowchart TD
    subgraph MCP Tool Invocation
        Req["MCP CallToolRequest"] --> Disp["dispatch.rs (defaults format='lean')"]
        Disp --> Tool["Tool Handler (search, get_snippet, graph_match, read_file)"]
    end

    subgraph Formatting Engine (format/lean.rs)
        Tool --> Formatter["format::lean Formatter"]
        Formatter --> T1["format_lean_search (Turn 1 Hits + T2 Scents)"]
        Formatter --> T2a["format_lean_code_symbol / format_lean_doc_chunk (Turn 2a L<num>: Blocks)"]
        Formatter --> T2b["format_lean_graph_match (Turn 2b Indented ASCII Tree)"]
        Formatter --> T3["format_lean_read_file (Turn 3 Markdown File Slices)"]
    end

    subgraph LLM Agent Context
        T1 --> Wire["Value::String (Raw String in Content::Text)"]
        T2a --> Wire
        T2b --> Wire
        T3 --> Wire
        Wire --> LLM["LLM Prompt Context (60-70% Token Savings, High Signal)"]
    end
```

## Consequences

### Positive
- **60% to 70% Token Savings**: Drastically reduces prompt tokens consumed by search results and graph trees, allowing agents to retain larger working contexts across extended sessions.
- **Zero JSON Escaping Overhead**: Turn 3 source file reads and Turn 2a snippets emit genuine code blocks without `\n` and `\"` escaping artifacts.
- **Enhanced Agent Steering**: Every turn provides deterministic, semantically non-duplicative next-step handles (`-> [T2a fetch]`, `-> [T2b graph]`, `-> [T3 full file]`) steering the LLM smoothly through the 3-Tier progressive disclosure model.
- **Dual Support**: Automated test suites and programmatic MCP clients retain access to structured JSON via `format: "json"`.

### Negative / Trade-offs
- Tool handlers must maintain multiline text formatters in addition to domain structs.
- Unit tests asserting on wire text must inspect formatted Markdown/ASCII rather than JSON keys (addressed via `format::lean::tests` and comprehensive end-to-end tool tests).
