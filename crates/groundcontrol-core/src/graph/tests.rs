#![allow(unused_results)]

use std::collections::{HashMap, HashSet};

use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::Serialize;

use groundcontrol_common::config::{EdgeClass, EdgeSource, EdgeTypeConfig};
use groundcontrol_common::types::{
    CommunityDetectionResult, Document, EdgeProvenance, ResolutionConfidence, WikiLink,
};

use super::types::GRAPH_SCHEMA_VERSION;
use super::{GraphEdge, GraphNode, KnowledgeGraph};

fn make_doc(path: &str, title: &str, tags: Vec<&str>, wikilinks: Vec<&str>) -> Document {
    Document {
        path: path.to_string(),
        frontmatter: None,
        title: Some(title.to_string()),
        tags: tags.into_iter().map(|t| t.to_string()).collect(),
        wikilinks: wikilinks
            .into_iter()
            .map(|t| WikiLink { target: t.to_string(), alias: None })
            .collect(),
        template: None,
        content: String::new(),
        content_hash: String::new(),
    }
}

fn wikilink_config() -> EdgeTypeConfig {
    EdgeTypeConfig {
        name: "Wikilink".to_string(),
        source: EdgeSource::Wikilink,
        weight: 1.0,
        bidirectional: false,
        field: None,
        direction: None,
        max_frequency: None,
        class: None,
        description: None,
        allowed_source_templates: None,
        allowed_target_templates: None,
    }
}

fn tag_config() -> EdgeTypeConfig {
    EdgeTypeConfig {
        name: "SharedTag".to_string(),
        source: EdgeSource::Tag,
        weight: 0.5,
        bidirectional: false,
        field: None,
        direction: None,
        max_frequency: None,
        class: None,
        description: None,
        allowed_source_templates: None,
        allowed_target_templates: None,
    }
}

#[test]
fn test_add_remove_nodes() {
    let mut graph = KnowledgeGraph::new();

    graph.add_node("a.md", Some("A"));
    graph.add_node("b.md", Some("B"));
    graph.add_node("c.md", Some("C"));
    assert_eq!(graph.node_count(), 3);

    graph.remove_node("b.md").unwrap();
    assert_eq!(graph.node_count(), 2);
    assert!(graph.get_node("b.md").is_none());
    assert!(graph.get_node("a.md").is_some());
    assert!(graph.get_node("c.md").is_some());

    // Removing a non-existent node returns error.
    assert!(graph.remove_node("nonexistent.md").is_err());
}

#[test]
fn test_add_edges() {
    let mut graph = KnowledgeGraph::new();

    graph.add_node("a.md", Some("A"));
    graph.add_node("b.md", Some("B"));
    graph.add_node("c.md", Some("C"));

    graph.add_edge("a.md", "b.md", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("b.md", "c.md", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    assert_eq!(graph.edge_count(), 2);

    // Traversal from a should reach b and c.
    let results = graph.traverse_bfs("a.md", 3, None, None);
    let paths: Vec<&str> = results.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"b.md"));
    assert!(paths.contains(&"c.md"));
}

#[test]
fn test_build_edges_from_wikilinks() {
    let mut graph = KnowledgeGraph::new();

    let doc = make_doc("note.md", "Note", vec![], vec!["target1.md", "target2.md"]);
    let configs = vec![wikilink_config()];

    graph.build_edges_for_document(&doc, &configs, std::slice::from_ref(&doc));

    assert_eq!(graph.edge_count(), 2);
    assert!(graph.get_node("target1.md").is_some());
    assert!(graph.get_node("target2.md").is_some());

    // Verify forward links.
    let fwd = graph.forwardlinks("note.md", None);
    let targets = fwd.get("Wikilink").unwrap();
    assert!(targets.contains(&"target1.md".to_string()));
    assert!(targets.contains(&"target2.md".to_string()));
}

#[test]
fn test_build_edges_from_shared_tags() {
    let mut graph = KnowledgeGraph::new();

    let doc_a = make_doc("a.md", "A", vec!["rust", "async"], vec![]);
    let doc_b = make_doc("b.md", "B", vec!["rust"], vec![]);
    let doc_c = make_doc("c.md", "C", vec!["python"], vec![]);

    let all_docs = vec![doc_a.clone(), doc_b.clone(), doc_c.clone()];
    let configs = vec![tag_config()];

    graph.build_edges_for_document(&doc_a, &configs, &all_docs);

    // a shares "rust" with b, but not with c.
    assert!(graph.edge_count() >= 1);
    let fwd = graph.forwardlinks("a.md", None);
    let shared = fwd.get("SharedTag").unwrap();
    assert!(shared.contains(&"b.md".to_string()));
    assert!(!shared.contains(&"c.md".to_string()));
}

#[test]
fn test_bfs_traversal() {
    let mut graph = KnowledgeGraph::new();

    // Chain: A → B → C → D → E
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("D", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let results = graph.traverse_bfs("A", 2, None, None);
    let paths: Vec<&str> = results.iter().map(|(p, _)| p.as_str()).collect();

    assert!(paths.contains(&"B"));
    assert!(paths.contains(&"C"));
    assert!(!paths.contains(&"D"));
    assert!(!paths.contains(&"E"));

    // Verify depths.
    let b_depth = results.iter().find(|(p, _)| p == "B").unwrap().1;
    let c_depth = results.iter().find(|(p, _)| p == "C").unwrap().1;
    assert_eq!(b_depth, 1);
    assert_eq!(c_depth, 2);
}

#[test]
fn test_dfs_traversal() {
    let mut graph = KnowledgeGraph::new();

    // Chain: A → B → C → D → E
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("D", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let results = graph.traverse_dfs("A", 3, None, None);
    let paths: Vec<&str> = results.iter().map(|(p, _)| p.as_str()).collect();

    // DFS should reach B, C, D within depth 3.
    assert!(paths.contains(&"B"));
    assert!(paths.contains(&"C"));
    assert!(paths.contains(&"D"));
    // E is at depth 4, so should not appear.
    assert!(!paths.contains(&"E"));
}

#[test]
fn test_save_load() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("a.md", Some("A"));
    graph.add_node("b.md", Some("B"));
    graph.add_edge("a.md", "b.md", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.bin");

    graph.save(&path).unwrap();

    let loaded = KnowledgeGraph::load(&path).unwrap();
    assert_eq!(loaded.node_count(), graph.node_count());
    assert_eq!(loaded.edge_count(), graph.edge_count());
    assert!(loaded.get_node("a.md").is_some());
    assert!(loaded.get_node("b.md").is_some());

    // Verify edge is preserved.
    let fwd = loaded.forwardlinks("a.md", None);
    assert!(fwd.get("Link").unwrap().contains(&"b.md".to_string()));
}

#[test]
fn test_cross_corpus_edge_payload_round_trips() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("doc.md", Some("Doc"));
    graph.add_cross_corpus_edge(
        "doc.md",
        "backend::api::create_user",
        "documents",
        1.0,
        EdgeProvenance::DocumentsCode,
        EdgeClass::Structural,
        Some("backend".to_string()),
        Some(ResolutionConfidence::High),
        Some("src/api/user.rs".to_string()),
        Some("api::create_user".to_string()),
        Some("Symbol".to_string()),
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.bin");
    graph.save(&path).unwrap();
    let loaded = KnowledgeGraph::load(&path).unwrap();

    // The full remote-endpoint payload must survive save() -> load().
    let src_idx = loaded.get_node("doc.md").expect("source node present");
    let edge = loaded
        .inner_graph()
        .edges_directed(src_idx, Direction::Outgoing)
        .next()
        .expect("cross-corpus edge present");
    let data = edge.weight();
    assert_eq!(data.target_corpus.as_deref(), Some("backend"));
    assert_eq!(data.confidence, Some(ResolutionConfidence::High));
    assert_eq!(data.target_path.as_deref(), Some("src/api/user.rs"));
    assert_eq!(data.target_symbol.as_deref(), Some("api::create_user"));
    assert_eq!(data.target_kind.as_deref(), Some("Symbol"));
}

#[test]
fn test_load_rejects_schema_version_mismatch() {
    #[derive(Serialize)]
    struct LegacyGraphData {
        version: u32,
        graph: petgraph::graph::DiGraph<GraphNode, GraphEdge>,
    }
    let legacy = LegacyGraphData {
        version: GRAPH_SCHEMA_VERSION.wrapping_add(1),
        graph: petgraph::graph::DiGraph::new(),
    };
    let bytes = postcard::to_allocvec(&legacy).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.bin");
    std::fs::write(&path, bytes).unwrap();

    assert!(KnowledgeGraph::load(&path).is_err());
}

#[test]
fn test_backlinks_forwardlinks() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("A", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    // backlinks(B) should contain A.
    let bl = graph.backlinks("B", None);
    let sources = bl.get("Link").unwrap();
    assert!(sources.contains(&"A".to_string()));
    assert_eq!(sources.len(), 1);

    // forwardlinks(A) should contain B and C.
    let fl = graph.forwardlinks("A", None);
    let targets = fl.get("Link").unwrap();
    assert!(targets.contains(&"B".to_string()));
    assert!(targets.contains(&"C".to_string()));
    assert_eq!(targets.len(), 2);
}

#[test]
fn test_shortest_path() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let path = graph.shortest_path("A", "D", None, None).unwrap();
    assert_eq!(path, vec!["A", "B", "C", "D"]);

    // No path from D to A (directed graph).
    assert!(graph.shortest_path("D", "A", None, None).is_none());

    // Non-existent node.
    assert!(graph.shortest_path("A", "Z", None, None).is_none());
}

#[test]
fn test_graph_stats() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("orphan.md", Some("Orphan"));
    graph.add_edge("a.md", "b.md", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("a.md", "c.md", "Tag", 0.5, EdgeProvenance::SharedTag, EdgeClass::Semantic);
    graph.add_edge("b.md", "c.md", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let stats = graph.stats();
    assert_eq!(stats.node_count, 4); // orphan, a, b, c
    assert_eq!(stats.edge_count, 3);
    assert_eq!(stats.orphan_count, 1);
    assert_eq!(*stats.edge_type_distribution.get("Link").unwrap(), 2);
    assert_eq!(*stats.edge_type_distribution.get("Tag").unwrap(), 1);

    // Most connected: a.md has 2 outgoing, b.md has 1 in + 1 out = 2, c.md has 2 in.
    assert!(!stats.most_connected.is_empty());
}

#[test]
fn test_community_detection_empty_graph() {
    let graph = KnowledgeGraph::new();
    let result = graph.detect_communities();
    assert!(result.communities.is_empty());
    assert_eq!(result.modularity, 0.0);
    assert_eq!(result.iterations, 0);
}

#[test]
fn test_community_detection_single_node() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("single.md", Some("Single"));

    let result = graph.detect_communities();
    assert_eq!(result.communities.len(), 1);
    assert_eq!(result.communities[0].members.len(), 1);
    assert_eq!(result.communities[0].members[0], "single.md");
}

#[test]
fn test_community_detection_two_cliques() {
    let mut graph = KnowledgeGraph::new();

    // Clique 1: A, B, C
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("A", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    // Clique 2: D, E, F
    graph.add_edge("D", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("E", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("D", "F", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("F", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("E", "F", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("F", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    // Single weak link between cliques.
    graph.add_edge("C", "D", "Link", 0.1, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let result = graph.detect_communities();

    assert_eq!(
        result.communities.len(),
        2,
        "Expected 2 communities, got {}: {:?}",
        result.communities.len(),
        result.communities
    );

    assert!(
        result.modularity > 0.0,
        "Modularity should be positive for well-separated clusters: {}",
        result.modularity
    );

    let mut sizes: Vec<usize> = result.communities.iter().map(|c| c.members.len()).collect();
    sizes.sort();
    assert_eq!(sizes, vec![3, 3]);

    let abc_community =
        result.communities.iter().find(|c| c.members.contains(&"A".to_string())).unwrap();
    assert!(abc_community.members.contains(&"B".to_string()));
    assert!(abc_community.members.contains(&"C".to_string()));

    let def_community =
        result.communities.iter().find(|c| c.members.contains(&"D".to_string())).unwrap();
    assert!(def_community.members.contains(&"E".to_string()));
    assert!(def_community.members.contains(&"F".to_string()));
}

#[test]
fn test_community_detection_disconnected_components() {
    let mut graph = KnowledgeGraph::new();

    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    graph.add_edge("C", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("D", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let result = graph.detect_communities();

    assert!(
        result.communities.len() >= 2,
        "Disconnected components should be in separate communities"
    );

    let ab_community =
        result.communities.iter().find(|c| c.members.contains(&"A".to_string())).unwrap();
    assert!(ab_community.members.contains(&"B".to_string()));

    let cd_community =
        result.communities.iter().find(|c| c.members.contains(&"C".to_string())).unwrap();
    assert!(cd_community.members.contains(&"D".to_string()));

    assert!(!ab_community.members.contains(&"C".to_string()));
}

#[test]
fn test_community_detection_no_edges() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("A", None);
    graph.add_node("B", None);
    graph.add_node("C", None);

    let result = graph.detect_communities();

    assert_eq!(result.communities.len(), 3);
    assert_eq!(result.modularity, 0.0);
}

fn all_communities_connected(graph: &KnowledgeGraph, result: &CommunityDetectionResult) -> bool {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for edge in graph.inner_graph().edge_references() {
        let s = graph.inner_graph().node_weight(edge.source()).unwrap().path.clone();
        let t = graph.inner_graph().node_weight(edge.target()).unwrap().path.clone();
        if s == t {
            continue;
        }
        adjacency.entry(s.clone()).or_default().push(t.clone());
        adjacency.entry(t).or_default().push(s);
    }

    for comm in &result.communities {
        if comm.members.len() <= 1 {
            continue;
        }
        let member_set: HashSet<&str> = comm.members.iter().map(|s| s.as_str()).collect();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(comm.members[0].clone());
        let _ = visited.insert(comm.members[0].clone());
        while let Some(node) = queue.pop_front() {
            if let Some(neigh) = adjacency.get(&node) {
                for n in neigh {
                    if member_set.contains(n.as_str()) && !visited.contains(n) {
                        let _ = visited.insert(n.clone());
                        queue.push_back(n.clone());
                    }
                }
            }
        }
        if visited.len() != comm.members.len() {
            return false;
        }
    }
    true
}

#[test]
fn test_leiden_communities_are_internally_connected() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("D", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("E", "F", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("F", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "D", "Link", 0.05, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let leiden = graph.detect_communities_leiden();
    assert!(
        all_communities_connected(&graph, &leiden),
        "every Leiden community must be internally connected"
    );
}

#[test]
fn test_leiden_splits_disconnected_community() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge("H1", "a", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("H1", "b", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("a", "b", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("H2", "c", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("H2", "d", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("c", "d", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let leiden = graph.detect_communities_leiden();
    assert!(all_communities_connected(&graph, &leiden));
    for comm in &leiden.communities {
        let has_c1 = comm.members.iter().any(|m| m == "H1" || m == "a" || m == "b");
        let has_c2 = comm.members.iter().any(|m| m == "H2" || m == "c" || m == "d");
        assert!(!(has_c1 && has_c2), "a community must not span two disconnected components");
    }
}

#[test]
fn test_leiden_is_deterministic() {
    let build = || {
        let mut graph = KnowledgeGraph::new();
        graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("C", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("D", "E", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("E", "F", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("F", "D", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph.add_edge("C", "D", "Link", 0.05, EdgeProvenance::Wikilink, EdgeClass::Structural);
        graph
    };

    let r1 = build().detect_communities_leiden();
    let r2 = build().detect_communities_leiden();

    assert_eq!(r1.communities.len(), r2.communities.len());
    let members1: Vec<Vec<String>> = r1.communities.iter().map(|c| c.members.clone()).collect();
    let members2: Vec<Vec<String>> = r2.communities.iter().map(|c| c.members.clone()).collect();
    assert_eq!(members1, members2, "Leiden partition must be deterministic");
    assert_eq!(r1.modularity, r2.modularity);
}

#[test]
fn test_community_densities() {
    let mut graph = KnowledgeGraph::new();

    graph.add_edge("A", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("A", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "A", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("B", "C", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);
    graph.add_edge("C", "B", "Link", 1.0, EdgeProvenance::Wikilink, EdgeClass::Structural);

    let densities = graph.community_densities();
    assert!(!densities.is_empty());

    let triangle =
        densities.iter().find(|d| d.node_count == 3).expect("Should find a community with 3 nodes");
    assert!(
        (triangle.density - 1.0).abs() < 0.001,
        "Fully connected triangle should have density 1.0, got {}",
        triangle.density
    );
}

#[test]
fn test_traverse_lineage_outgoing() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "docs/adrs/003.md",
        "docs/adrs/002.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "docs/adrs/002.md",
        "docs/adrs/001.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let chain = graph.traverse_lineage("docs/adrs/003.md", "supersedes", "outgoing", 5);
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[0].path, "docs/adrs/003.md");
    assert_eq!(chain[0].depth, 0);
    assert_eq!(chain[0].direction, "start");

    assert_eq!(chain[1].path, "docs/adrs/002.md");
    assert_eq!(chain[1].depth, 1);
    assert_eq!(chain[1].direction, "outgoing");

    assert_eq!(chain[2].path, "docs/adrs/001.md");
    assert_eq!(chain[2].depth, 2);
    assert_eq!(chain[2].direction, "outgoing");
}

#[test]
fn test_traverse_lineage_incoming() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "docs/adrs/003.md",
        "docs/adrs/002.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "docs/adrs/002.md",
        "docs/adrs/001.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let chain = graph.traverse_lineage("docs/adrs/001.md", "supersedes", "incoming", 5);
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[0].path, "docs/adrs/001.md");
    assert_eq!(chain[0].depth, 0);

    assert_eq!(chain[1].path, "docs/adrs/002.md");
    assert_eq!(chain[1].depth, 1);
    assert_eq!(chain[1].direction, "incoming");

    assert_eq!(chain[2].path, "docs/adrs/003.md");
    assert_eq!(chain[2].depth, 2);
    assert_eq!(chain[2].direction, "incoming");
}

#[test]
fn test_traverse_lineage_handles_cycle() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "A.md",
        "B.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "B.md",
        "A.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let chain = graph.traverse_lineage("A.md", "supersedes", "outgoing", 5);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].path, "A.md");
    assert_eq!(chain[1].path, "B.md");
}

#[test]
fn test_extract_lineage_for_node() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "B.md",
        "A.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "A.md",
        "C.md",
        "implements",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "D.md",
        "A.md",
        "depends_on",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let lineage = graph.extract_lineage_for_node("A.md").expect("Should have lineage for A.md");
    assert_eq!(lineage.superseded_by, vec!["B.md"]);
    assert_eq!(lineage.implements, vec!["C.md"]);
    assert_eq!(lineage.depended_on_by, vec!["D.md"]);
    assert!(lineage.supersedes.is_empty());
}

#[test]
fn test_detect_broken_links() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "docs/valid.md",
        "docs/missing.md",
        "Wikilink",
        1.0,
        EdgeProvenance::Wikilink,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "docs/valid.md",
        "docs/existing.md",
        "Wikilink",
        1.0,
        EdgeProvenance::Wikilink,
        EdgeClass::Structural,
    );

    let mut existing = HashSet::new();
    existing.insert("docs/valid.md".to_string());
    existing.insert("docs/existing.md".to_string());

    let broken = graph.detect_broken_links(&existing);
    assert_eq!(broken.len(), 1);
    assert_eq!(broken[0].source, "docs/valid.md");
    assert_eq!(broken[0].target, "docs/missing.md");
}

#[test]
fn test_detect_circular_dependencies() {
    let mut graph = KnowledgeGraph::new();
    graph.add_edge(
        "A.md",
        "B.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "B.md",
        "C.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );
    graph.add_edge(
        "C.md",
        "A.md",
        "supersedes",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let cycles = graph.detect_circular_dependencies(&["supersedes"]);
    assert!(!cycles.is_empty());
    assert_eq!(cycles[0].edge_type, "supersedes");
    assert!(cycles[0].cycle.contains(&"A.md".to_string()));
    assert!(cycles[0].cycle.contains(&"B.md".to_string()));
    assert!(cycles[0].cycle.contains(&"C.md".to_string()));
}

#[test]
fn test_detect_orphan_adrs() {
    let mut graph = KnowledgeGraph::new();
    graph.add_node("docs/adrs/001.md", Some("Connected ADR"));
    graph.add_node("docs/adrs/002.md", Some("Orphan ADR"));
    graph.add_node("docs/specs/engine.md", Some("Spec"));

    graph.add_edge(
        "docs/adrs/001.md",
        "docs/specs/engine.md",
        "adr_for",
        1.0,
        EdgeProvenance::Frontmatter,
        EdgeClass::Structural,
    );

    let adr_paths = vec!["docs/adrs/001.md".to_string(), "docs/adrs/002.md".to_string()];

    let orphans = graph.detect_orphan_adrs(&adr_paths);
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].path, "docs/adrs/002.md");
}
