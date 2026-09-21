//! Verification tests for GraphView layout, binary wire protocol, and tiered LOD.

use groundcontrol_graphview::layout::octree::Octree;
use groundcontrol_graphview::layout::{compute_force_layout, LayoutConfig};
use groundcontrol_graphview::layout::{EdgeLayout, GraphLayout, NodeLayout};
use groundcontrol_graphview::wire::binary::{BinaryWireEncoder, WIRE_MAGIC, WIRE_VERSION};

#[test]
fn test_octree_repulsion_synthetic_cloud() {
    let n = 2000;
    let mut positions = Vec::with_capacity(n);
    let mut masses = Vec::with_capacity(n);

    for i in 0..n {
        let f = i as f32;
        positions.push([f.sin() * 200.0, f.cos() * 200.0, (f * 0.5).sin() * 200.0]);
        masses.push(1.0 + (i % 10) as f32);
    }

    let octree = Octree::build(&positions, &masses, 0.8);
    let forces = octree.compute_all_repulsions(&positions, 5000.0, 100.0);

    assert_eq!(forces.len(), n);
    for f in forces {
        assert!(!f[0].is_nan());
        assert!(!f[1].is_nan());
        assert!(!f[2].is_nan());
    }
}

#[test]
fn test_force_layout_simulation() {
    let paths = vec![
        "crates/groundcontrol-core/src/lib.rs".to_string(),
        "crates/groundcontrol-core/src/engine.rs".to_string(),
        "docs/concepts/search.md".to_string(),
    ];
    let titles = vec![None, None, None];
    let degrees = vec![2, 1, 1];
    let communities = vec![0, 0, 1];
    let raw_edges =
        vec![(0, 1, "wikilink".to_string(), 1.0f32), (0, 2, "calls".to_string(), 1.0f32)];

    // Test Directory Clustering
    let dir_config = LayoutConfig {
        iterations: 15,
        cluster_mode: groundcontrol_graphview::layout::ClusterMode::Directory,
        ..Default::default()
    };
    let (dir_positions, dir_edges) =
        compute_force_layout(&paths, &titles, &degrees, &communities, &raw_edges, &dir_config);

    assert_eq!(dir_positions.len(), 3);
    assert_eq!(dir_edges.len(), 2);
    for pos in dir_positions {
        assert!(!pos[0].is_nan());
        assert!(!pos[1].is_nan());
        assert!(!pos[2].is_nan());
    }

    // Test Community Clustering
    let comm_config = LayoutConfig {
        iterations: 15,
        cluster_mode: groundcontrol_graphview::layout::ClusterMode::Community,
        ..Default::default()
    };
    let (comm_positions, comm_edges) =
        compute_force_layout(&paths, &titles, &degrees, &communities, &raw_edges, &comm_config);

    assert_eq!(comm_positions.len(), 3);
    assert_eq!(comm_edges.len(), 2);
    for pos in comm_positions {
        assert!(!pos[0].is_nan());
        assert!(!pos[1].is_nan());
        assert!(!pos[2].is_nan());
    }
}

#[test]
fn test_binary_protocol_packing() {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for i in 0..100 {
        nodes.push(NodeLayout {
            id: i as u32,
            path: format!("crates/core/src/file_{}.rs", i),
            title: Some(format!("Module {}", i)),
            position: [i as f32 * 2.0, (i * 3) as f32, (i * 4) as f32],
            degree: (i % 8) as usize,
            community: (i % 3) as u32,
            entity_type: if i % 2 == 0 { "Function".to_string() } else { "DocNode".to_string() },
            color_rgb: 0x10b981,
            size: 4.5,
        });

        if i > 0 {
            edges.push(EdgeLayout {
                source: (i - 1) as u32,
                target: i as u32,
                edge_type: "calls".to_string(),
                weight: 1.0,
            });
        }
    }

    let layout =
        GraphLayout { corpus: "test_corpus".to_string(), nodes, edges, communities_count: 3 };

    let encoded = BinaryWireEncoder::encode(&layout);
    assert!(encoded.len() > 16 + 100 * 32 + 99 * 12);

    let magic = u32::from_le_bytes(encoded[0..4].try_into().unwrap());
    let version = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
    let node_count = u32::from_le_bytes(encoded[8..12].try_into().unwrap());
    let edge_count = u32::from_le_bytes(encoded[12..16].try_into().unwrap());

    assert_eq!(magic, WIRE_MAGIC);
    assert_eq!(version, WIRE_VERSION);
    assert_eq!(node_count, 100);
    assert_eq!(edge_count, 99);
}
