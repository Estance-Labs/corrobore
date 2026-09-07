// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! SQL compiles into the shared structural query AST (epic #90, item #259).
//!
//! The contract under test is the boundary the epic asks for: a documented
//! SQL subset becomes the same `cypher_parser::QueryAst` the Cypher frontend
//! produces, consumed by the same planner and executor, with a relational
//! projection that says which SQL column comes from which typed record value.
//! No SQL text is ever turned into Cypher text.
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, HashMap};

use cypher_executor::{RecordNode, RecordRelationship, RecordValue};
use cypher_parser::{
    ClauseKind, ComparisonOperator, LiteralValue, OrderDirection, ProjectionItem, QueryKind,
    WhereExpression,
};
use sql_frontend::{
    Cell, ColumnType, CommandTag, QueryMode, SqlCommand, SqlValue, column_type, compile,
    project_row, split_statements,
};

fn query(sql: &str) -> sql_frontend::CompiledQuery {
    match compile(sql, &[]).unwrap() {
        SqlCommand::Query(query) => query,
        other => panic!("expected a graph query, got {other:?}"),
    }
}

#[test]
fn select_from_a_label_table_compiles_to_a_bounded_match() {
    let compiled = query(
        r#"SELECT a.name AS actor, a.tier, a.id FROM "ThreatActor" AS a
           WHERE a.tier >= 2 AND (a.name <> 'x' OR a.active IS NOT NULL)
           ORDER BY a.name DESC LIMIT 20 OFFSET 5"#,
    );
    assert_eq!(compiled.mode, QueryMode::Read);
    assert_eq!(compiled.tag, CommandTag::Select);
    let ast = &compiled.ast;
    assert_eq!(ast.kind, QueryKind::Read);
    assert!(ast.clauses.contains(&ClauseKind::Match));
    assert!(ast.clauses.contains(&ClauseKind::Where));
    assert!(ast.clauses.contains(&ClauseKind::Return));
    let parsed = ast.query.as_ref().unwrap();
    let matched = parsed.match_clause.as_ref().unwrap();
    assert_eq!(matched.start.variable, "a");
    assert_eq!(matched.start.label.as_deref(), Some("ThreatActor"));
    assert!(matched.relationship.is_none());

    // WHERE keeps its boolean structure: AND binds tighter, parentheses nest.
    let WhereExpression::And(terms) = &parsed.where_clause.as_ref().unwrap().expression else {
        panic!("top level is a conjunction");
    };
    assert!(matches!(
        &terms[0],
        WhereExpression::Comparison { left, operator: ComparisonOperator::Gte, right: LiteralValue::Integer(2) }
            if left.variable == "a" && left.property == "tier"
    ));
    assert!(matches!(&terms[1], WhereExpression::Or(inner) if inner.len() == 2));

    // Property columns are pushed down as property projections in SELECT order,
    // with ORDER BY, LIMIT and OFFSET carried by the same clause.
    let returned = parsed.return_clause.as_ref().unwrap();
    assert_eq!(returned.items.len(), 3);
    assert!(matches!(&returned.items[0], ProjectionItem::Property(p) if p.property == "name"));
    assert!(matches!(&returned.items[2], ProjectionItem::Property(p) if p.property == "id"));
    assert_eq!(returned.order_by[0].direction, OrderDirection::Desc);
    assert_eq!(returned.limit, Some(20));
    assert_eq!(returned.skip, Some(5));

    // The relational projection names the SQL columns, alias first.
    let names: Vec<&str> = compiled.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["actor", "tier", "id"]);
}

#[test]
fn a_join_on_relationship_endpoints_compiles_to_the_pattern() {
    let compiled = query(
        r#"SELECT a.name, r.type, r.source_id, r.target_id, m.name
           FROM "ThreatActor" AS a
           JOIN "USES" AS r ON r.source_id = a.id
           JOIN "Malware" AS m ON r.target_id = m.id
           WHERE m.family = 'X-Agent' LIMIT 50"#,
    );
    let parsed = compiled.ast.query.as_ref().unwrap();
    let matched = parsed.match_clause.as_ref().unwrap();
    assert_eq!(matched.start.label.as_deref(), Some("ThreatActor"));
    let (relationship, target) = matched.relationship.as_ref().unwrap();
    assert_eq!(relationship.variable.as_deref(), Some("r"));
    assert_eq!(relationship.rel_type.as_deref(), Some("USES"));
    assert_eq!(target.variable, "m");
    assert_eq!(target.label.as_deref(), Some("Malware"));
    // Structural columns of the relationship cannot be property projections,
    // so the whole variables are returned and the projection derives the cells.
    let returned = parsed.return_clause.as_ref().unwrap();
    assert!(
        returned
            .items
            .iter()
            .any(|item| matches!(item, ProjectionItem::Variable(v) if v == "r"))
    );
    let names: Vec<&str> = compiled.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["name", "type", "source_id", "target_id", "name"]
    );
}

#[test]
fn the_reverse_join_direction_is_honoured() {
    let compiled = query(
        r#"SELECT m.name FROM "Malware" AS m
           JOIN "USES" AS r ON r.target_id = m.id
           JOIN "ThreatActor" AS a ON r.source_id = a.id"#,
    );
    let matched = compiled
        .ast
        .query
        .as_ref()
        .unwrap()
        .match_clause
        .as_ref()
        .unwrap();
    // The pattern is always source -> target, whatever order the joins name.
    assert_eq!(matched.start.variable, "a");
    assert_eq!(matched.relationship.as_ref().unwrap().1.variable, "m");
}

#[test]
fn aggregates_compile_to_typed_projection_items() {
    let compiled = query(
        r#"SELECT count(*) AS total, sum(a.tier), avg(a.tier), min(a.tier), max(a.tier) FROM "ThreatActor" AS a"#,
    );
    let returned = compiled
        .ast
        .query
        .as_ref()
        .unwrap()
        .return_clause
        .as_ref()
        .unwrap();
    assert!(matches!(&returned.items[0], ProjectionItem::Count(v) if v == "*"));
    assert!(matches!(&returned.items[1], ProjectionItem::Sum(_)));
    assert!(matches!(&returned.items[2], ProjectionItem::Average(_)));
    assert!(matches!(&returned.items[3], ProjectionItem::Minimum(_)));
    assert!(matches!(&returned.items[4], ProjectionItem::Maximum(_)));
    let names: Vec<&str> = compiled.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["total", "sum", "avg", "min", "max"]);
    assert_eq!(compiled.ast.aggregations.len(), 5);
}

#[test]
fn star_and_structural_columns_return_whole_records() {
    let compiled = query(r#"SELECT * FROM "Indicator" AS i LIMIT 5"#);
    let names: Vec<&str> = compiled.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "id",
            "labels",
            "status",
            "confidence",
            "evidence_refs",
            "properties"
        ]
    );
    let returned = compiled
        .ast
        .query
        .as_ref()
        .unwrap()
        .return_clause
        .as_ref()
        .unwrap();
    assert_eq!(
        returned.items,
        vec![ProjectionItem::Variable("i".to_owned())]
    );
    assert_eq!(returned.limit, Some(5));
}

#[test]
fn the_generic_tables_scan_every_label_and_type() {
    let compiled = query("SELECT n.id FROM nodes AS n WHERE n.status = 'validated'");
    let matched = compiled
        .ast
        .query
        .as_ref()
        .unwrap()
        .match_clause
        .as_ref()
        .unwrap();
    assert!(matched.start.label.is_none());

    let compiled = query("SELECT r.id, r.type FROM relationships AS r WHERE r.type = 'USES'");
    let matched = compiled
        .ast
        .query
        .as_ref()
        .unwrap()
        .match_clause
        .as_ref()
        .unwrap();
    let (relationship, _) = matched.relationship.as_ref().unwrap();
    // A type equality on the relationship folds into the pattern rather than
    // becoming a predicate the executor cannot evaluate.
    assert_eq!(relationship.rel_type.as_deref(), Some("USES"));
    assert!(compiled.ast.query.as_ref().unwrap().where_clause.is_none());
}

#[test]
fn insert_update_and_delete_compile_to_mutations() {
    let insert = query(
        r#"INSERT INTO "Sensor" (name, reading, live) VALUES ('north', 42, true) RETURNING id, name"#,
    );
    assert_eq!(insert.mode, QueryMode::Mutation);
    assert_eq!(insert.tag, CommandTag::Insert);
    assert!(matches!(
        insert.ast.kind,
        QueryKind::Mutation | QueryKind::Mixed
    ));
    let create = insert
        .ast
        .query
        .as_ref()
        .unwrap()
        .create_clause
        .as_ref()
        .unwrap();
    assert_eq!(create.nodes[0].label.as_deref(), Some("Sensor"));
    assert_eq!(
        create.nodes[0].properties,
        vec![
            ("name".to_owned(), LiteralValue::String("north".into())),
            ("reading".to_owned(), LiteralValue::Integer(42)),
            ("live".to_owned(), LiteralValue::Boolean(true)),
        ]
    );
    assert!(insert.ast.query.as_ref().unwrap().return_clause.is_some());

    let update =
        query(r#"UPDATE "Sensor" AS s SET reading = 43, live = false WHERE s.name = 'north'"#);
    assert_eq!(update.tag, CommandTag::Update);
    let parsed = update.ast.query.as_ref().unwrap();
    assert_eq!(
        parsed.match_clause.as_ref().unwrap().start.label.as_deref(),
        Some("Sensor")
    );
    let set = parsed.set_clause.as_ref().unwrap();
    assert_eq!(set.assignments.len(), 2);
    assert_eq!(set.assignments[0].target.variable, "s");
    assert_eq!(set.assignments[0].value, LiteralValue::Integer(43));

    let delete = query(r#"DELETE FROM "Sensor" WHERE name = 'north'"#);
    assert_eq!(delete.tag, CommandTag::Delete);
    let parsed = delete.ast.query.as_ref().unwrap();
    // Without an alias the table gets a generated variable that the predicate
    // and the DELETE both use.
    let variable = parsed.match_clause.as_ref().unwrap().start.variable.clone();
    assert_eq!(
        parsed.delete_clause.as_ref().unwrap().variables,
        vec![variable.clone()]
    );
    assert!(matches!(
        &parsed.where_clause.as_ref().unwrap().expression,
        WhereExpression::Comparison { left, .. } if left.variable == variable && left.property == "name"
    ));
}

#[test]
fn parameters_bind_by_position_and_by_name() {
    let compiled = match compile(
        r#"SELECT a.name FROM "ThreatActor" AS a WHERE a.tier > $1 AND a.name = $who LIMIT $2"#,
        &[
            SqlValue::Integer(2),
            SqlValue::Integer(10),
            SqlValue::Text("APT28".into()),
        ],
    )
    .unwrap()
    {
        SqlCommand::Query(query) => query,
        other => panic!("{other:?}"),
    };
    // Named parameters are supplied after the positional ones, in first-use
    // order, so `$who` is the third value here.
    let parsed = compiled.ast.query.as_ref().unwrap();
    let WhereExpression::And(terms) = &parsed.where_clause.as_ref().unwrap().expression else {
        panic!("conjunction expected");
    };
    assert!(matches!(
        &terms[0],
        WhereExpression::Comparison {
            right: LiteralValue::Integer(2),
            ..
        }
    ));
    assert!(
        matches!(&terms[1], WhereExpression::Comparison { right: LiteralValue::String(s), .. } if s == "APT28")
    );
    assert_eq!(parsed.return_clause.as_ref().unwrap().limit, Some(10));

    let error = compile(
        r#"SELECT a.name FROM "ThreatActor" AS a WHERE a.tier > $1"#,
        &[],
    )
    .unwrap_err();
    assert_eq!(error.sqlstate, "22023");
}

#[test]
fn unsupported_shapes_fail_with_stable_sqlstates() {
    for (sql, sqlstate) in [
        ("WITH RECURSIVE t AS (SELECT 1) SELECT * FROM t", "0A000"),
        (r#"SELECT a.name FROM "A" AS a GROUP BY a.name"#, "0A000"),
        (r#"SELECT a.name FROM "A" AS a, "B" AS b"#, "0A000"),
        (
            r#"INSERT INTO "USES" (source_id, target_id) VALUES ('a', 'b')"#,
            "0A000",
        ),
        (
            r#"SELECT a.name FROM "A" AS a WHERE a.labels = 'x'"#,
            "0A000",
        ),
        (r#"SELECT a.name FROM "A" AS a ORDER BY count(a)"#, "0A000"),
        (r#"SELECT a.name, count(*) FROM "A" AS a"#, "42803"),
        ("SELEKT 1", "42601"),
        (r#"SELECT b.name FROM "A" AS a"#, "42P01"),
        ("DROP TABLE x", "0A000"),
    ] {
        let error = compile(sql, &[]).unwrap_err();
        assert_eq!(error.sqlstate, sqlstate, "{sql}: {}", error.message);
    }
}

#[test]
fn catalog_scalar_and_control_statements_are_recognised() {
    assert!(matches!(
        compile("SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name", &[]).unwrap(),
        SqlCommand::Catalog(catalog) if catalog.table.is_none()
    ));
    assert!(matches!(
        compile("SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'Actor'", &[]).unwrap(),
        SqlCommand::Catalog(catalog) if catalog.table.as_deref() == Some("Actor")
    ));
    let SqlCommand::Scalar(scalar) =
        compile("SELECT version(), 1 AS one, 'x' AS s, current_user", &[]).unwrap()
    else {
        panic!("scalar select");
    };
    let names: Vec<&str> = scalar.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["version", "one", "s", "current_user"]);
    assert!(matches!(&scalar.row[0], Cell::Text(text) if text.contains("Corrobore")));
    assert_eq!(scalar.row[1], Cell::Int(1));
    for (sql, expected) in [
        ("BEGIN", SqlCommand::Begin { read_only: false }),
        (
            "START TRANSACTION READ ONLY",
            SqlCommand::Begin { read_only: true },
        ),
        ("COMMIT", SqlCommand::Commit),
        ("END", SqlCommand::Commit),
        ("ROLLBACK", SqlCommand::Rollback),
        ("SET client_encoding TO 'UTF8'", SqlCommand::Set),
        ("SET search_path = public", SqlCommand::Set),
        (
            "SHOW server_version",
            SqlCommand::Show("server_version".to_owned()),
        ),
        ("", SqlCommand::Empty),
        ("   ", SqlCommand::Empty),
    ] {
        assert_eq!(compile(sql, &[]).unwrap(), expected, "{sql}");
    }
}

#[test]
fn statements_split_on_semicolons_outside_quotes() {
    let statements =
        split_statements("SELECT 1; INSERT INTO \"A\" (x) VALUES ('a;b'); ; SET \"k;\" = 1;");
    assert_eq!(
        statements,
        vec![
            "SELECT 1",
            "INSERT INTO \"A\" (x) VALUES ('a;b')",
            "SET \"k;\" = 1",
        ]
    );
}

#[test]
fn rows_project_typed_cells_from_record_values() {
    let compiled = query(
        r#"SELECT a.id, a.labels, a.name, a.tier, a.properties, r.type, r.source_id, m.id
           FROM "Actor" AS a JOIN "USES" AS r ON r.source_id = a.id JOIN "Malware" AS m ON r.target_id = m.id"#,
    );
    let mut actor_properties = BTreeMap::new();
    actor_properties.insert("name".to_owned(), RecordValue::String("APT28".into()));
    actor_properties.insert("tier".to_owned(), RecordValue::Integer(3));
    actor_properties.insert("status".to_owned(), RecordValue::String("candidate".into()));
    let mut values = HashMap::new();
    values.insert(
        "a".to_owned(),
        RecordValue::Node(RecordNode {
            id: "node--a".into(),
            labels: vec!["Actor".into()],
            properties: actor_properties,
        }),
    );
    values.insert(
        "r".to_owned(),
        RecordValue::Relationship(RecordRelationship {
            id: "rel--1".into(),
            rel_type: "USES".into(),
            source_id: "node--a".into(),
            target_id: "node--m".into(),
            properties: BTreeMap::new(),
        }),
    );
    values.insert(
        "m".to_owned(),
        RecordValue::Node(RecordNode {
            id: "node--m".into(),
            labels: vec!["Malware".into()],
            properties: BTreeMap::new(),
        }),
    );
    let row = project_row(&compiled.columns, &values);
    assert_eq!(row[0], Cell::Text("node--a".into()));
    assert_eq!(row[1], Cell::Json(serde_json::json!(["Actor"])));
    assert_eq!(row[2], Cell::Text("APT28".into()));
    assert_eq!(row[3], Cell::Int(3));
    // `properties` excludes the reserved native metadata keys.
    assert_eq!(
        row[4],
        Cell::Json(serde_json::json!({"name": "APT28", "tier": 3}))
    );
    assert_eq!(row[5], Cell::Text("USES".into()));
    assert_eq!(row[6], Cell::Text("node--a".into()));
    assert_eq!(row[7], Cell::Text("node--m".into()));

    // Column types are inferred from the cells a result actually holds.
    assert_eq!(column_type(&[Cell::Int(1), Cell::Null]), ColumnType::Int8);
    assert_eq!(
        column_type(&[Cell::Int(1), Cell::Float("2.5".into())]),
        ColumnType::Float8
    );
    assert_eq!(column_type(&[Cell::Bool(true)]), ColumnType::Bool);
    assert_eq!(
        column_type(&[Cell::Json(serde_json::json!([]))]),
        ColumnType::Jsonb
    );
    assert_eq!(column_type(&[Cell::Null]), ColumnType::Text);
    assert_eq!(
        column_type(&[Cell::Int(1), Cell::Text("x".into())]),
        ColumnType::Text
    );
}
