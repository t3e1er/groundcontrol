use std::collections::HashMap;
use std::path::Path;

use groundcontrol_common::{Error, Result};

use super::types::{GraphData, GRAPH_SCHEMA_VERSION};
use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// Serialize the graph to a file using postcard.
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = GraphData { version: GRAPH_SCHEMA_VERSION, graph: self.graph.clone() };
        let encoded =
            postcard::to_allocvec(&data).map_err(|e| Error::Graph(format!("serialize: {}", e)))?;
        std::fs::write(path, encoded).map_err(|e| Error::Graph(format!("write: {}", e)))?;
        Ok(())
    }

    /// Deserialize a graph from a file.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::Graph(format!("read: {}", e)))?;
        let data: GraphData = postcard::from_bytes(&bytes)
            .map_err(|e| Error::Graph(format!("deserialize: {}", e)))?;

        if data.version != GRAPH_SCHEMA_VERSION {
            return Err(Error::Graph(format!(
                "graph schema version mismatch: on-disk {} != expected {}; rebuild required",
                data.version, GRAPH_SCHEMA_VERSION
            )));
        }

        let mut node_map = HashMap::new();
        for idx in data.graph.node_indices() {
            if let Some(node) = data.graph.node_weight(idx) {
                let _ = node_map.insert(node.path.clone(), idx);
            }
        }

        Ok(Self { graph: data.graph, node_map })
    }
}
