//! Transactional commits and intermediate checkpointing.

use groundcontrol_common::Result;

use crate::engine::state::Engine;

impl Engine {
    /// Commit pending lexical updates and checkpoints metadata without
    /// incurring quadratic graph re-serialization and edge table rewrites.
    pub fn commit_intermediate(&mut self) -> Result<()> {
        self.bm25.commit()?;
        self.store.commit_batch()?;
        let _ = self.store.checkpoint_passive();
        Ok(())
    }

    /// Commit all pending changes across all registered retrieval algorithms and SQLite.
    pub fn commit(&mut self) -> Result<()> {
        self.commit_algorithms()?;
        self.graph.save(&self.index_dir.join("graph.bin"))?;
        if let Some(ref vi) = self.vector_index() {
            if vi.is_dirty() && !vi.is_empty() {
                let _ = vi.save_binary(&self.index_dir.join("vectors.bin"));
            }
        }
        if !self.binary.is_empty() {
            let _ = self.binary.save_to_path(&self.index_dir.join("fingerprints_v3.bin"));
        }
        self.store.commit_batch()?;
        self.store.checkpoint()?;
        Ok(())
    }

    /// Abandon any uncommitted changes in SQLite batch and release uncommitted BM25 writer buffer.
    pub fn abandon_uncommitted_batch(&mut self) {
        let _ = self.store.rollback_batch();
        self.bm25.release_writer();
    }

    /// Execute a closure within a transactional boundary across catalog and search indices.
    pub fn transactional<F, R>(&mut self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Engine) -> Result<R>,
    {
        self.store.begin_batch()?;
        match f(self) {
            Ok(res) => {
                self.commit()?;
                Ok(res)
            }
            Err(e) => {
                self.abandon_uncommitted_batch();
                Err(e)
            }
        }
    }
}
