---
title: "ADR 016: Generic-Normalized Scope Path Resolution for Code Symbols"
category: "code-architecture"
status: "accepted"
tags: ["adr", "ast", "generics", "normalization", "sqlite", "decision"]
related:
  - "[[docs/code-architecture/index]]"
  - "[[docs/code-architecture/generic-scope-normalization]]"
---

# ADR 016: Generic-Normalized Scope Path Resolution for Code Symbols

## Status
Accepted / Implemented

## Context
In languages with rich generic systems (Rust, C++, TypeScript), AI agents frequently query symbols omitting lifetime annotations or concrete type parameters (e.g. searching `EarlyBinder > instantiate` instead of `EarlyBinder<'tcx, T> > instantiate`). Because symbol qualified names were stored verbatim in SQLite, exact string equality lookups yielded `404 Not Found`, forcing agents into frustrating trial-and-error cycles.

## Decision
We implemented a two-stage **Generic-Normalized Scope Resolution** algorithm:
1. Pure normalization function `normalize_scope_path` that strips balanced `<...>` and lifetimes while preserving ` > ` hierarchy delimiters.
2. In `crates/groundcontrol-core/src/persistence/mod.rs`, when `find_symbols_by_qualified_name` yields 0 exact matches, query SQLite with normalized component matching on symbol leaf and scope path prefix.
3. If multiple parameterized signatures match, return candidate signatures for disambiguation.

## Consequences

### Positive
- Queries like `EarlyBinder > instantiate` resolve immediately to the method implementation in ~200 tokens.
- Eliminates 404 lookup failures for AI agents navigating generic codebases.
- Sub-millisecond lookup latency preserved via indexed SQLite queries.

### Trade-offs
- Overloaded methods across different generic specializations require candidate disambiguation.
