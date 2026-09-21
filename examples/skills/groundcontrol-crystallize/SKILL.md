---
name: groundcontrol-crystallize
description: >-
  Distill raw episodic traces, discussion logs, and debugging sessions into permanent semantic knowledge notes.
  Use this skill to crystallize concepts, track knowledge lineage with Cypher-Lite, and ensure graph health.
---

# GroundControl Knowledge Crystallization

This skill teaches agents how to implement **Continuous Knowledge Crystallization (Principle 3)**: transforming ephemeral, high-entropy conversational and debugging traces into structured, durable semantic assets with formal provenance using the 17-tool suite.

---

## 1. The Crystallization Workflow

When an engineering task, debugging session, or design consensus produces non-obvious knowledge:

1. **Identify Volatile Episodic Context**:
   Review the conversation trajectory or scratch log to isolate:
   - Root causes of subtle bugs
   - Architecture decisions and trade-offs
   - Recurring implementation patterns or invariants
2. **Discover Available Schema Templates**:
   Call `list_templates` to select the appropriate schema (e.g. `system_concept`, `decision_record`).
3. **Crystallize into Semantic Note with Lineage**:
   Call `write_note` with `mode="create"`, supplying structured frontmatter with `derived_from`:
   ```json
   {
     "path": "concepts/sqlite-concurrency-patterns.md",
     "mode": "create",
     "content": "---\ntitle: \"SQLite WAL Concurrency and Shared Cache\"\nstatus: accepted\ntemplate: system_concept\nderived_from: \"incidents/inc-001-index-lock.md\"\ntags:\n  - sqlite\n  - concurrency\n  - storage\n---\n\n# SQLite WAL Concurrency\n\n## Overview\nMechanisms for multi-reader single-writer SQLite WAL concurrency.\n\n## Mechanisms\n...\n\n## Trade-Offs\n..."
   }
   ```
4. **Validate Immediate Conformance**:
   Call `validate` on the newly crystallized note:
   ```json
   {
     "path": "concepts/sqlite-concurrency-patterns.md"
   }
   ```
5. **Trace Knowledge Lineage with Cypher-Lite**:
   To inspect the origin of a concept and view upstream decisions or source notes, call `graph_match`:
   ```json
   {
     "pattern": "(:DocNode {path: \"concepts/sqlite-concurrency-patterns.md\"})-[:derived_from*1..3]->(source)",
     "edge_class": "semantic"
   }
   ```

---

## 2. Graph Health & Density Auditing

To maintain high knowledge quality and graph connectivity:

1. **Inspect Graph Topology & Density**:
   Call `status` with `scope="graph"` to review total nodes, edge density, and connected components:
   ```json
   {
     "scope": "graph"
   }
   ```
2. **Detect Architectural Clusters**:
   Call `graph_communities` with `view="architecture"` to verify how the new concept clusters with existing subsystems:
   ```json
   {
     "view": "architecture"
   }
   ```
3. **Audit Index Coverage**:
   Call `status` with `scope="coverage"` to ensure newly added directories and notes are indexed:
   ```json
   {
     "scope": "coverage"
   }
   ```
