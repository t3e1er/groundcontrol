---
title: "Zero-Copy Storage & Binary Vector Layouts"
description: "Packed binary vectors, file offset mappings, and high-density SQLite catalog schemas."
category: "implementation"
status: "active"
tags: ["zero-copy", "binary-vectors", "sqlite", "storage-layout", "compression"]
related:
  - "[[docs/architecture/implementation/index]]"
  - "[[docs/architecture/trust/files-are-ground-truth]]"
---

# Zero-Copy Storage & Binary Vector Layouts

To eliminate memory duplication and guarantee sub-millisecond cold starts, `groundcontrol` employs zero-copy byte offsets and packed binary vector stores.

---

## 1. Packed Binary Vectors (`vectors.bin`)

Storing floating-point embedding vectors in JSON or text format inflates disk footprint by 400% and requires parsing overhead on every startup.

`groundcontrol` persists embeddings in a contiguous, aligned binary format:
```
[Header: 32 bytes]
  - Magic Bytes: 0x43 0x54 0x58 0x56 ("CTXV")
  - Schema Version: u32
  - Dimensions: u32 (768)
  - Vector Count: u64
[Vector Data: Count * 768 * 4 bytes]
  - Contiguous IEEE 754 float32 values aligned to 64-byte cache lines
```
During startup, vectors are memory-mapped directly into the HNSW search space with zero JSON deserialization.

---

## 2. Zero-Copy File Byte Offsets

Rather than duplicating source code strings inside SQLite or Tantivy:
* The SQLite metadata catalog records only `(file_path, byte_start, byte_end, line_start, line_end)`.
* When `get_snippet` or `read_file` is invoked, `groundcontrol` seeks directly to `byte_start` on disk, reading the exact slice without loading the entire file into memory.

---

## 3. SQLite Metadata Catalog Schema (`meta.db`)

The internal SQLite catalog uses strict B-tree indexes designed for sub-millisecond queries:
* `code_symbols`: Indexed by `name`, `scope_path`, and `kind`.
* `code_edges`: Indexed by `(source, edge_type)` and `(target, edge_type)` for bidirectional Cypher-Lite joins.
* `notes`: Indexed by `path`, `category`, and `mtime`.
* `tags`: Indexed by `tag` and `node_path`.
