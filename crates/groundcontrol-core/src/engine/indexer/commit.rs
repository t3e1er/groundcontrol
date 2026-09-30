//! Transactional commits and intermediate checkpointing.

use groundcontrol_common::Result;

use crate::engine::state::Engine;

impl Engine {
    /// Commit pending lexical updates and checkpoints metadata without
    /// incurring quadratic graph re-serialization and edge table rewrites.
    pub fn commit_intermediate(&mut self) -> Result<()> {
        let _ = self.store.commit_batch();
        let _ = self.store.checkpoint_passive();
        self.bm25.commit()?;
        Ok(())
    }

    /// Commit all pending changes across all registered retrieval algorithms and SQLite.
    pub fn commit(&mut self) -> Result<()> {
        let _ = self.store.commit_batch();
        let _ = self.store.checkpoint();
        self.graph.save(&self.index_dir.join("graph.bin"))?;
        if let Some(ref vi) = self.vector_index() {
            if vi.is_dirty() && !vi.is_empty() {
                let _ = vi.save_binary(&self.index_dir.join("vectors.bin"));
            }
        }
        if !self.binary.is_empty() {
            let _ = self.binary.save_to_path(&self.index_dir.join("fingerprints_v3.bin"));
        }
        self.commit_algorithms()?;
        Ok(())
    }
}
