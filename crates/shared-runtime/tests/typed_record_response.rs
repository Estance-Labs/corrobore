// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! The gateway response carries the executor's typed values and ordered
//! columns for protocol adapters, while its JSON contract stays byte-identical
//! for HTTP callers (epic #12, item #258).
#![allow(clippy::unwrap_used)]

use graph_core::{SessionId, WorkspaceId};
use shared_runtime::{
    CypherBudgetRef, CypherGateway, CypherParameters, CypherRequest, CypherResponseData,
    RecordValue,
};

fn request(query: &str, mutation: bool) -> CypherRequest {
    let workspace = WorkspaceId::new("workspace--typed").unwrap();
    let session = SessionId::new("session--typed").unwrap();
    let budget = CypherBudgetRef::new("budget--typed").unwrap();
    if mutation {
        CypherRequest::build_mutation_request(
            query,
            CypherParameters::default(),
            workspace,
            session,
            budget,
        )
        .unwrap()
    } else {
        CypherRequest::build_read_only_request(
            query,
            CypherParameters::default(),
            workspace,
            session,
            budget,
        )
        .unwrap()
    }
}

#[test]
fn gateway_records_expose_typed_values_and_columns() {
    let mut gateway = CypherGateway::strict_default();
    gateway
        .execute(&request(
            "CREATE (n:Reading {name: 'north', value: 7, live: true}) RETURN n",
            true,
        ))
        .unwrap();

    let response = gateway
        .execute(&request(
            "MATCH (n:Reading) RETURN n.value, n.name, n LIMIT 5",
            false,
        ))
        .unwrap();

    assert_eq!(response.columns, vec!["n.value", "n.name", "n"]);
    let CypherResponseData::Records(records) = &response.data else {
        panic!("expected records");
    };
    let record = &records[0];
    assert_eq!(record.values["n.value"], RecordValue::Integer(7));
    assert_eq!(record.values["n.name"], RecordValue::String("north".into()));
    assert!(matches!(record.values["n"], RecordValue::Node(_)));
    // The string twin is untouched.
    assert_eq!(record.fields["n.value"], "7");
}

#[test]
fn typed_values_and_columns_never_reach_the_json_contract() {
    let mut gateway = CypherGateway::strict_default();
    gateway
        .execute(&request("CREATE (n:Reading {value: 7}) RETURN n", true))
        .unwrap();
    let response = gateway
        .execute(&request("MATCH (n:Reading) RETURN n.value LIMIT 1", false))
        .unwrap();

    let json = serde_json::to_value(&response).unwrap();
    // No new key at the response level and none inside a record: the HTTP
    // surface promised by the OpenAPI contract is unchanged.
    assert!(json.get("columns").is_none());
    let record = &json["data"]["Records"][0];
    assert_eq!(record["fields"]["n.value"], "7");
    assert!(record.get("values").is_none());

    // A response deserialized from JSON has empty typed twins rather than
    // failing, so older payloads still parse.
    let parsed: shared_runtime::CypherResponse = serde_json::from_value(json).unwrap();
    assert!(parsed.columns.is_empty());
    let CypherResponseData::Records(records) = parsed.data else {
        panic!("expected records");
    };
    assert!(records[0].values.is_empty());
}
