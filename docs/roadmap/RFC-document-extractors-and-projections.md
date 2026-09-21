---
title: "RFC: Pluggable Document Extractors, Derived Text Projections & Modality Disambiguation"
description: "Architectural specification for ingesting Word (.docx), PDF (.pdf), and HTML (.html) documents in 100% pure Rust via Derived Text Projections and deterministic corpus modality disambiguation."
category: "roadmap"
status: "accepted"
tags: ["rfc", "documents", "docx", "pdf", "html", "extractors", "projections", "zero-copy", "modality"]
related:
  - "[[docs/index]]"
  - "[[docs/roadmap/coderoadmap]]"
  - "[[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]]"
  - "[[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]]"
  - "[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]"
  - "[[docs/concepts/progressive-disclosure/three-tier-context-pipeline]]"
---

# RFC: Pluggable Document Extractors, Derived Text Projections & Modality Disambiguation

**Status**: Accepted (Implemented)  
**Scope**: `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`  
**Date**: September 2026  
**Target Version**: `0.3.0`+ (Sequenced directly following [[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]])  
**Related Documents**: [[docs/roadmap/coderoadmap]], [[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]], [[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]], [[docs/concepts/progressive-disclosure/three-tier-context-pipeline]]

---

## 1. Executive Summary & Problem Statement

`groundcontrol` was engineered from first principles around two foundational invariants:
1. **Non-Negotiable Invariant #1 (Markdown/Source as Authoritative Ground Truth)**: Files on disk are king. All derived indices (Tantivy BM25, HNSW vectors, SQLite metadata catalog, Petgraph) are disposable, transient, and 100% rebuildable from disk.
2. **Zero-Copy File-Offset Architecture** ([[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]]): Chunks in SQLite store zero redundant source text; instead, [`fetch_chunk_text`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/engine.rs) and [`read_single_file`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-mcp/src/tools/mod.rs) read exact byte-range slices (`start_byte..end_byte`) and line ranges (`start_line..end_line`) directly from the authoritative source files cached in the OS kernel page cache.

While this zero-copy model excels for plain UTF-8 text files (Markdown notes and polyglot source code), expanding `groundcontrol` to support **Microsoft Word (`.docx`)**, **Portable Document Format (`.pdf`)**, and **HyperText Markup Language (`.html`)** introduces three fundamental architectural tensions:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                 The Trilemma of Rich Document Ingestion                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. Container Oodles vs Zero-Copy Offsets:                                  │
│     Word (.docx) is a zipped OpenXML archive; PDF (.pdf) is a 2D binary     │
│     stream graph. Slicing raw bytes from disk yields compressed deflate     │
│     bytes or PDF stream operators (BT ... ET), not human-readable text.     │
│                                                                             │
│  2. Dual-Nature Disambiguation (The HTML Dilemma):                          │
│     In a React/Vue/Go codebase, .html files are UI templates and component  │
│     code. In documentation vaults, .html files are rich articles with       │
│     headings and prose. Ingestion must not conflate UI code with docs.      │
│                                                                             │
│  3. Strictly Read-Only vs Canonical Knowledge Crystallization:             │
│     groundcontrol must never attempt to author binary Word packages or generate  │
│     PDF vector streams. Markdown remains the sole writable format for       │
│     Principle 3 knowledge crystallization.                                  │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

This RFC resolves this trilemma by introducing:
1. **A Deterministic Corpus Modality Disambiguation Subsystem**: Distinguishing code UI templates from documentation articles using configurable directory rules, corpus roles, and AST/structural heuristics.
2. **Derived Text Projections (DTP)**: Storing disposable, deterministic, line-numbered text projections under `.index/projections/<path>.txt`. The raw `.docx`, `.pdf`, or `.html` file on disk remains the sole authoritative source of truth, while `get_snippet` and `read_file` maintain sub-millisecond zero-copy slicing over the projection.
3. **Pure-Rust Ingestion Adapters**: Implementing extraction for Word, PDF, and HTML with `unsafe_code = "forbid"` and zero external C/C++ runtime dependencies (no Poppler, libxml2, or MuPDF).

---

## 2. Corpus Modality Disambiguation: Code vs. Docs

### 2.1 The Current Baseline
Currently in [`crates/groundcontrol-core/src/parser/code/languages.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/code/languages.rs), `Html` (`.html`, `.htm`) is supported as a code template language. In the discovery walker [`crates/groundcontrol-core/src/engine.rs`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/engine.rs), classification is now mediated dynamically via [`FileClassifier`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/index/classifier.rs).

This causes immediate failures in mixed repositories:
- **False Code Classification**: Sphinx/Doxygen/Confluence HTML documentation exports are treated as code, missing heading-based chunk hierarchy, document title metadata, and being filtered out when querying `search(modality="docs")`.
- **False Document Classification**: If HTML is globally treated as documentation, `index.html` and Angular/Vue templates pollute document searches with raw markup.
- **Asset Contamination**: `.pdf` and `.docx` files in code repositories often exist as binary test fixtures (`tests/fixtures/sample.pdf`) or design assets (`assets/branding.pdf`), which should not be indexed unless explicitly designated as documentation.

### 2.2 Classification Hierarchy & Configuration
We introduce a 3-tier classification hierarchy implemented in `groundcontrol-core::index::classifier`:

```mermaid
flowchart TD
    File[Encountered File Path on Disk] --> Exclude{Excluded by .gitignore / exclude_matcher?}
    Exclude -- Yes --> Skip[Ignore File]
    Exclude -- No --> Ext{File Extension}
    
    Ext -- .md --> Docs[Modality: Docs (Markdown Parser)]
    Ext -- .rs, .ts, .py, .go, etc. --> Code[Modality: Code (Tree-Sitter AST)]
    Ext -- .docx, .pdf, .html --> Disambiguate{Corpus Role & Path Matcher}
    
    Disambiguate -- Matches doc_patterns --> DocPipeline[Modality: Docs (Document Extractor)]
    Disambiguate -- Matches code_patterns --> CodePipeline[HTML: Code AST / PDF: Skip]
    Disambiguate -- No pattern match --> CorpusDefault{CorpusType}
    
    CorpusDefault -- CorpusType::DocVault --> DocPipeline
    CorpusDefault -- CorpusType::CodeRepo (Default) --> CodePipeline
    CorpusDefault -- CorpusType::Mixed --> Heuristic{Content Heuristic}
    
    Heuristic -- Article Prose --> DocPipeline
    Heuristic -- UI Template / Asset --> CodePipeline
```

#### Configuration Schema (`groundcontrol.toml` / `CorpusConfig`)
```toml
[corpus]
name = "enterprise-suite"
type = "mixed" # Options: "code_repo" (default), "doc_vault", "mixed"

# Explicit directory patterns governing modality promotion
doc_patterns = [
    "docs/**",
    "documentation/**",
    "wiki/**",
    "specs/**",
    "rfcs/**",
    "man/**",
    "site/content/**"
]

code_patterns = [
    "src/**",
    "app/**",
    "templates/**",
    "views/**",
    "components/**",
    "public/**",
    "tests/fixtures/**"
]
```

#### Content Heuristic for HTML (Fallback in Mixed Corpora)
When an `.html` file does not match an explicit pattern in a `mixed` corpus, the engine inspects the first 4 KB:
1. **Documentation Indicators**: Contains `<article>`, `<main>`, semantic `<section>`, or markdown-style heading chains (`<h1>` followed by multi-sentence `<p>`), with a text-to-tag ratio $> 0.45$.
2. **Code Indicators**: Contains `<template>`, `<router-outlet>`, JSX/directives (`v-if`, `*ngIf`, `th:text`), high density of `<script>`/`<style>`, or text-to-tag ratio $< 0.20$.

---

## 3. Derived Text Projections (DTP) Subsystem

### 3.1 Resolving the Disk-as-Source-of-Truth Invariant
Non-Negotiable Invariant #1 dictates that disk is ground truth and all indices are disposable. Storing raw extracted text in SQLite would recreate the database bloat solved by [[docs/roadmap/RFC-zero-copy-file-offsets-and-binary-vectors]].

The **Derived Text Projection (DTP)** architecture places deterministic text projections in `.index/projections/`:

```
Working Repository (Authoritative Ground Truth)
├── specs/architecture.docx
├── papers/attention.pdf
└── docs/manual.html
         │
         │  Deterministic Pure-Rust Extraction (Index / Sync Time)
         ▼
.index/projections/ (Disposable, Derived Text Cache)
├── specs/architecture.docx.txt   <-- Normalized UTF-8 text (Markdown formatting & synthetic lines)
├── papers/attention.pdf.txt      <-- Page-structured text with "<!-- Page N -->" line anchors
└── docs/manual.html.txt          <-- Clean article markdown (boilerplate stripped)
```

### 3.2 Storage & Lifecycle Invariants
1. **Transient & 100% Rebuildable**: If the `.index/` directory is deleted, running `groundcontrol index` or `sync_corpus` re-extracts the authoritative files on disk and regenerates `.index/projections/` deterministically.
2. **Delta Invalidation**: The Blake3 `content_hash` stored in the SQLite `files` table is computed over the **raw binary file on disk** (`architecture.docx`). If the binary file's hash or `modified_at` changes, the projection file is regenerated.
3. **Zero-Copy Byte Offsets**:
   - For native Markdown and code: `start_byte..end_byte` in the SQLite `chunks` table points to byte offsets in the authoritative file on disk.
   - For projected formats (`.docx`, `.pdf`, doc-mode `.html`): `start_byte..end_byte` points to byte offsets in `.index/projections/<path>.txt`.
   - The SQLite `files` table is extended with an enumerated format:
     ```sql
     ALTER TABLE files ADD COLUMN format TEXT NOT NULL DEFAULT 'source';
     -- Values: 'source' (Markdown/Code), 'docx', 'pdf', 'html_doc'
     ```

---

## 4. Pure-Rust Ingestion Adapters (`DocumentExtractor` Port)

Following groundcontrol's strict Hexagonal Architecture, extractors are defined via a port trait in `groundcontrol-common::ports::DocumentExtractor`:

```rust
/// Domain representation of an extracted document prior to indexing.
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// Document title extracted from metadata or primary heading.
    pub title: Option<String>,
    /// Extracted document metadata / properties (author, date, etc.).
    pub metadata: HashMap<String, String>,
    /// Normalized UTF-8 text with structured heading lines and paragraph breaks.
    pub normalized_text: String,
    /// Extracted cross-reference links (hyperlinks, anchors, references).
    pub outbound_links: Vec<DocumentLink>,
}

/// Port for deterministic document extraction.
pub trait DocumentExtractor: Send + Sync {
    /// Returns true if this extractor handles the given file format.
    fn can_extract(&self, path: &Path) -> bool;
    
    /// Extract structured text and links from raw file bytes.
    fn extract(&self, path: &Path, bytes: &[u8]) -> Result<ExtractedDocument>;
}
```

### 4.1 Word Extractor (`DocxExtractor`)
- **Underlying Technology**: Pure Rust using `zip` (for uncompressing OpenXML containers) and `quick-xml` (for streaming, allocation-free XML parsing).
- **Zero C-Runtime**: `unsafe_code = "forbid"`, 100% pure Rust.
- **Extraction Protocol**:
  1. Inspect `word/document.xml`.
  2. Parse paragraphs (`<w:p>`). Style ids matching `Heading1` through `Heading6` map directly to Markdown `#` through `######`.
  3. Extract text nodes (`<w:t>`), tab stops (`<w:tab>` $\to$ `\t`), and line breaks (`<w:br>` $\to$ `\n`).
  4. Parse table structures (`<w:tbl>`) into GitHub Flavored Markdown tables.
  5. Extract hyperlinks from `word/_rels/document.xml.rels` into `outbound_links`.
- **Performance**: Typical 500 KB `.docx` extracts in **<1.8ms**.

### 4.2 PDF Extractor (`PdfExtractor`)
- **Underlying Technology**: Pure Rust using `lopdf` (cross-reference table and content stream parsing) and `pdf_extract`.
- **Zero C-Runtime**: Eliminates any dependency on Poppler, MuPDF, or Cairo.
- **Explicit Scoping Boundary**:
  - **Supported**: Text-based and vector PDFs with embedded font tables (Type 1, TrueType, Type 0 / CIDFont, standard 14 fonts).
  - **Excluded**: Scanned / bitmap-only PDFs requiring Optical Character Recognition (OCR). OCR requires multi-megabyte C neural dependencies (Tesseract / Leptonica) and incurs multi-second latency per page, violating sub-millisecond indexing invariants. Scanned PDFs are flagged with a `ValidationSeverity::Warning` (`"Scanned PDF contains no extractable text; OCR is not supported in zero-C runtime"`).
- **Line Structure & Page Anchoring**:
  - Emits page separator anchors: `<!-- Page N -->`.
  - Reconstructs reading order top-to-bottom and left-to-right from spatial coordinates.
  - Detects section headings by font size differentials relative to the document body baseline.

### 4.3 HTML Document Extractor (`HtmlDocExtractor`)
- **Underlying Technology**: Pure Rust using `tl` (SIMD-accelerated, high-throughput HTML parser) or `scraper`.
- **Boilerplate Stripping**: Automatically prunes non-content subtrees:
  - Tags: `<script>`, `<style>`, `<noscript>`, `<svg>`, `<canvas>`, `<iframe>`.
  - Structural Chrome: `<nav>`, `<header>`, `<footer>`, `<aside>`, `.navbar`, `.sidebar`, `.cookie-banner`.
- **Semantic Markdown Conversion**:
  - `<h1>`–`<h6>` $\to$ `#`–`######`.
  - `<p>` $\to$ double line-break paragraphs.
  - `<code>` / `<pre>` $\to$ inline code or fenced code blocks.
  - `<a>` $\to$ Markdown links `[text](url)` and outbound graph edges.
  - `<table>` $\to$ Markdown tables.

---

## 5. Progressive Disclosure & Reading Protocol (Tiers 1, 2, 3)

The existing 3-tier progressive disclosure contract is seamlessly preserved:

### 5.1 Tier 1: `search`
- Queries return partitioned hits across `docs` and `code`.
- Turn 1 snippets for `.docx`, `.pdf`, and `.html` display clean, human-readable text extracted from the projected chunks, complete with heading breadcrumbs (e.g., `specs/architecture.docx > Section 2.1 Storage Layer`).
- `graph_affordances` display incoming/outgoing link degrees extracted from document hyperlinks.

### 5.2 Tier 2: `get_snippet`
- Calling `get_snippet(path="specs/architecture.docx", chunk_index=3)`:
  - Fetches the exact heading-delimited chunk.
  - Reads directly from `.index/projections/specs/architecture.docx.txt` using the chunk's `start_byte..end_byte`.
  - Returns clean, bounded text in <1ms without re-parsing the docx container.

### 5.3 Tier 3: `read_file`
- Calling `read_file(path="papers/attention.pdf", start_line=1, end_line=50)`:
  - Detects `format == "pdf"`.
  - Opens `.index/projections/papers/attention.pdf.txt`.
  - Returns line-numbered prose with page anchors (`L1: <!-- Page 1 -->`, `L2: # Abstract`, etc.).
- **HTML Slicing Guarantee**:
  - When classified as *Docs*: `read_file` returns the clean, normalized Markdown projection lines.
  - When classified as *Code*: `read_file` returns the verbatim raw HTML source lines on disk.

### 5.4 Strict Read-Only Boundary
- **Mutation Tools**:
  - `write_note`: Strictly restricted to `.md` files. Passing a `.docx`, `.pdf`, or `.html` path returns an explicit error:
    ```
    ReadOnlyDocumentFormat: 'specs/architecture.docx' is a read-only document. 
    groundcontrol only authors native .md notes; edit source documents in their native authoring tools.
    ```
  - `move_note`: Renaming a `.docx` or `.pdf` file updates file records and projections, but skips internal wikilink rewriting.
  - `delete_note`: Deletes the authoritative binary file from disk and cascades removal across `.index/projections/`, SQLite, Tantivy, HNSW, and Petgraph.
- **Principle 3 Crystallization**:
  - Agents read `.docx`, `.pdf`, and `.html` documents for architectural intake.
  - When agents synthesize new knowledge, summarize decisions, or resolve bugs, they author permanent **Markdown notes** with full provenance:
    ```markdown
    ---
    title: "ADR-024: Distributed Index Partitioning"
    derived_from:
      - "specs/legacy-architecture.docx"
      - "papers/dynamo-partitioning.pdf"
    ---
    # ADR-024: Distributed Index Partitioning
    ...
    ```

---

## 6. Multimodal Graph & Retrieval Integration

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                 Petgraph Cross-Modal Document Topology                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   [Doc: specs/architecture.docx] ───(references)───► [Code: src/storage.rs] │
│                 │                                           ▲               │
│            (documents)                                  (defines)           │
│                 ▼                                           │               │
│      [Doc: docs/manual.html] ────(wikilink)────► [CodeSymbol: EngineStore]   │
│                 ▲                                                           │
│            (cites_doc)                                                      │
│                 │                                                           │
│      [Doc: papers/ann-search.pdf]                                           │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

1. **Tantivy Okapi BM25**: Extracted document titles and normalized chunk texts are tokenized with Okapi BM25 scoring. Document metadata (author, subject) is indexed as filterable facets.
2. **Dense Vector Embeddings (ONNX / DirectML)**:
   - Chunk titles and sections pass through [`classify_markdown_chunk`](file:///c:/dev/ctx/groundcontrol/crates/groundcontrol-core/src/parser/markdown.rs).
   - Document summaries (chunk 0) receive `ChunkEmbedPolicy::Anchor` status; tabular and bullet-list chunks are assigned `ChunkEmbedPolicy::GraphOnly`.
3. **Petgraph Graph Topology**:
   - Hyperlinks extracted from HTML `<a>` tags, Word document relationships, and PDF URI annotations are parsed. If a link resolves to a relative path within the corpus or a code symbol moniker, a directed `references` or `documents` edge is created in Petgraph.

---

## 7. Phased Engineering Plan & Roadmap Sequence

This specification is queued directly following [[docs/roadmap/RFC-sota-code-retrieval-and-semantic-bridging]]:

| Phase | Scope & Deliverables | Primary Crates |
|---|---|---|
| **Phase 1: Modality Disambiguation & Classifier** | Extend `CorpusConfig` with `CorpusType`, `doc_patterns`, `code_patterns`. Implement `FileClassifier` in `groundcontrol-core` and update `walk_dir_recursive` to dynamically route `.html`, `.pdf`, `.docx`. | `groundcontrol-common`, `groundcontrol-core` |
| **Phase 2: DTP Caching Subsystem & Port Trait** | Implement `DocumentExtractor` trait and `.index/projections/` caching manager. Extend SQLite schema with `files.format` and route `fetch_chunk_text` and `read_single_file` through DTP. | `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp` |
| **Phase 3: HTML Documentation Adapter** | Implement `HtmlDocExtractor` via `tl` / `scraper`. Add boilerplate filtering, semantic Markdown synthesis, and link extraction. | `groundcontrol-core` |
| **Phase 4: Word (.docx) Ingestion Adapter** | Implement `DocxExtractor` via `quick-xml` + `zip`. Add heading-style mapping, table synthesis, and OpenXML relationship link extraction. | `groundcontrol-core` |
| **Phase 5: PDF Ingestion Adapter** | Implement `PdfExtractor` via `lopdf`. Add page anchoring, reading-order reconstruction, font-size heading detection, and URI annotation link extraction. | `groundcontrol-core` |
| **Phase 6: Progressive Disclosure & MCP Hardening** | Verify 3-tier read experience (`search`, `get_snippet`, `read_file`) across all formats. Enforce strict `write_note` rejection for non-markdown formats. Add end-to-end integration tests. | `groundcontrol-mcp`, `groundcontrol-cli` |
