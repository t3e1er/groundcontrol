---
title: "RFC: Zero-Copy File-Offset Architecture & Binary Vector Indexing"
category: "code-architecture"
status: "implemented"
tags: ["rfc", "storage", "vector", "tantivy", "sqlite", "zero-copy", "performance"]
related:
  - "[[docs/index]]"
  - "[[docs/roadmap/coderoadmap]]"
  - "[[docs/architecture/adr/adr-008-anchor-embedding-paradigm]]"
  - "[[docs/architecture/implementation/zero-copy-storage]]"
  - "[[docs/concepts/search/hybrid-retrieval-theory]]"
---

# RFC: Zero-Copy File-Offset Architecture & Binary Vector Indexing

**Status**: Implemented  
**Scope**: `groundcontrol-common`, `groundcontrol-core`, `groundcontrol-mcp`, `groundcontrol-cli`  
**Date**: September 2026  
**Related Documents**: [[docs/roadmap/coderoadmap]], [[docs/architecture/adr/adr-008-anchor-embedding-paradigm]], [[docs/architecture/implementation/zero-copy-storage]]

---

## 1. Executive Summary & Empirical Problem Statement

During the 177,000-file multi-corpus benchmark (Kubernetes, Rust, TypeScript), empirical disk telemetry at 14,644 indexed files revealed an on-disk index size of **2.15 GB** against ~**295 MB** of parsed source code—an expansion factor of **~7.2x**.

```
Active .index/ Directory Breakdown (14,644 Files Indexed):
Total: 2,146.4 MB (2.15 GB)
├── meta.db (SQLite Catalog):       1,363 MB  (63.5%)  █████████████
├── vectors.json (HNSW Store):        555 MB  (25.8%)  █████
├── tantivy/ (BM25 Postings):         190 MB   (8.8%)  ██
└── graph.bin (Petgraph Postcard):     39 MB   (1.8%)  ▌
```

### The Three Root Causes of Bloat

1. **Triplicate Source Text Duplication**:
   The engine creates and persists three separate copies of every source line on disk:
   - *Copy 1*: Authoritative raw source files (`src/**/*.rs`, `pkg/**/*.go`, etc.).
   - *Copy 2*: SQLite `chunks` table stores raw uncompressed text (`text TEXT NOT NULL`), occupying ~820 MB of `meta.db`.
   - *Copy 3*: Tantivy indexes and stores raw text inside compressed LZ4 `.store` segment files (`builder.add_text_field("body", TEXT | STORED)`), occupying ~120 MB of `tantivy/`.
2. **ASCII JSON Float Serialization Bloat**:
   `vectors.json` stores 768-dimensional `f32` vectors as human-readable JSON float strings (`[0.0234125, -0.0123981, ...]`). Each vector requires ~6.8 KB in ASCII, compared to exactly $768 \times 4 = 3,072$ bytes in raw binary IEEE 754 representation. This incurs a **~2.2x serialization penalty** (555 MB for 81,047 vectors vs. 249 MB in raw binary) and forces multi-second CPU parsing overhead on server cold starts.
3. **Inversion of Principle 1 (Files as Authoritative Ground Truth)**:
   Non-Negotiable Invariant #1 dictates: *"Markdown/source is authoritative ground truth: Files on disk are king. All indices are derived, disposable, and 100% rebuildable."* Storing redundant text caches inside derived indices violates this architectural invariant and inflates disk footprints.

---

## 2. Proposed Architecture: Zero-Copy File-Offset Subsystem

This RFC replaces redundant text stores with a **Zero-Copy File-Offset Architecture** and replaces `vectors.json` with a memory-mappable binary layout `vectors.bin`.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Zero-Copy Storage Topology                         │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   Authoritative Working Directory (Local Disk)                              │
│   └── pkg/kubelet/kubelet.go [Cached in OS Kernel Page Cache]                │
│             ▲                                  ▲                            │
│             │ Direct Byte Slice Read           │ Zero-Copy mmap Read        │
│             │ (start_byte..end_byte)           │ (<50µs latency)            │
│             │                                  │                            │
│   ┌─────────┴───────────────┐      ┌───────────┴───────────────┐            │
│   │   SQLite (meta.db)      │      │    Tantivy (BM25)         │            │
│   │   chunks table:         │      │    field_body: TEXT only  │            │
│   │   - file_path           │      │    - Tokenizes words      │            │
│   │   - chunk_index         │      │    - Builds inverted post.│            │
│   │   - start_byte          │      │    - NO STORED raw text   │            │
│   │   - end_byte            │      │    - Zero bytes in .store │            │
│   │   - start_line/end_line │      └───────────────────────────┘            │
│   │   - [NO TEXT COLUMN]    │                                               │
│   └─────────────────────────┘                                               │
│                                                                             │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │   Vector Index (vectors.bin)                                        │   │
│   │   Raw IEEE 754 f32 binary array + header (safetensors / bytemuck)   │   │
│   │   - 768 dims * 4 bytes = 3,072 bytes per vector                     │   │
│   │   - Zero ASCII parse overhead; direct mmap or bulk read             │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed Technical Specification

### 3.1 SQLite Schema Modernization (`meta.db`)

The `chunks` table schema is modified to drop the redundant `text` column:

```sql
-- Current Schema (Bloated with text duplicate):
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    text TEXT NOT NULL  -- REMOVED
);

-- Proposed Zero-Copy Schema:
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL
);
CREATE INDEX idx_chunks_file_chunk ON chunks(file_path, chunk_index);
```

**Savings**: Eliminates ~820 MB of duplicate text strings across 14,600 files in SQLite.

---

### 3.2 Tantivy Schema Tuning (`crates/groundcontrol-core/src/index/mod.rs`)

Tantivy's `field_body` is stripped of the `STORED` flag:

```rust
// File: crates/groundcontrol-core/src/index/mod.rs

fn build_schema() -> (Schema, Field, Field, Field, Field, Field, Field) {
    let mut builder = Schema::builder();
    let field_path = builder.add_text_field("path", STRING | STORED);
    let field_chunk_index = builder.add_text_field("chunk_index", STORED);
    let field_title = builder.add_text_field("title", TEXT | STORED);
    
    // Body is tokenized and indexed for Okapi BM25 scoring,
    // but NOT duplicated into Tantivy's compressed .store file:
    let field_body = builder.add_text_field("body", TEXT); 
    
    let field_tags = builder.add_text_field("tags", TEXT | STORED);
    let field_modality = builder.add_text_field("modality", STRING | STORED);
    let schema = builder.build();
    (schema, field_path, field_chunk_index, field_title, field_body, field_tags, field_modality)
}
```

**Why this works**:
- Okapi BM25 score calculation depends strictly on the inverted postings (`.term`, `.idx`, `.pos`), which record term frequencies and document lengths.
- Storing the raw body text in Tantivy was only utilized to reconstruct snippet text. By retrieving snippets via file byte offsets directly from the authoritative disk files, the `.store` file overhead is avoided entirely.
- **Savings**: Shrinks Tantivy `.store` files by **~60%** (~110 MB on Kubernetes 14k files).

---

### 3.3 Binary Vector Format (`vectors.bin`)

Replace `vectors.json` with a packed binary format (`vectors.bin`):

```
┌────────────────────────────────────────────────────────────────────────┐
│                        vectors.bin Binary Format                       │
├─────────────────┬───────────┬──────────────┬───────────────────────────┤
│ Magic (4B)      │ "CTXV"    │ 0x43545856   │ Magic signature           │
│ Version (2B)    │ u16       │ 1            │ Layout format version     │
│ Dimensions (2B) │ u16       │ 768          │ Embedding dimension       │
│ Vector Count(4B)│ u32       │ N            │ Number of vectors stored  │
│ Reserved (20B)  │ [u8; 20]  │ 0x00...      │ Alignment & future flags  │
├─────────────────┴───────────┴──────────────┴───────────────────────────┤
│ Payload Section: Raw Contiguous Array of [f32; 768]                    │
│ Total bytes = N * 768 * 4 bytes                                        │
├────────────────────────────────────────────────────────────────────────┤
│ Metadata Section (Postcard / MessagePack at EOF):                      │
│ Serialized map of ID -> { doc_path: String, chunk_index: usize, ... }  │
└────────────────────────────────────────────────────────────────────────┘
```

#### In-Engine Serialization Implementation
```rust
impl VectorIndex {
    pub fn save_binary(&self, path: &Path) -> Result<()> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(b"CTXV")?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&(self.dimensions as u16).to_le_bytes())?;
        file.write_all(&(self.vectors.len() as u32).to_le_bytes())?;
        file.write_all(&[0u8; 20])?; // Reserved padding
        
        for (_id, vec) in &self.vectors {
            let bytes = bytemuck::cast_slice(vec);
            file.write_all(bytes)?;
        }
        
        // Serialize metadata cleanly via postcard
        let meta_bytes = postcard::to_allocvec(&self.meta)?;
        file.write_all(&(meta_bytes.len() as u64).to_le_bytes())?;
        file.write_all(&meta_bytes)?;
        Ok(())
    }
}
```

**Savings**:
- 81,047 vectors $\times$ 768 floats $\times$ 4 bytes = **249.0 MB**.
- Shrinks vector storage from **554.6 MB down to 249.0 MB** (**55.1% reduction**).
- Deserialization speed increases from **~4.8 seconds (serde_json)** to **~18 milliseconds (direct binary read / mmap)**.

---

### 3.4 Direct File-Slice Snippet Retrieval (`Engine::fetch_chunk_text`)

When tools (`search`, `get_snippet`, `read_file`) need to render a snippet:

```rust
impl Engine {
    /// Retrieve chunk text directly from disk using byte-range coordinates.
    pub fn fetch_chunk_text(&self, rel_path: &str, start_byte: usize, end_byte: usize) -> Result<String> {
        let full_path = self.corpus_root.join(rel_path);
        let mut file = File::open(&full_path)
            .map_err(|e| Error::IO(format!("failed to open authoritative source {}: {e}", full_path.display())))?;
        
        file.seek(SeekFrom::Start(start_byte as u64))?;
        let len = end_byte.saturating_sub(start_byte);
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        
        String::from_utf8(buf).map_err(|e| Error::Encoding(e.to_string()))
    }
}
```

**Latency Impact**:
- Because the files in the corpus are active project files, the host OS kernel page cache maintains them in physical RAM.
- Byte-range slice reads execute in **10–40 microseconds**, faster than decompressing LZ4 blocks from Tantivy or decoding JSON from SQLite.

---

## 4. Empirical Impact & Comparative Projections

Based on the 14,644 indexed Kubernetes files:

| Subsystem | Current Footprint (14.6k files) | Zero-Copy + Binary Layout | Reduction |
|---|---|---|---|
| **SQLite Catalog (`meta.db`)** | 1,363.2 MB | **~350.0 MB** | **-74.3%** |
| **Vector Store (`vectors.bin`)** | 554.6 MB | **249.0 MB** | **-55.1%** |
| **BM25 Index (`tantivy/`)** | 189.6 MB | **~78.0 MB** | **-58.8%** |
| **Petgraph Graph (`graph.bin`)**| 39.1 MB | **39.1 MB** | **0.0%** (already binary) |
| **Total Index Footprint** | **2,146.5 MB (2.15 GB)** | **~716.1 MB (0.70 GB)** | **-66.6% (3x reduction)** |
| **Expansion vs. Raw Source** | **~7.2x** | **~2.4x** | **Substantially leaner** |
| **Full Kubernetes (37k files)** | Projected: **~5.5 GB** | Projected: **~1.8 GB** | Saves **~3.7 GB** |

---

## 5. Phased Implementation Plan

### Phase 1: Binary Vector Serialization (`vectors.bin`)
1. Implement `VectorIndex::save_binary` and `VectorIndex::load_binary` using `bytemuck` and `postcard`.
2. Update `engine.rs` to write and read `vectors.bin`.
3. Eliminate `serde_json` serialization paths for vector persistence (greenfield policy: no backwards compatibility shims).

### Phase 2: Tantivy `body` Unstored Field Tuning
1. Change `builder.add_text_field("body", TEXT | STORED)` to `builder.add_text_field("body", TEXT)` in `crates/groundcontrol-core/src/index/mod.rs`.
2. Update search snippet generation to query byte coordinates from SQLite and slice from disk.

### Phase 3: SQLite `chunks` Schema Modernization
1. Update SQLite migration in `crates/groundcontrol-core/src/persistence/` to omit the `text` column in `chunks`.
2. Update `ingest_parsed_record` and `Store::insert_chunks` to insert `start_byte`, `end_byte`, `start_line`, `end_line` without string allocations.
3. Wire `Engine::fetch_chunk_text` across all MCP retrieval tools (`search`, `get_snippet`).

---

## 6. Greenfield Discipline & Invariant Audit

- **Invariant 1 (Files are Authoritative)**: Enforced. The indices no longer cache duplicate copies of file text.
- **Invariant 2 (Pure Rust & Safety)**: Preserved. All byte-slice operations and bytemuck casts run under `#![forbid(unsafe_code)]`.
- **Greenfield Policy (No Backwards Compatibility)**: Indices are 100% disposable and rebuildable. No migration shims or fallback JSON loaders are introduced; the format replaces `vectors.json` outright.
