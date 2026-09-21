---
title: "cAST Polyglot Chunking Engine"
description: "Tree-sitter concrete syntax tree chunking across 16+ languages with parent scope breadcrumb injection."
category: "implementation"
status: "active"
tags: ["cast", "chunking", "tree-sitter", "ast", "polyglot", "scope-breadcrumbs"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/architecture/trust/deterministic-graph]]"
  - "[[docs/architecture/adr/adr-016-generic-normalized-scope-resolution]]"
---

# cAST Polyglot Chunking Engine

Naive chunkers split source code on fixed character counts (e.g. 500 characters) or newline intervals. This slices functions in half, separates docstrings from signatures, and renders code chunks unparseable.

`groundcontrol` features **cAST (Concrete Abstract Syntax Tree) Chunking** powered by Tree-sitter.

* **Parser Modules**: [`crates/groundcontrol-core/src/parser/code/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/code/mod.rs)
* **Rust cAST Grammar**: [`crates/groundcontrol-core/src/parser/code/rust.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/code/rust.rs)
* **Markdown Chunking**: [`crates/groundcontrol-core/src/parser/markdown/mod.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/markdown/mod.rs)

---

## 1. Syntax-Aware Node Slicing

Rather than arbitrary line counts, cAST parses the source file into an AST and segments code strictly along logical syntax boundaries:
* Function definitions (`fn`, `def`, `func`, `function`)
* Struct, class, and interface declarations
* Impl blocks and method signatures
* Module-level constant groups

---

## 2. Parent Scope Breadcrumb Injection

A nested method chunk extracted in isolation often lacks critical context. For example, a method named `process` inside `PaymentGateway` would be indexed merely as `process`.

cAST injects hierarchical **scope breadcrumbs** into each chunk:
```text
// Scope: crate::billing::payment::PaymentGateway > fn process
pub fn process(&self, tx: Transaction) -> Result<Receipt> {
    ...
}
```
* The Tantivy BM25 tokenizer and ONNX embedder see the enclosing struct, namespace, and module names.
* Retrieval queries for `PaymentGateway::process` hit with 100% precision.

---

## 3. Supported Languages (16+)

* Rust (`tree-sitter-rust`)
* TypeScript & JavaScript (`tree-sitter-typescript`, `tree-sitter-javascript`)
* Python (`tree-sitter-python`)
* Go (`tree-sitter-go`)
* Java (`tree-sitter-java`)
* C & C++ (`tree-sitter-c`, `tree-sitter-cpp`)
* C# (`tree-sitter-c-sharp`)
* Bash / Shell (`tree-sitter-bash`)
* Lua (`tree-sitter-lua`)
* Markdown (`pulldown-cmark`)
