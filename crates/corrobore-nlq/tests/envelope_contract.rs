// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! The `nlq/v1` action envelope and its trust boundary (epic #82, item #260).
//!
//! A model's output is untrusted text until this crate has validated it. The
//! contract under test: exactly one action per envelope, unknown fields
//! refused, trusted runtime context refused wherever it appears, evidence
//! identifiers only from the caller's allow-list, and canonicalization through
//! the real Cypher, `INVESTIGATE` and memory parsers.
#![allow(clippy::unwrap_used)]

use corrobore_nlq::{
    Action, ActionKind, Envelope, Language, RejectionCode, SCHEMA_VERSION, TrustBoundary, validate,
};
use serde_json::json;

fn boundary() -> TrustBoundary {
    TrustBoundary::new(["span--1", "span--2"], false)
}

fn envelope(action: serde_json::Value) -> serde_json::Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "language": "fr",
        "action": action,
        "bounded": {"limit": 50},
        "evidence_refs": [],
        "reason_code": "compiled"
    })
}

#[test]
fn a_bounded_read_canonicalizes_through_the_cypher_parser() {
    let validated = validate(
        &envelope(json!({
            "kind": "cypher_read",
            "query": "  MATCH (a:ThreatActor)   WHERE a.name = 'APT28' RETURN a.name  LIMIT 20 "
        })),
        &boundary(),
    )
    .unwrap();
    assert_eq!(validated.kind(), ActionKind::CypherRead);
    assert_eq!(validated.language(), &Language::Fr);
    // The canonical form is the parser's normalized text, so two phrasings of
    // one query compare equal.
    assert_eq!(
        validated.canonical(),
        "MATCH (a:ThreatActor) WHERE a.name = 'APT28' RETURN a.name LIMIT 20"
    );
    assert!(!validated.writes());
}

#[test]
fn a_read_must_be_bounded_and_read_only() {
    let unbounded = validate(
        &envelope(json!({"kind": "cypher_read", "query": "MATCH (a:ThreatActor) RETURN a"})),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(unbounded.code, RejectionCode::Unbounded);

    let write = validate(
        &envelope(json!({"kind": "cypher_read", "query": "CREATE (a:ThreatActor {name: 'x'})"})),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(write.code, RejectionCode::ReadEmitsWrite);

    let unsupported = validate(
        &envelope(
            json!({"kind": "cypher_read", "query": "MATCH (a) UNWIND a.x AS y RETURN y LIMIT 1"}),
        ),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(unsupported.code, RejectionCode::UnsupportedCypher);
}

#[test]
fn a_write_proposal_parses_as_a_mutation_and_is_never_executed_here() {
    let refused = validate(
        &envelope(json!({
            "kind": "cypher_write_proposal",
            "query": "MATCH (a:ThreatActor {name: 'APT28'}) SET a.tier = 3"
        })),
        &boundary(),
    )
    .unwrap_err();
    // The caller did not allow writes: a proposal is still refused so a model
    // cannot smuggle a mutation into a read-only task.
    assert_eq!(refused.code, RejectionCode::WriteNotAllowed);

    let allowed = TrustBoundary::new(["span--1"], true);
    let validated = validate(
        &envelope(json!({
            "kind": "cypher_write_proposal",
            "query": "MATCH (a:ThreatActor {name: 'APT28'}) SET a.tier = 3"
        })),
        &allowed,
    )
    .unwrap();
    assert_eq!(validated.kind(), ActionKind::CypherWriteProposal);
    assert!(validated.writes());
    // A proposal whose query is actually a read is a mislabelled action.
    let mislabelled = validate(
        &envelope(json!({"kind": "cypher_write_proposal", "query": "MATCH (a:ThreatActor) RETURN a LIMIT 1"})),
        &allowed,
    )
    .unwrap_err();
    assert_eq!(mislabelled.code, RejectionCode::ActionKindMismatch);
}

#[test]
fn an_investigation_canonicalizes_through_the_investigate_parser() {
    let validated = validate(
        &envelope(json!({
            "kind": "investigation",
            "statement": "INVESTIGATE attribution OF Campaign(\"campaign--42\") REQUIRE independent_sources >= 2 RETURN assessment, unknowns"
        })),
        &boundary(),
    )
    .unwrap();
    assert_eq!(validated.kind(), ActionKind::Investigation);
    assert_eq!(
        validated.canonical(),
        "INVESTIGATE attribution OF Campaign(\"campaign--42\") REQUIRE independent_sources >= 2 RETURN assessment, unknowns"
    );

    let unsupported = validate(
        &envelope(json!({"kind": "investigation", "statement": "INVESTIGATE provenance OF Campaign(\"c\") RETURN assessment"})),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(unsupported.code, RejectionCode::UnsupportedInvestigation);
}

#[test]
fn memory_operations_deserialize_into_the_memory_v1_contract() {
    let recall = validate(
        &envelope(json!({
            "kind": "memory_operation",
            "operation": "recall",
            "input": {
                "objective": "infrastructure of APT28",
                "seed_ids": [],
                "limits": {"max_items": 20, "max_depth": 2, "max_payload_bytes": 65536, "max_cost": 500, "timeout_ms": 2000, "supernode_threshold": 1000},
                "page_token": null
            }
        })),
        &boundary(),
    )
    .unwrap();
    assert_eq!(validated_kind(&recall), ActionKind::MemoryOperation);
    assert!(!recall.writes());
    assert!(recall.canonical().starts_with("memory/v1 recall "));

    // A remember carries provenance: every source_id must be allow-listed.
    let mut remember = json!({
        "kind": "memory_operation",
        "operation": "remember",
        "input": {
            "identity_key": null,
            "kind": "observation",
            "schema_version": "v1",
            "content": {"format": "text", "value": "APT28 registered the domain"},
            "provenance": [{"source_id": "span--1", "locator": "p1", "observed_at": null}],
            "confidence": null,
            "valid_from": null, "valid_until": null, "expires_at": null,
            "tags": []
        }
    });
    let refused = validate(&envelope(remember.clone()), &boundary()).unwrap_err();
    assert_eq!(refused.code, RejectionCode::WriteNotAllowed);
    let allowed = TrustBoundary::new(["span--1"], true);
    let validated = validate(&envelope(remember.clone()), &allowed).unwrap();
    assert!(validated.writes());

    remember["input"]["provenance"][0]["source_id"] = json!("span--invented");
    let invented = validate(&envelope(remember), &allowed).unwrap_err();
    assert_eq!(invented.code, RejectionCode::InventedEvidence);

    // An input the contract does not accept is a typed rejection, not a panic.
    let malformed = validate(
        &envelope(
            json!({"kind": "memory_operation", "operation": "recall", "input": {"objective": "x"}}),
        ),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(malformed.code, RejectionCode::InvalidMemoryInput);
    let unknown = validate(
        &envelope(json!({"kind": "memory_operation", "operation": "explode", "input": {}})),
        &boundary(),
    )
    .unwrap_err();
    assert_eq!(unknown.code, RejectionCode::UnknownMemoryOperation);
}

fn validated_kind(validated: &corrobore_nlq::ValidatedEnvelope) -> ActionKind {
    validated.kind()
}

#[test]
fn trusted_runtime_context_is_refused_wherever_it_appears() {
    for (path, value) in [
        ("workspace_id", json!("workspace--other")),
        ("session_id", json!("session--x")),
        ("actor_id", json!("actor--admin")),
        ("agent_id", json!("agent--x")),
        ("permissions", json!(["write"])),
        ("request_id", json!("r")),
        ("correlation_id", json!("c")),
    ] {
        let mut top = envelope(json!({"kind": "abstain", "reason": "no evidence"}));
        top[path] = value.clone();
        let error = validate(&top, &boundary()).unwrap_err();
        assert_eq!(
            error.code,
            RejectionCode::TrustedContextSupplied,
            "{path} at top level"
        );

        let mut nested =
            envelope(json!({"kind": "cypher_read", "query": "MATCH (a) RETURN a LIMIT 1"}));
        nested["action"][path] = value.clone();
        let error = validate(&nested, &boundary()).unwrap_err();
        assert_eq!(
            error.code,
            RejectionCode::TrustedContextSupplied,
            "{path} inside the action"
        );

        let mut deep = envelope(json!({
            "kind": "memory_operation", "operation": "trace",
            "input": {"target": {"kind": "memory", "id": "mem--1"}}
        }));
        deep["action"]["input"][path] = value;
        let error = validate(&deep, &boundary()).unwrap_err();
        assert_eq!(
            error.code,
            RejectionCode::TrustedContextSupplied,
            "{path} inside the input"
        );
    }
}

#[test]
fn the_envelope_is_closed_and_versioned() {
    let mut unknown = envelope(json!({"kind": "abstain", "reason": "no evidence"}));
    unknown["extra"] = json!(1);
    assert_eq!(
        validate(&unknown, &boundary()).unwrap_err().code,
        RejectionCode::UnknownField
    );

    let mut wrong_version = envelope(json!({"kind": "abstain", "reason": "no evidence"}));
    wrong_version["schema_version"] = json!("nlq/v2");
    assert_eq!(
        validate(&wrong_version, &boundary()).unwrap_err().code,
        RejectionCode::UnsupportedSchemaVersion
    );

    let two_kinds =
        envelope(json!({"kind": "abstain", "reason": "x", "query": "MATCH (a) RETURN a LIMIT 1"}));
    assert_eq!(
        validate(&two_kinds, &boundary()).unwrap_err().code,
        RejectionCode::UnknownField
    );

    let missing = json!({"schema_version": SCHEMA_VERSION, "language": "fr"});
    assert_eq!(
        validate(&missing, &boundary()).unwrap_err().code,
        RejectionCode::Malformed
    );

    // Evidence references used by the envelope itself must be allow-listed too.
    let mut leaked = envelope(json!({"kind": "abstain", "reason": "x"}));
    leaked["evidence_refs"] = json!(["span--99"]);
    assert_eq!(
        validate(&leaked, &boundary()).unwrap_err().code,
        RejectionCode::InventedEvidence
    );
}

#[test]
fn clarification_abstain_and_unsupported_are_terminal_typed_results() {
    let clarification = validate(
        &envelope(json!({
            "kind": "clarification_required",
            "question": "Quel APT28 : l'acteur ou la campagne ?",
            "options": ["ThreatActor", "Campaign"]
        })),
        &boundary(),
    )
    .unwrap();
    assert_eq!(clarification.kind(), ActionKind::ClarificationRequired);
    assert!(!clarification.writes());
    assert!(clarification.is_terminal());

    let abstain = validate(
        &envelope(json!({"kind": "abstain", "reason": "no evidence supplied"})),
        &boundary(),
    )
    .unwrap();
    assert!(abstain.is_terminal());
    let unsupported = validate(
        &envelope(json!({"kind": "unsupported", "reason": "shortest path is not available"})),
        &boundary(),
    )
    .unwrap();
    assert!(unsupported.is_terminal());
}

#[test]
fn a_typed_envelope_round_trips_through_serde_and_its_language_tag() {
    let typed = Envelope::new(
        Language::En,
        Action::CypherRead {
            query: "MATCH (n:Indicator) RETURN n LIMIT 10".to_owned(),
        },
        "compiled",
    );
    let json = serde_json::to_value(&typed).unwrap();
    assert_eq!(json["schema_version"], SCHEMA_VERSION);
    assert_eq!(json["language"], "en");
    assert_eq!(json["action"]["kind"], "cypher_read");
    let back: Envelope = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, typed);
    assert_eq!(
        validate(&json, &boundary()).unwrap().canonical(),
        "MATCH (n:Indicator) RETURN n LIMIT 10"
    );
    assert_eq!(
        serde_json::to_value(Language::Other("de".to_owned())).unwrap(),
        "de"
    );
}
