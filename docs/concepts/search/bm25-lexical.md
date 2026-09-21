---
title: "Tantivy Okapi BM25 Lexical Search"
description: "High-performance full-text search, symbol tokenization, and sub-2ms lexical retrieval in pure Rust."
category: "search"
status: "active"
tags: ["tantivy", "bm25", "lexical", "tokenization", "sub-millisecond"]
related:
  - "[[docs/concepts/search/index]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
  - "[[docs/architecture/implementation/hexagonal-architecture]]"
---

# Tantivy Okapi BM25 Lexical Search

For exact keyword queries, verbatim function names, compiler errors, and identifier searches, `groundcontrol` embeds **Tantivy**—the leading 100% pure Rust search engine library.

* **Source Implementation**: [`crates/groundcontrol-core/src/index/tantivy.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/index/tantivy.rs)
* **TextIndex Port Trait**: [`crates/groundcontrol-common/src/ports.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-common/src/ports.rs)

---

## Technical Features

1. **Sub-2ms Latency**: Tantivy utilizes memory-mapped inverted indices (`mmap`) and SIMD-accelerated bit-packing, allowing `groundcontrol` to achieve p50 query latencies under 2.2ms across hundreds of thousands of symbols.
2. **Code-Aware Tokenization**: Standard natural-language tokenizers split on underscores or drop casing, which destroys code identifiers like `get_user_by_id` or `HTTPRequest`. `groundcontrol` utilizes custom code tokenizers that index:
   * CamelCase boundaries (`HttpRequest` $\to$ `Http`, `Request`, `HttpRequest`)
   * snake_case components (`parse_ast_node` $\to$ `parse`, `ast`, `node`, `parse_ast_node`)
   * Scope paths (`groundcontrol_core::engine::Engine`)
3. **Okapi BM25 Scoring**:
   $$Score(D, Q) = \sum_{i=1}^{N} IDF(q_i) \cdot \frac{f(q_i, D) \cdot (k_1 + 1)}{f(q_i, D) + k_1 \cdot \left(1 - b + b \cdot \frac{|D|}{\text{avgdl}}\right)}$$
   Configured with $k_1 = 1.2$ and $b = 0.75$.

---

## Dedicated BM25 Mode

When an agent needs an exact identifier search without semantic vector fuzzy matching:
```json
{
  "query": "NewCorpusManager",
  "mode": "bm25",
  "snippets": 3
}
```
This bypasses vector encoding, returning top lexical matches in under 2ms.
