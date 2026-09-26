use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::Path;

use groundcontrol_common::{Error, Result};

use super::types::{GraphData, GraphDataRef, GRAPH_SCHEMA_VERSION};
use super::KnowledgeGraph;

impl KnowledgeGraph {
    /// Serialize the graph directly to disk without cloning the in-memory graph.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path).map_err(|e| Error::Graph(format!("create file: {}", e)))?;
        let mut writer = BufWriter::new(file);
        let data = GraphDataRef { version: GRAPH_SCHEMA_VERSION, graph: &self.graph };
        postcard::to_io(&data, &mut writer)
            .map_err(|e| Error::Graph(format!("serialize stream: {}", e)))?;
        Ok(())
    }

    /// Deserialize a graph from disk via buffered reading.
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::Graph(format!("open file: {}", e)))?;
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| Error::Graph(format!("read stream: {}", e)))?;

        let data: GraphData = postcard::from_bytes(&bytes)
            .map_err(|e| Error::Graph(format!("deserialize: {}", e)))?;

        if data.version != GRAPH_SCHEMA_VERSION {
            return Err(Error::Graph(format!(
                "graph schema version mismatch: on-disk {} != expected {}; rebuild required",
                data.version, GRAPH_SCHEMA_VERSION
            )));
        }

        let mut node_map = HashMap::with_capacity(data.graph.node_count());
        for idx in data.graph.node_indices() {
            if let Some(node) = data.graph.node_weight(idx) {
                let _ = node_map.insert(node.path.clone(), idx);
            }
        }

        Ok(Self { graph: data.graph, node_map })
    }
}
