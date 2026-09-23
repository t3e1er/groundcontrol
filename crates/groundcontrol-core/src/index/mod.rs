//! Index management: orchestrates tantivy (BM25) and HNSW (vector) indices.

pub mod classifier;
pub mod exclude;
pub mod pipeline;

pub use classifier::{FileClassification, FileClassifier};

use std::path::Path;

use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::{BooleanQuery, Occur, QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, Schema, Value, STORED, STRING, TEXT},
    Index, IndexReader, IndexWriter, ReloadPolicy, Term,
};

use groundcontrol_common::{
    types::{Chunk, EntityKind, Modality, ScoreBreakdown, SearchResult},
    Error, Result,
};

/// Full-text BM25 search index backed by Tantivy.
pub struct BM25Index {
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    index_path: Option<std::path::PathBuf>,
    // Field handles
    field_path: Field,
    field_chunk_index: Field,
    field_title: Field,
    field_body: Field,
    field_tags: Field,
    field_modality: Field,
}

/// Scan the Tantivy index directory for stale lockfiles (`.tantivy-*.lock`)
/// and clean them up if no active process holds an advisory lock on them.
pub fn heal_stale_lockfiles(index_path: &Path) {
    if !index_path.exists() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(index_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with(".tantivy-") && file_name.ends_with(".lock") {
                    if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path)
                    {
                        use fs4::fs_std::FileExt;
                        if file.try_lock_exclusive().is_ok() {
                            drop(file);
                            if let Err(e) = std::fs::remove_file(&path) {
                                tracing::debug!(
                                    "Failed to remove stale lockfile {}: {}",
                                    path.display(),
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "Removed stale Tantivy lockfile: {}",
                                    path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

impl BM25Index {
    /// Build the shared schema used by all BM25Index instances.
    fn build_schema() -> (Schema, Field, Field, Field, Field, Field, Field) {
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
        (schema, field_path, field_chunk_index, field_title, field_body, field_tags, field_modality)
    }

    /// Open or create a Tantivy index at the given directory.
    pub fn open(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path).map_err(|e| Error::Index(e.to_string()))?;

        // Clean up any stale lockfiles from previously killed processes.
        heal_stale_lockfiles(index_path);

        let (
            schema,
            field_path,
            field_chunk_index,
            field_title,
            field_body,
            field_tags,
            field_modality,
        ) = Self::build_schema();

        let dir = MmapDirectory::open(index_path).map_err(|e| Error::Index(e.to_string()))?;

        let index = Index::open_or_create(dir, schema).map_err(|e| Error::Index(e.to_string()))?;

        // Don't acquire writer at open — only needed for mutations.
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e: tantivy::TantivyError| Error::Index(e.to_string()))?;

        Ok(Self {
            index,
            reader,
            writer: None,
            index_path: Some(index_path.to_path_buf()),
            field_path,
            field_chunk_index,
            field_title,
            field_body,
            field_tags,
            field_modality,
        })
    }

    /// Create an in-memory index (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let (
            schema,
            field_path,
            field_chunk_index,
            field_title,
            field_body,
            field_tags,
            field_modality,
        ) = Self::build_schema();

        let index = Index::create_in_ram(schema);

        // Don't acquire writer at open — only needed for mutations.
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e: tantivy::TantivyError| Error::Index(e.to_string()))?;

        Ok(Self {
            index,
            reader,
            writer: None,
            index_path: None,
            field_path,
            field_chunk_index,
            field_title,
            field_body,
            field_tags,
            field_modality,
        })
    }

    /// Lazily acquire the IndexWriter if not already held.
    /// This acquires an exclusive file lock on the index directory.
    fn ensure_writer(&mut self) -> Result<&mut IndexWriter> {
        if self.writer.is_none() {
            let writer = match self.index.writer(50_000_000) {
                Ok(w) => w,
                Err(e) => {
                    // Try healing stale lockfiles if we have an index path, then retry once.
                    if let Some(ref path) = self.index_path {
                        heal_stale_lockfiles(path);
                    }
                    self.index.writer(50_000_000).map_err(|retry_err| {
                        Error::Index(format!(
                            "Failed to acquire Lockfile: {} (retry also failed: {})",
                            e, retry_err
                        ))
                    })?
                }
            };
            self.writer = Some(writer);
        }
        Ok(self.writer.as_mut().unwrap())
    }

    /// Release the IndexWriter, dropping the exclusive file lock.
    /// Call this after commit to allow other processes to access the index.
    pub fn release_writer(&mut self) {
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }
    }

    /// Add all chunks for a document to the index.
    ///
    /// Each chunk becomes a separate tantivy document with the doc_path,
    /// chunk_index, and body text. Does NOT auto-commit.
    pub fn add_document(
        &mut self,
        doc_path: &str,
        title: Option<&str>,
        tags: &[String],
        chunks: &[Chunk],
    ) -> Result<()> {
        let tags_text = tags.join(" ");
        let title_text = title.unwrap_or("");

        // Copy field handles before borrowing self mutably for writer.
        let field_path = self.field_path;
        let field_chunk_index = self.field_chunk_index;
        let field_title = self.field_title;
        let field_body = self.field_body;
        let field_tags = self.field_tags;
        let field_modality = self.field_modality;

        let docs: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let modality_tag =
                    chunk.entity_kind.as_ref().map(EntityKind::modality_tag).unwrap_or("docs");
                doc!(
                    field_path => doc_path,
                    field_chunk_index => chunk.chunk_index.to_string(),
                    field_title => title_text,
                    field_body => chunk.text.as_str(),
                    field_tags => tags_text.as_str(),
                    field_modality => modality_tag,
                )
            })
            .collect();

        let writer = self.ensure_writer()?;
        let mut failed = false;
        for tantivy_doc in &docs {
            if let Err(e) = writer.add_document(tantivy_doc.clone()) {
                tracing::warn!("Tantivy add_document error: {e}. Re-acquiring index writer...");
                failed = true;
                break;
            }
        }

        if failed {
            self.release_writer();
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Some(ref path) = self.index_path {
                heal_stale_lockfiles(path);
            }
            let writer = self.ensure_writer()?;
            for tantivy_doc in docs {
                writer.add_document(tantivy_doc).map_err(|e| Error::Index(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Remove all indexed chunks for a given document path.
    ///
    /// Does NOT auto-commit.
    pub fn remove_document(&mut self, doc_path: &str) -> Result<()> {
        let field_path = self.field_path;
        let writer = self.ensure_writer()?;
        let term = Term::from_field_text(field_path, doc_path);
        let _ = writer.delete_term(term);
        Ok(())
    }

    /// Delete all documents from the BM25 index.
    pub fn clear(&mut self) -> Result<()> {
        let writer = self.ensure_writer()?;
        writer.delete_all_documents().map_err(|e| Error::Index(e.to_string()))?;
        Ok(())
    }

    /// Commit pending changes to disk.
    pub fn commit(&mut self) -> Result<()> {
        if let Some(ref mut writer) = self.writer {
            if let Err(e) = writer.commit() {
                tracing::warn!("Tantivy commit error: {e}. Re-acquiring index writer...");
                drop(self.writer.take());
                std::thread::sleep(std::time::Duration::from_millis(250));
                if let Some(ref path) = self.index_path {
                    heal_stale_lockfiles(path);
                }
                let writer = self.ensure_writer()?;
                writer
                    .commit()
                    .map_err(|e2| Error::Index(format!("Commit failed after retry: {e2}")))?;
            }
        }
        Ok(())
    }

    /// Search the BM25 index with a text query (no modality restriction).
    ///
    /// Thin wrapper over [`BM25Index::search_with_modality`] with
    /// [`Modality::Both`]. Returns ranked results with scores and snippets.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search_with_modality(query, limit, Modality::Both)
    }

    /// Search the BM25 index, restricting results to the requested [`Modality`].
    ///
    /// When `modality == Both`, this parses and runs the user query unchanged.
    /// Otherwise it wraps the parsed query in a boolean AND with an exact-match
    /// term query on the indexed `modality` field ("code" / "docs"), so only
    /// chunks tagged with the requested modality are returned.
    pub fn search_with_modality(
        &self,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        // Reload the reader to pick up latest commits.
        self.reader.reload().map_err(|e| Error::Index(e.to_string()))?;

        let searcher = self.reader.searcher();

        let query_parser =
            QueryParser::for_index(&self.index, vec![self.field_body, self.field_title]);

        let parsed_query =
            query_parser.parse_query(query).map_err(|e| Error::Index(e.to_string()))?;

        let tag = match modality {
            Modality::Both => None,
            Modality::Docs => Some("docs"),
            Modality::Code => Some("code"),
        };

        let top_docs = if let Some(tag) = tag {
            let term = Term::from_field_text(self.field_modality, tag);
            let term_query = TermQuery::new(term, IndexRecordOption::Basic);
            let boolean = BooleanQuery::new(vec![
                (Occur::Must, parsed_query),
                (Occur::Must, Box::new(term_query)),
            ]);
            searcher
                .search(&boolean, &TopDocs::with_limit(limit).order_by_score())
                .map_err(|e| Error::Index(e.to_string()))?
        } else {
            searcher
                .search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())
                .map_err(|e| Error::Index(e.to_string()))?
        };

        let mut results = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let retrieved: tantivy::TantivyDocument =
                searcher.doc(doc_address).map_err(|e| Error::Index(e.to_string()))?;

            let path = retrieved
                .get_first(self.field_path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let chunk_index_str =
                retrieved.get_first(self.field_chunk_index).and_then(|v| v.as_str()).unwrap_or("0");
            let chunk_index = chunk_index_str.parse::<usize>().unwrap_or(0);

            let score_f64 = score as f64;

            results.push(
                SearchResult::new(path, score_f64)
                    .with_chunk_index(Some(chunk_index))
                    .with_score_components(ScoreBreakdown {
                        bm25: score_f64,
                        vector: 0.0,
                        graph_boost: 0.0,
                        graph_hops: None,
                    }),
            );
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Port adapter: TextIndex
// ---------------------------------------------------------------------------

impl groundcontrol_common::ports::TextIndex for BM25Index {
    fn release_writer(&mut self) {
        BM25Index::release_writer(self)
    }

    fn add_document(
        &mut self,
        doc_path: &str,
        title: Option<&str>,
        tags: &[String],
        chunks: &[Chunk],
    ) -> Result<()> {
        BM25Index::add_document(self, doc_path, title, tags, chunks)
    }

    fn remove_document(&mut self, doc_path: &str) -> Result<()> {
        BM25Index::remove_document(self, doc_path)
    }

    fn commit(&mut self) -> Result<()> {
        BM25Index::commit(self)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        BM25Index::search(self, query, limit)
    }

    fn search_with_modality(
        &self,
        query: &str,
        limit: usize,
        modality: Modality,
    ) -> Result<Vec<SearchResult>> {
        BM25Index::search_with_modality(self, query, limit, modality)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use groundcontrol_common::types::Chunk;

    /// Helper to create a simple chunk.
    fn make_chunk(doc_path: &str, index: usize, text: &str) -> Chunk {
        Chunk::new(doc_path, index, text, 0, text.len())
    }

    #[test]
    fn test_create_in_memory() {
        let index = BM25Index::open_in_memory();
        assert!(index.is_ok());
    }

    #[test]
    fn test_add_and_search() {
        let mut index = BM25Index::open_in_memory().unwrap();

        let chunks1 = vec![make_chunk(
            "notes/rust.md",
            0,
            "Rust is a systems programming language focused on safety and performance",
        )];
        let chunks2 = vec![make_chunk(
            "notes/python.md",
            0,
            "Python is a dynamic interpreted language popular for data science",
        )];
        let chunks3 = vec![make_chunk(
            "notes/java.md",
            0,
            "Java is an object-oriented language that runs on the JVM",
        )];

        index
            .add_document(
                "notes/rust.md",
                Some("Rust Language"),
                &["rust".to_string(), "systems".to_string()],
                &chunks1,
            )
            .unwrap();
        index
            .add_document(
                "notes/python.md",
                Some("Python Language"),
                &["python".to_string()],
                &chunks2,
            )
            .unwrap();
        index
            .add_document("notes/java.md", Some("Java Language"), &["java".to_string()], &chunks3)
            .unwrap();
        index.commit().unwrap();

        let results = index.search("systems programming safety", 10).unwrap();
        assert!(!results.is_empty(), "Expected at least one result");
        assert_eq!(results[0].path, "notes/rust.md");
    }

    #[test]
    fn test_remove_document() {
        let mut index = BM25Index::open_in_memory().unwrap();

        let chunks = vec![make_chunk(
            "notes/remove_me.md",
            0,
            "This document should be removed from the index",
        )];
        index.add_document("notes/remove_me.md", Some("Remove Me"), &[], &chunks).unwrap();
        index.commit().unwrap();

        // Verify it's searchable first.
        let results = index.search("removed from the index", 10).unwrap();
        assert!(!results.is_empty());

        // Remove and commit.
        index.remove_document("notes/remove_me.md").unwrap();
        index.commit().unwrap();

        // Should no longer appear in results.
        let results = index.search("removed from the index", 10).unwrap();
        assert!(results.is_empty(), "Document should have been removed");
    }

    #[test]
    fn test_search_by_title() {
        let mut index = BM25Index::open_in_memory().unwrap();

        let chunks = vec![make_chunk(
            "notes/kubernetes.md",
            0,
            "Container orchestration platform for deploying applications",
        )];
        index
            .add_document(
                "notes/kubernetes.md",
                Some("Kubernetes Deep Dive"),
                &["k8s".to_string()],
                &chunks,
            )
            .unwrap();
        index.commit().unwrap();

        // Search using title text — should find the document.
        let results = index.search("Kubernetes Deep Dive", 10).unwrap();
        assert!(!results.is_empty(), "Should find document by title");
        assert_eq!(results[0].path, "notes/kubernetes.md");
    }

    #[test]
    fn test_search_multiple_results() {
        let mut index = BM25Index::open_in_memory().unwrap();

        let chunks1 = vec![make_chunk(
            "notes/ml_intro.md",
            0,
            "Machine learning is a subset of artificial intelligence that learns from data",
        )];
        let chunks2 = vec![make_chunk(
            "notes/ml_advanced.md",
            0,
            "Advanced machine learning covers deep learning neural networks and transformers",
        )];
        let chunks3 = vec![make_chunk(
            "notes/cooking.md",
            0,
            "This recipe explains how to make a perfect sourdough bread",
        )];

        index
            .add_document(
                "notes/ml_intro.md",
                Some("ML Introduction"),
                &["ml".to_string()],
                &chunks1,
            )
            .unwrap();
        index
            .add_document(
                "notes/ml_advanced.md",
                Some("Advanced ML"),
                &["ml".to_string(), "deep-learning".to_string()],
                &chunks2,
            )
            .unwrap();
        index
            .add_document(
                "notes/cooking.md",
                Some("Sourdough Recipe"),
                &["cooking".to_string()],
                &chunks3,
            )
            .unwrap();
        index.commit().unwrap();

        let results = index.search("machine learning", 10).unwrap();
        assert!(
            results.len() >= 2,
            "Expected at least 2 results for 'machine learning', got {}",
            results.len()
        );

        // Both ML documents should appear, cooking should not.
        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"notes/ml_intro.md"));
        assert!(paths.contains(&"notes/ml_advanced.md"));
        assert!(!paths.contains(&"notes/cooking.md"));

        // Results should be ordered by relevance (descending score).
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "Results should be in descending score order"
            );
        }
    }

    #[test]
    fn test_search_with_modality_filters() {
        let mut index = BM25Index::open_in_memory().unwrap();

        // A documentation chunk (default entity_kind = Documentation).
        let doc_chunk = make_chunk(
            "notes/guide.md",
            0,
            "The retrieval engine performs hybrid search over the corpus",
        );
        // A code chunk (entity_kind = CodeChunk via with_code_metadata).
        let code_chunk = Chunk::new(
            "src/engine.rs",
            0,
            "fn search_hybrid() performs hybrid retrieval over the corpus",
            0,
            60,
        )
        .with_code_metadata("rust", "crate::engine", 1, 3);

        index.add_document("notes/guide.md", Some("Guide"), &[], &[doc_chunk]).unwrap();
        index.add_document("src/engine.rs", Some("engine.rs"), &[], &[code_chunk]).unwrap();
        index.commit().unwrap();

        // Code modality returns only the code chunk.
        let code_results =
            index.search_with_modality("hybrid retrieval corpus", 10, Modality::Code).unwrap();
        assert!(!code_results.is_empty(), "expected code results");
        assert!(code_results.iter().all(|r| r.path == "src/engine.rs"));

        // Docs modality returns only the doc chunk.
        let doc_results =
            index.search_with_modality("hybrid search corpus", 10, Modality::Docs).unwrap();
        assert!(!doc_results.is_empty(), "expected doc results");
        assert!(doc_results.iter().all(|r| r.path == "notes/guide.md"));

        // Both returns both.
        let both_results = index.search_with_modality("hybrid corpus", 10, Modality::Both).unwrap();
        let paths: Vec<&str> = both_results.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"notes/guide.md"));
        assert!(paths.contains(&"src/engine.rs"));

        // Default `search` is equivalent to Modality::Both.
        let default_results = index.search("hybrid corpus", 10).unwrap();
        assert_eq!(default_results.len(), both_results.len());
    }

    #[test]
    fn test_lockfile_self_healing() {
        let temp = tempfile::TempDir::new().unwrap();
        let index_dir = temp.path().join("tantivy");
        std::fs::create_dir_all(&index_dir).unwrap();

        // Simulate an orphaned stale lock file left behind by a killed process
        let stale_lock = index_dir.join(".tantivy-writer.lock");
        std::fs::write(&stale_lock, b"stale lock content").unwrap();
        assert!(stale_lock.exists());

        // Opening BM25Index should detect and remove the stale lockfile
        let mut index = BM25Index::open(&index_dir).expect("should heal and open");

        // Verify we can write and commit
        let chunks = vec![make_chunk("doc.md", 0, "Test content")];
        index.add_document("doc.md", Some("Title"), &[], &chunks).unwrap();
        index.commit().unwrap();

        let res = index.search("content", 5).unwrap();
        assert_eq!(res.len(), 1);
    }
}
