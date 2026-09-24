//! Tantivy schema definition and field handles.

use tantivy::schema::{Field, Schema, STORED, STRING, TEXT};

/// The collection of Tantivy field handles used by BM25Index.
#[derive(Clone, Copy)]
pub struct BM25Fields {
    /// Document file path.
    pub path: Field,
    /// Chunk index within document.
    pub chunk_index: Field,
    /// Document title or symbol name.
    pub title: Field,
    /// Text content for BM25 retrieval.
    pub body: Field,
    /// Metadata tags.
    pub tags: Field,
    /// Coarse modality tag ("code" or "docs").
    pub modality: Field,
}

/// Build the shared schema and field handles used by all BM25Index instances.
pub fn build_schema() -> (Schema, BM25Fields) {
    let mut builder = Schema::builder();
    let field_path = builder.add_text_field("path", STRING | STORED);
    let field_chunk_index = builder.add_text_field("chunk_index", STORED);
    let field_title = builder.add_text_field("title", TEXT | STORED);
    // Body is indexed for BM25 scoring but NOT stored, eliminating redundant text in .store files.
    let field_body = builder.add_text_field("body", TEXT);
    let field_tags = builder.add_text_field("tags", TEXT | STORED);
    // Coarse modality tag ("code"/"docs") for exact-match filtering.
    let field_modality = builder.add_text_field("modality", STRING | STORED);
    let schema = builder.build();
    let fields = BM25Fields {
        path: field_path,
        chunk_index: field_chunk_index,
        title: field_title,
        body: field_body,
        tags: field_tags,
        modality: field_modality,
    };
    (schema, fields)
}
