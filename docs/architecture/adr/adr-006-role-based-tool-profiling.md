---
title: "ADR 006: Role-Based Tool Profiling (Scout, Analysis, All)"
category: "agentic-strategy"
status: "accepted"
tags: ["adr", "tool-profiling", "mcp", "security", "decision"]
related:
  - "[[docs/agentic-strategy/index]]"
  - "[[docs/agentic-strategy/tool-profiling]]"
  - "[[docs/agentic-strategy/swarm-topologies]]"
---

# ADR 006: Role-Based Tool Profiling (Scout, Analysis, All)

## Status
Accepted / Implemented

## Context
As `groundcontrol` expanded to 39 registered MCP tools, serializing the full tool registry in `tools/list` consumed ~5,500 tokens of prompt context on every interaction. Furthermore, exposing mutating tools (`delete_note`, `move_note`, `reindex_corpus`) to read-only exploratory agents created operational security risks.

## Decision
We implemented a hierarchical `--profile` command-line flag in `groundcontrol-cli` and `groundcontrol-mcp`:
$$\text{scout} \subset \text{analysis} \subset \text{all}$$

1. **`scout` (9 tools)**: Minimal retrieve/navigate set for fast exploratory agents.
2. **`analysis` (30 tools)**: Adds read-only graph algorithms, template validators, and code intelligence.
3. **`all` (39 tools)**: Full suite including mutating and system administration tools.

## Consequences

### Positive
- Conserves ~4,500 prompt tokens per request for scout and navigator agents.
- Provides defense-in-depth against accidental corpus mutations.
- Enables clean swarm separation where specialized agents receive exactly the capabilities they require.

### Trade-offs
- Tool availability must be configured per agent process; however, any hidden tool invoked directly still executes if known by the caller.
