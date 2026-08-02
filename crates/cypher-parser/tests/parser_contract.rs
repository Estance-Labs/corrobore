// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use cypher_parser::{LiteralValue, ParseErrorCode, QueryKind, parse_query};

#[test]
fn parse_query_returns_deterministic_normalized_ast_for_read_query() {
    let ast = parse_query(" MATCH (n) RETURN n LIMIT 1 ")
        .expect("supported query should parse into deterministic ast");

    assert_eq!(ast.normalized_query, "MATCH (n) RETURN n LIMIT 1");
    assert_eq!(ast.kind, QueryKind::Read);
}

#[test]
fn parse_query_rejects_unsupported_feature_with_actionable_shape() {
    let error = parse_query("LOAD CSV FROM 'file:///tmp/data.csv' AS row RETURN row")
        .expect_err("unsupported feature should be rejected");

    assert_eq!(error.code, ParseErrorCode::UnsupportedFeature);
    assert!(error.suggestion.is_some());
}

#[test]
fn parse_query_rejects_additional_unsupported_features() {
    let call_apoc = parse_query("CALL APOC RETURN 1").expect_err("CALL APOC should be rejected");
    assert_eq!(call_apoc.code, ParseErrorCode::UnsupportedFeature);
    assert!(call_apoc.message.contains("CALL APOC"));

    let call_dbms = parse_query("CALL DBMS RETURN 1").expect_err("CALL DBMS should be rejected");
    assert_eq!(call_dbms.code, ParseErrorCode::UnsupportedFeature);
    assert!(call_dbms.message.contains("CALL DBMS"));

    let foreach =
        parse_query("FOREACH (x IN [1,2] | CREATE (:N))").expect_err("FOREACH should be rejected");
    assert_eq!(foreach.code, ParseErrorCode::UnsupportedFeature);
    assert!(foreach.message.contains("FOREACH"));
}

#[test]
fn parse_query_classifies_read_mutation_and_mixed_kinds() {
    let read = parse_query("MATCH (n) RETURN n").expect("read query should parse");
    assert_eq!(read.kind, QueryKind::Read);

    let mutation = parse_query("CREATE (n:Indicator)").expect("mutation query should parse");
    assert_eq!(mutation.kind, QueryKind::Mutation);

    let mixed = parse_query("MATCH (n) DELETE n").expect("mixed query should parse");
    assert_eq!(mixed.kind, QueryKind::Mixed);
}

#[test]
fn parse_merge_relationship_preserves_source_edge_and_target_patterns() {
    let ast = parse_query(
        "MATCH (s:Entity {id: 'source'}) MERGE (s)-[r:TARGETS]->(o:Entity {id: 'target'}) RETURN r",
    )
    .expect("relationship MERGE should parse");

    let parsed = ast.query.expect("structured query should be attached");
    let merge = parsed.merge_clause.expect("MERGE clause should exist");
    assert_eq!(merge.pattern.variable, "s");
    let (relationship, target) = merge
        .relationship
        .expect("relationship and target patterns should be retained");
    assert_eq!(relationship.variable.as_deref(), Some("r"));
    assert_eq!(relationship.rel_type.as_deref(), Some("TARGETS"));
    assert_eq!(target.variable, "o");
    assert_eq!(target.label.as_deref(), Some("Entity"));
}

#[test]
fn parse_create_relationship_preserves_source_edge_and_target_patterns() {
    let ast = parse_query(
        "MATCH (s:Entity {id: 'source'}) CREATE (s)-[r:TARGETS]->(o:Entity {id: 'target'}) RETURN r",
    )
    .expect("relationship CREATE should parse");

    let parsed = ast.query.expect("structured query should be attached");
    let create = parsed.create_clause.expect("CREATE clause should exist");
    let (relationship, target) = create
        .relationship
        .expect("relationship and target patterns should be retained");
    assert_eq!(relationship.variable.as_deref(), Some("r"));
    assert_eq!(relationship.rel_type.as_deref(), Some("TARGETS"));
    assert_eq!(target.variable, "o");
    assert_eq!(target.label.as_deref(), Some("Entity"));
}

// ---------------------------------------------------------------------------
// Mutation parsing: CREATE
// ---------------------------------------------------------------------------

#[test]
fn parse_create_node_with_label_produces_create_clause() {
    let ast = parse_query("CREATE (n:Indicator)").expect("CREATE with label should parse");

    assert_eq!(ast.kind, QueryKind::Mutation);
    let parsed = ast.query.expect("structured query should be attached");
    let create = parsed.create_clause.expect("CREATE clause should exist");
    assert_eq!(create.nodes.len(), 1);
    assert_eq!(create.nodes[0].variable, "n");
    assert_eq!(create.nodes[0].label.as_deref(), Some("Indicator"));
    assert!(create.nodes[0].properties.is_empty());
}

#[test]
fn parse_create_node_with_inline_properties_captures_properties() {
    let ast = parse_query("CREATE (n:Indicator {name: 'alpha', score: 10})")
        .expect("CREATE with inline properties should parse");

    assert_eq!(ast.kind, QueryKind::Mutation);
    let parsed = ast.query.expect("structured query should be attached");
    let create = parsed.create_clause.expect("CREATE clause should exist");
    assert_eq!(create.nodes.len(), 1);
    assert_eq!(
        create.nodes[0].properties,
        vec![
            ("name".to_owned(), LiteralValue::String("alpha".to_owned())),
            ("score".to_owned(), LiteralValue::Integer(10)),
        ]
    );
}

#[test]
fn parse_create_node_with_return_produces_both_clauses() {
    let ast = parse_query("CREATE (n:Indicator {name: 'x'}) RETURN n")
        .expect("CREATE+RETURN should parse");

    // CREATE + RETURN is classified as Mixed because RETURN is a read clause.
    assert_eq!(ast.kind, QueryKind::Mixed);
    let parsed = ast.query.expect("structured query should be attached");
    assert!(parsed.create_clause.is_some());
    assert!(parsed.return_clause.is_some());
}

// ---------------------------------------------------------------------------
// Mutation parsing: MERGE
// ---------------------------------------------------------------------------

#[test]
fn parse_merge_node_produces_merge_clause() {
    let ast =
        parse_query("MERGE (n:Indicator {name: 'alpha'})").expect("MERGE with props should parse");

    assert_eq!(ast.kind, QueryKind::Mutation);
    let parsed = ast.query.expect("structured query should be attached");
    let merge = parsed.merge_clause.expect("MERGE clause should exist");
    assert_eq!(merge.pattern.variable, "n");
    assert_eq!(merge.pattern.label.as_deref(), Some("Indicator"));
    assert_eq!(merge.pattern.properties.len(), 1);
    assert!(merge.relationship.is_none());
}

#[test]
fn parse_merge_node_with_return_produces_both_clauses() {
    let ast = parse_query("MERGE (n:Indicator {name: 'alpha'}) RETURN n")
        .expect("MERGE+RETURN should parse");

    let parsed = ast.query.expect("structured query should be attached");
    assert!(parsed.merge_clause.is_some());
    assert!(parsed.return_clause.is_some());
}

// ---------------------------------------------------------------------------
// Mutation parsing: SET (mixed query)
// ---------------------------------------------------------------------------

#[test]
fn parse_match_set_return_produces_set_clause() {
    let ast = parse_query("MATCH (n:Indicator) WHERE n.name = 'alpha' SET n.score = 20 RETURN n")
        .expect("MATCH+SET+RETURN should parse");

    assert_eq!(ast.kind, QueryKind::Mixed);
    let parsed = ast.query.expect("structured query should be attached");
    assert!(parsed.match_clause.is_some());
    let set = parsed.set_clause.expect("SET clause should exist");
    assert_eq!(set.assignments.len(), 1);
    assert_eq!(set.assignments[0].target.variable, "n");
    assert_eq!(set.assignments[0].target.property, "score");
    assert_eq!(set.assignments[0].value, LiteralValue::Integer(20));
    assert!(parsed.return_clause.is_some());
}

#[test]
fn parse_match_set_with_multiple_assignments() {
    let ast = parse_query("MATCH (n:Indicator) SET n.score = 20, n.active = true RETURN n")
        .expect("SET with multiple assignments should parse");

    let parsed = ast.query.expect("structured query should be attached");
    let set = parsed.set_clause.expect("SET clause should exist");
    assert_eq!(set.assignments.len(), 2);
}

#[test]
fn parse_set_preserves_homogeneous_typed_list_literals() {
    let ast = parse_query(
        "MATCH (n:Indicator) SET n.aliases = ['alpha,beta', 'gamma'], n.ports = [80, 443], n.scores = [0.4, 0.9], n.flags = [true, false] RETURN n",
    )
    .expect("bounded homogeneous list literals should parse");

    let parsed = ast.query.expect("structured query should be attached");
    let assignments = parsed
        .set_clause
        .expect("SET clause should exist")
        .assignments;
    assert_eq!(assignments.len(), 4);
    assert_eq!(
        assignments[0].value,
        LiteralValue::List(vec![
            LiteralValue::String("alpha,beta".to_owned()),
            LiteralValue::String("gamma".to_owned()),
        ])
    );
    assert_eq!(
        assignments[1].value,
        LiteralValue::List(vec![LiteralValue::Integer(80), LiteralValue::Integer(443)])
    );
    assert_eq!(
        assignments[2].value,
        LiteralValue::List(vec![
            LiteralValue::Float("0.4".to_owned()),
            LiteralValue::Float("0.9".to_owned()),
        ])
    );
    assert_eq!(
        assignments[3].value,
        LiteralValue::List(vec![
            LiteralValue::Boolean(true),
            LiteralValue::Boolean(false),
        ])
    );
}

#[test]
fn parse_set_preserves_bounded_property_reference_lists() {
    let ast = parse_query(
        "MATCH (r:Report)-[rel:CITES]->(e:Evidence) SET rel.evidence_refs = [e.id] RETURN rel",
    )
    .expect("a bounded property-reference list should parse");

    let assignment = &ast
        .query
        .expect("structured query should be attached")
        .set_clause
        .expect("SET clause should exist")
        .assignments[0];
    assert_eq!(
        assignment.value,
        LiteralValue::PropertyReferenceList(vec![cypher_parser::PropertyRef {
            variable: "e".to_owned(),
            property: "id".to_owned(),
        }])
    );
}

#[test]
fn parse_set_accepts_the_guides_double_quoted_status_literal() {
    let ast = parse_query(
        "MATCH (r:Report)-[rel:CITES]->(e:Evidence) SET rel.status = \"candidate\" RETURN rel.status",
    )
    .expect("the user-facing guide syntax should remain valid");

    assert_eq!(
        ast.query
            .expect("structured query should be attached")
            .set_clause
            .expect("SET clause should exist")
            .assignments[0]
            .value,
        LiteralValue::String("candidate".to_owned())
    );
}

#[test]
fn parse_the_complete_multi_match_guide_example_without_rewriting_it() {
    let ast = parse_query(
        "MATCH (a:ThreatActor {name: \"APT28\"}) MATCH (e:EvidenceSpan {id: \"span--123\"}) MERGE (a)-[r:USES]->(m:Malware {name: \"X-Agent\"}) SET r.confidence = 0.82, r.evidence_refs = [e.id], r.status = \"candidate\" RETURN r",
    )
    .expect("the complete guide example should parse unchanged");

    let parsed = ast.query.expect("structured query should be attached");
    let matched = parsed.match_clause.expect("primary MATCH should exist");
    assert_eq!(matched.start.variable, "a");
    assert_eq!(matched.additional_nodes.len(), 1);
    assert_eq!(matched.additional_nodes[0].variable, "e");
    assert!(parsed.merge_clause.is_some());
    assert!(parsed.set_clause.is_some());
}

#[test]
fn parse_create_keeps_clause_words_inside_quoted_property_text_as_data() {
    parse_query(
        "CREATE (n:ThreatActor {stix_id: 'intrusion-set--94f0bef7-d7a2-51fd-99f4-c2df6e1a9ac4', name: 'Cicada', description: 'Chinese government-linked APT group involved in espionage-type operations since 2009, with a strong focus on Japanese organizations and MSPs. Uses living-off-the-land tools, custom DLL loaders, and cu', confidence: 50})",
    )
    .expect("quoted description text must not become Cypher syntax");
}

#[test]
fn parse_set_rejects_mixed_and_nested_list_literals() {
    for query in [
        "MATCH (n) SET n.values = ['alpha', 1]",
        "MATCH (n) SET n.values = [[1, 2]]",
    ] {
        let error = parse_query(query).expect_err("unsupported list shape should be rejected");
        assert_eq!(error.code, ParseErrorCode::InvalidSyntax);
        assert!(error.message.contains("list"));
    }
}

// ---------------------------------------------------------------------------
// Mutation parsing: DELETE (mixed query)
// ---------------------------------------------------------------------------

#[test]
fn parse_match_delete_produces_delete_clause() {
    let ast = parse_query("MATCH (n:Indicator) WHERE n.name = 'alpha' DELETE n")
        .expect("MATCH+DELETE should parse");

    assert_eq!(ast.kind, QueryKind::Mixed);
    let parsed = ast.query.expect("structured query should be attached");
    assert!(parsed.match_clause.is_some());
    let delete = parsed.delete_clause.expect("DELETE clause should exist");
    assert_eq!(delete.variables, vec!["n".to_owned()]);
}

// ---------------------------------------------------------------------------
// Mutation parsing: REMOVE (mixed query)
// ---------------------------------------------------------------------------

#[test]
fn parse_match_remove_produces_remove_clause() {
    let ast = parse_query("MATCH (n:Indicator) REMOVE n.score RETURN n")
        .expect("MATCH+REMOVE+RETURN should parse");

    assert_eq!(ast.kind, QueryKind::Mixed);
    let parsed = ast.query.expect("structured query should be attached");
    assert!(parsed.match_clause.is_some());
    let remove = parsed.remove_clause.expect("REMOVE clause should exist");
    assert_eq!(remove.targets.len(), 1);
    assert_eq!(remove.targets[0].variable, "n");
    assert_eq!(remove.targets[0].property, "score");
    assert!(parsed.return_clause.is_some());
}

// ---------------------------------------------------------------------------
// Node pattern inline properties (used by both CREATE and MERGE)
// ---------------------------------------------------------------------------

#[test]
fn parse_node_pattern_with_inline_properties_in_match() {
    let ast = parse_query("MATCH (n:Indicator {name: 'alpha'}) RETURN n")
        .expect("MATCH with inline properties should parse");

    let parsed = ast.query.expect("structured query should be attached");
    let mc = parsed.match_clause.expect("match clause should exist");
    assert_eq!(mc.start.properties.len(), 1);
    assert_eq!(
        mc.start.properties[0],
        ("name".to_owned(), LiteralValue::String("alpha".to_owned()))
    );
}
