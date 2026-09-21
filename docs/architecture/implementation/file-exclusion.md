---
title: "File Exclusion & Gitignore Pattern Engine"
description: "Explicit, single-file repository exclusion filtering using gitignore-compatible pattern matching."
category: "implementation"
status: "active"
tags: ["indexing", "discovery", "exclude", "gitignore", "groundcontrol-toml"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/architecture/implementation/hexagonal-architecture]]"
---

# File Exclusion & Gitignore Pattern Engine

During repository and knowledge base indexing, ingesting test suites, build outputs, mock data, or vendor packages leads to index bloat, wasted embedding compute, and polluted search results.

`groundcontrol` incorporates an explicit, deterministic file exclusion engine that filters files and directories at **discovery time** before any files are read, parsed, or embedded.

* **Configuration**: [`crates/groundcontrol-common/src/config.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/config.rs)
* **Matcher Engine**: [`crates/groundcontrol-core/src/index/exclude.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/index/exclude.rs)
* **Discovery Walker**: [`crates/groundcontrol-core/src/engine.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/engine.rs)
* **Continuous File Watcher**: [`crates/groundcontrol-core/src/watcher/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/watcher/mod.rs)

---

## 1. Filter Hierarchy & Precedence

Discovery evaluates paths through a strict, deterministic two-tier model:

1. **Non-Negatable Safety Core**:
   Directories that can never be crawled under any circumstances to prevent infinite recursion, catalog corruption, or thousands of external packages:
   - `.git`
   - `.index`
   - `node_modules`
2. **Explicit Repository Exclusions (`[exclude.patterns]`)**:
   Configured directly in `groundcontrol.toml`. By default, initialized repositories include:
   - **VCS & Tool metadata**: `.git`, `.svn`, `.hg`, `.index/`, `.fastembed_cache/`
   - **Dependencies**: `node_modules/`, `vendor/`, `Pods/`
   - **Build artifacts**: `target/`, `dist/`, `build/`, `out/`, `bin/`, `obj/`
   - **Virtualenvs & caches**: `.venv/`, `venv/`, `env/`, `__pycache__/`, `.cache/`, `.next/`, `.nuxt/`, `.turbo/`
   - **Test suites & fixtures**: `tests/`, `test/`, `__tests__/`, `fixtures/`, `testdata/`, `spec/`, `specs/`, `*.test.*`, `*.spec.*`, `*_test.go`, `*_test.py`
   - **Binaries & compiled objects**: `*.exe`, `*.dll`, `*.so`, `*.dylib`, `*.bin`, `*.wasm`, `*.pyc`, `*.o`, `*.a`, `*.db`, `*.sqlite`
   - **Archives**: `*.zip`, `*.tar`, `*.gz`, `*.bz2`, `*.xz`, `*.7z`

---

## 2. Configuration Schema (`groundcontrol.toml`)

All exclusions are declared in a single, authoritative place inside `<repo_root>/groundcontrol.toml`:

```toml
name = "my-project"
path = "."
index_mode = "full"

[exclude]
patterns = [
    "target/**",
    "dist/**",
    "build/**",
    "tests/**",
    "*.test.*",
    "!tests/e2e/**", # Un-ignore specific paths via gitignore negation syntax
]
```

### Automatic Gitignore Migration (`groundcontrol init`)

Rather than re-evaluating multi-layered `.gitignore` or `.groundcontrolignore` files dynamically on every disk traversal, `groundcontrol init` inspects your existing `.gitignore`, merges any active project exclusions with the standard defaults, and writes concrete patterns into `groundcontrol.toml`.

What you see in `groundcontrol.toml` is what gets excluded — single source of truth, zero hidden runtime probing.

### Negation (`!`) Support

Standard gitignore negation semantics are supported in `[exclude.patterns]`. For example, to index a specific test folder while keeping the rest of the test suite excluded:

```toml
[exclude]
patterns = [
    "tests/**",
    "!tests/e2e/**"
]
```

---

## 3. Subtree Pruning & Uniform Enforcement

- **Discovery Traversal**: `walk_dir_recursive` tests directory paths against `ExcludeMatcher::is_excluded(&path, true)` and prunes whole subtrees immediately, avoiding traversal into `node_modules`, `target`, or `tests`.
- **Live Watcher**: `CorpusWatcher` and `classify_event_with_matcher` apply the same matcher to incoming notify events so changes inside excluded directories or test files never trigger unnecessary delta re-indexing.
- **Wikilink Rewriting**: `walk_markdown_files_for_rewrite` honors the same matcher during note moves and renames.
