---
title: "ADR 009: Greenfield Engineering Discipline & Zero Backwards Compatibility"
category: "code-architecture"
status: "accepted"
tags: ["adr", "greenfield", "discipline", "no-debt", "decision"]
related:
  - "[[docs/code-architecture/index]]"
  - "[[docs/code-architecture/pure-rust-invariants]]"
---

# ADR 009: Greenfield Engineering Discipline & Zero Backwards Compatibility

## Status
Accepted / Implemented

## Context
Enterprise codebases frequently accumulate technical debt by attempting to maintain backwards-compatibility shims, deprecated function aliases, and legacy on-disk index formats for non-existent external consumers. This increases cognitive overhead, complicates compiler verification, and slows down development.

## Decision
`groundcontrol` operates under strict **Greenfield Engineering Discipline**:
1. **No Backwards Compatibility Shims**: When an API, tool signature, or index layout changes, the old shape is replaced outright.
2. **Disposable Derived Indices**: Indices (`.index/`) are derived and 100% rebuildable. Schema changes trigger an index rebuild rather than carrying legacy migration logic.
3. **Zero Dead Code**: Clippy runs with `-D warnings`. Any function, struct, or branch made obsolete by a change must be deleted within the same commit. Blanket `#[allow(dead_code)]` is strictly prohibited.

## Consequences

### Positive
- The codebase remains minimal, coherent, and free of obsolete code branches.
- Reviewers and AI agents only need to understand one active implementation path for every feature.
- Refactoring is fearless and rapid.

### Trade-offs
- Upgrades to on-disk index schemas require wiping and rebuilding `.index/` (which runs in seconds to minutes).
