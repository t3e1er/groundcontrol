---
name: groundcontrol-curate
description: >-
  Create, update, move, and formally validate markdown notes against corpus schemas and taxonomies.
  Use this skill when authoring Architecture Decision Records (ADRs), creating technical documentation,
  updating frontmatter metadata, or verifying corpus structural integrity.
---

# GroundControl Knowledge Curation & Schema Validation

This skill guides agents through drafting, updating, moving, and formally validating notes and taxonomy hierarchies in `groundcontrol` knowledge bases using the authoritative 17-tool suite.

---

## 1. Creating Notes from Templates

When authoring a new note (e.g. ADR, Incident Report, Architecture Concept):

1. **List Available Templates**:
   Call `list_templates` to discover the schema definitions configured in the corpus `.templates/` directory:
   ```json
   {}
   ```
2. **Inspect Template Requirements**:
   Note required frontmatter fields (e.g. `title`, `status`, `date`, `tags`), optional fields, and required section headers.
3. **Write the Note**:
   Call `write_note` with `mode="create"`, target path, and markdown content including YAML frontmatter:
   ```json
   {
     "path": "decisions/adr-002-tantivy-bm25.md",
     "mode": "create",
     "content": "---\ntitle: \"ADR 002: Tantivy Inverted Index for BM25 Retrieval\"\nstatus: accepted\ndate: 2026-08-30\ntemplate: decision_record\ntags:\n  - architecture\n  - search\n  - bm25\n---\n\n# ADR 002: Tantivy Inverted Index\n\n## Context\n...\n\n## Decision\n...\n\n## Consequences\n..."
   }
   ```
4. **Validate Immediate Conformance**:
   Call `validate` on the newly created path to ensure zero schema errors:
   ```json
   {
     "path": "decisions/adr-002-tantivy-bm25.md"
   }
   ```

---

## 2. Updating and Moving Notes

When modifying existing documentation:

1. **Read Current Content or Frontmatter**:
   Call `read_file` (with line slicing if large) or inspect via `list_notes` to see existing frontmatter and structure.
2. **Apply Content Updates**:
   Call `write_note` with mode (`overwrite`, `append`, or `prepend`):
   ```json
   {
     "path": "decisions/adr-002-tantivy-bm25.md",
     "mode": "append",
     "content": "\n\n## Implementation Notes\nValidated with sub-2ms query latency."
   }
   ```
3. **Move / Rename Notes**:
   Call `move_note` to rename notes while automatically rewriting inbound wikilinks across other notes:
   ```json
   {
     "from": "decisions/adr-002-tantivy-bm25.md",
     "to": "decisions/adr-002-tantivy-index.md"
   }
   ```
4. **Re-Validate**:
   Call `validate` to confirm that changes satisfy template constraints:
   ```json
   {
     "path": "decisions/adr-002-tantivy-index.md"
   }
   ```

---

## 3. Vault-Wide Structural Audits

To audit the health and consistency of the entire knowledge base:

1. **Validate All Notes in Corpus**:
   Call `validate` with no `path` argument to perform a full corpus audit (missing required frontmatter, invalid enum values, broken links, or empty sections):
   ```json
   {}
   ```
2. **Validate Tag & Category Taxonomy**:
   Call `validate` with `check_taxonomy=true` to identify orphan tags, inconsistent casing, or misspelled categories:
   ```json
   {
     "check_taxonomy": true
   }
   ```
