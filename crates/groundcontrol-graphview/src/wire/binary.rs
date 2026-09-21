//! High-density packed binary wire protocol serializer for 1M+ nodes.
//!
//! Layout:
//! - Header: 16 bytes (Magic, Version, NodeCount, EdgeCount)
//! - Nodes: 32 bytes per node
//! - Edges: 12 bytes per edge
//! - String Table: Length-prefixed UTF-8 strings

use std::collections::HashMap;

use crate::layout::GraphLayout;

/// Magic 4-byte header ("CTXV").
pub const WIRE_MAGIC: u32 = 0x43545856;
/// Protocol format version.
pub const WIRE_VERSION: u32 = 1;

/// Packed binary serializer.
pub struct BinaryWireEncoder;

impl BinaryWireEncoder {
    /// Encode a `GraphLayout` into a packed binary byte array.
    pub fn encode(layout: &GraphLayout) -> Vec<u8> {
        let node_count = layout.nodes.len() as u32;
        let edge_count = layout.edges.len() as u32;

        // Collect distinct strings for string table (entity_types, edge_types, and node paths)
        let mut string_table: Vec<String> = Vec::new();
        let mut string_to_idx: HashMap<String, u16> = HashMap::new();

        let mut intern_string = |s: &str| -> u16 {
            if let Some(&idx) = string_to_idx.get(s) {
                return idx;
            }
            let idx = string_table.len() as u16;
            string_table.push(s.to_string());
            string_to_idx.insert(s.to_string(), idx);
            idx
        };

        // Intern entity types and paths
        let mut node_type_indices = Vec::with_capacity(layout.nodes.len());
        let mut node_path_indices = Vec::with_capacity(layout.nodes.len());
        for node in &layout.nodes {
            node_type_indices.push(intern_string(&node.entity_type));
            node_path_indices.push(intern_string(&node.path));
        }

        // Intern edge types
        let mut edge_type_indices = Vec::with_capacity(layout.edges.len());
        for edge in &layout.edges {
            edge_type_indices.push(intern_string(&edge.edge_type));
        }

        // Compute total buffer capacity:
        // 16 (header) + 32 * nodes + 12 * edges + string table overhead
        let estimated_size = 16 + (node_count as usize) * 32 + (edge_count as usize) * 12 + 4096;
        let mut buf = Vec::with_capacity(estimated_size);

        // 1. Header (16 Bytes)
        buf.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
        buf.extend_from_slice(&WIRE_VERSION.to_le_bytes());
        buf.extend_from_slice(&node_count.to_le_bytes());
        buf.extend_from_slice(&edge_count.to_le_bytes());

        // 2. Nodes (32 Bytes each)
        for (i, node) in layout.nodes.iter().enumerate() {
            buf.extend_from_slice(&node.id.to_le_bytes()); // 4B
            buf.extend_from_slice(&node.position[0].to_le_bytes()); // 4B
            buf.extend_from_slice(&node.position[1].to_le_bytes()); // 4B
            buf.extend_from_slice(&node.position[2].to_le_bytes()); // 4B
            buf.extend_from_slice(&(node.community as u16).to_le_bytes()); // 2B
            buf.extend_from_slice(&(node.degree as u16).to_le_bytes()); // 2B
            buf.extend_from_slice(&node.color_rgb.to_le_bytes()); // 4B
            buf.extend_from_slice(&node.size.to_le_bytes()); // 4B
            buf.extend_from_slice(&node_type_indices[i].to_le_bytes()); // 2B
            buf.extend_from_slice(&node_path_indices[i].to_le_bytes()); // 2B
        }

        // 3. Edges (12 Bytes each)
        for (i, edge) in layout.edges.iter().enumerate() {
            buf.extend_from_slice(&edge.source.to_le_bytes()); // 4B
            buf.extend_from_slice(&edge.target.to_le_bytes()); // 4B
            buf.extend_from_slice(&edge_type_indices[i].to_le_bytes()); // 2B
            let quantized_weight = ((edge.weight.clamp(0.0, 10.0)) * 1000.0) as u16;
            buf.extend_from_slice(&quantized_weight.to_le_bytes()); // 2B
        }

        // 4. String Table
        let strings_count = string_table.len() as u32;
        buf.extend_from_slice(&strings_count.to_le_bytes());
        for s in string_table {
            let bytes = s.as_bytes();
            let len = bytes.len() as u16;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(bytes);
        }

        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{EdgeLayout, NodeLayout};

    #[test]
    fn test_binary_encode_header() {
        let layout = GraphLayout {
            corpus: "test".to_string(),
            nodes: vec![NodeLayout {
                id: 0,
                path: "test.md".to_string(),
                title: Some("Test".to_string()),
                position: [10.0, 20.0, 30.0],
                degree: 2,
                community: 1,
                entity_type: "DocNode".to_string(),
                color_rgb: 0x3b82f6,
                size: 5.0,
            }],
            edges: vec![EdgeLayout {
                source: 0,
                target: 0,
                edge_type: "wikilink".to_string(),
                weight: 1.0,
            }],
            communities_count: 1,
        };

        let encoded = BinaryWireEncoder::encode(&layout);
        assert!(encoded.len() >= 16 + 32 + 12);

        let magic = u32::from_le_bytes(encoded[0..4].try_into().unwrap());
        let version = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
        let node_count = u32::from_le_bytes(encoded[8..12].try_into().unwrap());
        let edge_count = u32::from_le_bytes(encoded[12..16].try_into().unwrap());

        assert_eq!(magic, WIRE_MAGIC);
        assert_eq!(version, WIRE_VERSION);
        assert_eq!(node_count, 1);
        assert_eq!(edge_count, 1);
    }
}
