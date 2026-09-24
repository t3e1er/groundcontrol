//! Full-text BM25 search index backed by Tantivy.

use std::path::{Path, PathBuf};

use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::{BooleanQuery, Occur, QueryParser, TermQuery},
    schema::{IndexRecordOption, Term, Value},
    Index, IndexReader, IndexWriter, ReloadPolicy,
};

use groundcontrol_common::{
    ports::TextIndex,
    types::{Chunk, EntityKind, Modality, ScoreBreakdown, SearchResult},
    Error, Result,
};

use super::lockfile::heal_stale_lockfiles;
use super::schema::{build_schema, BM25Fields};

/// Full-text BM25 search index backed by Tantivy.
pub struct BM25Index {
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    index_path: Option<PathBuf>,
    fields: BM25Fields,
}

impl BM25Index {
    /// Open or create a Tantivy index at the given directory.
    pub fn open(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path).map_err(|e| Error::Index(e.to_string()))?;

        // Clean up any stale lockfiles from previously killed processes.
        heal_stale_lockfiles(index_path);

        let (schema, fields) = build_schema();

        let dir = MmapDirectory::open(index_path).map_err(|e| Error::Index(e.to_string()))?;

        let index = Index::open_or_create(dir, schema).map_err(|e| Error::Index(e.to_string()))?;

        // Don't acquire writer at open — only needed for mutations.
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e: tantivy::TantivyError| Error::Index(e.to_string()))?;

        Ok(Self { index, reader, writer: None, index_path: Some(index_path.to_path_buf()), fields })
    }

    /// Create an in-memory index (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let (schema, fields) = build_schema();

        let index = Index::create_in_ram(schema);

        // Don't acquire writer at open — only needed for mutations.
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e: tantivy::TantivyError| Error::Index(e.to_string()))?;

        Ok(Self { index, reader, writer: None, index_path: None, fields })
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

        let field_path = self.fields.path;
        let field_chunk_index = self.fields.chunk_index;
        let field_title = self.fields.title;
        let field_body = self.fields.body;
        let field_tags = self.fields.tags;
        let field_modality = self.fields.modality;

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
        let field_path = self.fields.path;
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
            QueryParser::for_index(&self.index, vec![self.fields.body, self.fields.title]);

        let parsed_query =
            query_parser.parse_query(query).map_err(|e| Error::Index(e.to_string()))?;

        let tag = match modality {
            Modality::Both => None,
            Modality::Docs => Some("docs"),
            Modality::Code => Some("code"),
        };

        let top_docs = if let Some(tag) = tag {
            let term = Term::from_field_text(self.fields.modality, tag);
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
                .get_first(self.fields.path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let chunk_index_str = retrieved
                .get_first(self.fields.chunk_index)
                .and_then(|v| v.as_str())
                .unwrap_or("0");
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

impl TextIndex for BM25Index {
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
