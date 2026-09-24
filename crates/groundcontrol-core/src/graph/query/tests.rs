//! Unit tests for Cypher-Lite linear query parser and engine.

use super::ast::QueryDirection;
use super::parser::parse_path_pattern;

#[test]
fn test_parse_simple_pattern() {
    let pattern = parse_path_pattern("(:CodeSymbol {name: 'SelectVictimsOnNode'})-[:implements]->(:Interface)<-[:calls*1..2]-(c:CodeSymbol)").unwrap();
    assert_eq!(pattern.start_node.label.as_deref(), Some("CodeSymbol"));
    assert_eq!(
        pattern.start_node.properties.get("name").map(|s| s.as_str()),
        Some("SelectVictimsOnNode")
    );
    assert_eq!(pattern.steps.len(), 2);

    let (ref e1, ref n1) = pattern.steps[0];
    assert_eq!(e1.direction, QueryDirection::Outgoing);
    assert_eq!(e1.edge_types, vec!["implements"]);
    assert_eq!(n1.label.as_deref(), Some("Interface"));

    let (ref e2, ref n2) = pattern.steps[1];
    assert_eq!(e2.direction, QueryDirection::Incoming);
    assert_eq!(e2.edge_types, vec!["calls"]);
    assert_eq!(e2.min_hops, 1);
    assert_eq!(e2.max_hops, 2);
    assert_eq!(n2.variable.as_deref(), Some("c"));
    assert_eq!(n2.label.as_deref(), Some("CodeSymbol"));
}

#[test]
fn test_parse_doc_pattern() {
    let pattern = parse_path_pattern(
        "(:DocNode {title: 'Manifest Admission'})-[:wikilink*1..2]->(related:DocNode)",
    )
    .unwrap();
    assert_eq!(pattern.start_node.label.as_deref(), Some("DocNode"));
    assert_eq!(
        pattern.start_node.properties.get("title").map(|s| s.as_str()),
        Some("Manifest Admission")
    );
    assert_eq!(pattern.steps.len(), 1);
    let (ref e, ref n) = pattern.steps[0];
    assert_eq!(e.edge_types, vec!["wikilink"]);
    assert_eq!(n.variable.as_deref(), Some("related"));
    assert_eq!(n.label.as_deref(), Some("DocNode"));
}

#[test]
fn test_parse_shorthand_arrows() {
    let pattern = parse_path_pattern("(a)-->(b)<--(c)--(d)").unwrap();
    assert_eq!(pattern.steps.len(), 3);
    assert_eq!(pattern.steps[0].0.direction, QueryDirection::Outgoing);
    assert_eq!(pattern.steps[1].0.direction, QueryDirection::Incoming);
    assert_eq!(pattern.steps[2].0.direction, QueryDirection::Undirected);
}
