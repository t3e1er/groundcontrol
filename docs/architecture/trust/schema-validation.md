---
title: "Schema Validation & Taxonomy Enforcement"
description: "How groundcontrol validates note templates, catches broken wikilinks, and maintains corpus health."
category: "trust"
status: "active"
tags: ["schema", "validation", "templates", "taxonomy", "linting", "corpus-health"]
related:
  - "[[docs/architecture/trust/index]]"
  - "[[docs/architecture/trust/knowledge-crystallization]]"
  - "[[docs/concepts/progressive-disclosure/tool-profiles]]"
---

# Schema Validation & Taxonomy Enforcement

To prevent documentation decay, `groundcontrol` includes native schema validation tools: `validate` and `list_templates`.

---

## 1. Formal Note Templates (`.templates/`)

Knowledge vaults define required frontmatter schemas in a `.templates/` directory (or via `groundcontrol.toml`). For example, an Architectural Decision Record template:

```markdown
---
title: "${title}"
category: "architecture"
status: "proposed"
tags: ["adr"]
related: []
derived_from: []
---

# ${title}

## Context
## Decision
## Consequences
```

When agents write notes via the `write_note` MCP tool, the template ensures all required metadata fields exist before writing to disk.

---

## 2. The `validate` Tool

Agents and CI pipelines validate individual notes or the entire corpus using `validate`:

```json
{
  "path": "docs/architecture/adr-018.md",
  "template": "adr",
  "check_taxonomy": true
}
```

### Validation Checks
1. **Frontmatter Integrity**: Ensures all mandatory keys are present and conform to expected YAML types.
2. **Taxonomy Verification**: Confirms that `#tags` belong to the registered taxonomy list.
3. **Wikilink Integrity**: Detects dead links (`[[broken/path]]`) pointing to non-existent notes or symbols.
4. **Lineage Tracing**: Validates that all URIs in `derived_from` resolve to active notes or source symbols.

---

## 3. Whole-Corpus Health Audits

By omitting `path`, `validate` runs a full corpus health audit, reporting orphaned notes, circular dependency warnings, and unindexed symbols.
