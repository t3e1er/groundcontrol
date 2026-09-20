# `ctxvault` Examples & Starter Pack

This directory contains turnkey examples, agent steering configurations, workflow skills, multi-agent swarm blueprints, and a pre-configured starter knowledge vault for `ctxvault`.

---

## Directory Contents

| Directory | Description | Primary Use Case |
|---|---|---|
| [`steering/`](steering/) | System prompts and rules for AI assistants (`.cursorrules`, Claude Desktop, Windsurf, Antigravity, generic LLMs) | Drop into your editor or AI configuration to immediately teach your agent how to use `ctxvault` tools efficiently. |
| [`skills/`](skills/) | Production-ready `SKILL.md` runbooks for search, curation, crystallization, and ops | Copy into `.agents/skills/` to enable on-demand skill execution in AI IDEs. |
| [`agents/`](agents/) | Role definitions and swarm orchestration blueprints (Scout, Reader, Writer, Crystallizer) | Scaffold multi-agent pipelines for research, automated ADR creation, and vault refactoring. |
| [`starter-vault/`](starter-vault/) | Ready-to-index markdown knowledge base with `ctxvault.toml`, `.templates/`, and interlinked sample notes | Test or initialize a new project knowledge base with zero friction. |

---

## 1. Quick Start: Pointing `ctxvault` at the Starter Vault

You can start `ctxvault` against the included starter vault immediately:

```bash
# From repository root
ctxvault --corpus examples/starter-vault --sync
```

Or test a hybrid query directly using CLI client mode:

```bash
ctxvault --mode client --call search --query "How does hybrid retrieval work?" --args '{"mode":"hybrid","snippets":3}'
```

---

## 2. Setting Up in Your Editor

### Antigravity & Gemini IDE
Copy [`steering/ctxvault-rules.md`](steering/ctxvault-rules.md) to `.agents/rules/ctxvault-rules.md`, and copy the skill folders in [`skills/`](skills/) to `.agents/skills/`.

### Cursor IDE
Copy the snippet from [`steering/cursorrules.md`](steering/cursorrules.md) into your project's `.cursorrules` file.

### Claude Desktop
Add [`steering/claude-system-prompt.md`](steering/claude-system-prompt.md) to your Project Instructions or Custom Instructions.

### Multi-Agent Swarms
Consult [`agents/swarm_orchestration.md`](agents/swarm_orchestration.md) for complete message contracts and handoff schemas between Scout, Reader, Writer, and Crystallizer agents.
