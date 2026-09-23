---
title: "ADR 021: Modular Retrieval Algorithm Substrate"
category: "code-architecture"
status: "accepted"
tags: ["adr", "algorithm", "substrate", "ports", "retrieval", "ablation", "decision"]
related:
  - "[[docs/architecture/adr/index]]"
  - "[[docs/architecture/adr/adr-007-hexagonal-ports-adapters-isolation]]"
  - "[[docs/architecture/implementation/algorithm-substrate]]"
---

# ADR 021: Modular Retrieval Algorithm Substrate

## Status
Accepted / Implemented

## Context
Historically, the `Engine` (`indexer.rs` and `state.rs`) acted as a monolithic coordinator for all indexing operations. Parsing ASTs, generating 256-bit binary fingerprints, building Petgraph edges, and queuing vector batches were intermingled in a single ~250-line loop in `index_file_staged`.

This design had major drawbacks:
1. **Tight Coupling**: Adding or modifying an algorithm required altering engine internals and touched multiple files.
2. **Impaired Ablation**: The sister benchmark repository (`groundtruth`) had to rely on complex engine mocking or black-box JSON-RPC subprocesses to isolate algorithmic performance.
3. **Monolithic Testing**: Tests for individual indexing paths were mixed in a giant `tests.rs` file rather than localized to the relevant retrieval method.

## Decision
We refactored the indexing and retrieval pipeline into an **Algorithm Substrate / Component Architecture**:
1. Defined the [`RetrievalAlgorithm`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/ports/algorithm.rs) port trait in `groundcontrol-common::ports::algorithm`.
2. Created a clean intermediate representation, [`ParsedArtifact`](file:///c:/dev/semantic/groundcontrol/crates/groundcontrol-common/src/types/artifact.rs), separating pure CPU AST parsing from downstream index ingestion.
3. Decomposed algorithms into isolated subdirectories under `crates/groundcontrol-core/src/algorithm/`:
   - `bm25/`: `mod.rs`, `types.rs`, `tests.rs`
   - `binary/`: `mod.rs`, `types.rs`, `tests.rs`
   - `graph/`: `mod.rs`, `types.rs`, `tests.rs`
   - `dense/`: `mod.rs`, `types.rs`, `tests.rs`
   - `composite/`: `mod.rs`, `types.rs`, `tests.rs`, `rrf.rs`
   - `eval/`: `mod.rs`, `config.rs`, `index.rs`, `hit.rs`, `query.rs`, `sanitizer.rs`, `tests.rs`
4. Replaced the monolithic ingestion loop in `indexer.rs` with `broadcast_artifact(&parsed_artifact)`, delegating document ingestion to each algorithm component.
5. Provided inherent methods and `Deref` / `DerefMut` implementations on algorithm wrappers for zero-overhead backwards ergonomics.

## Consequences

### Positive
- **High Modularity**: Each algorithm manages its own storage schema, indexing logic, and query lifecycle.
- **Isolated Unit Testing**: Each algorithm features its own localized `tests.rs` test suite.
- **Sub-Millisecond Ablation**: `groundtruth` can instantiate and evaluate individual algorithms or combinations directly via `groundcontrol-core` without intermediate wrapper crates or MCP overhead.
- **Clean Ingestion Pipeline**: `index_file_staged` and `ingest_parsed_record` are simplified from monolithic multi-stage blocks to clean parse-and-broadcast invocations.

### Trade-offs
- Passing `ParsedArtifact` involves cloning or referencing intermediate chunk and symbol lists during the broadcast phase (mitigated by passing references `&ParsedArtifact`).
