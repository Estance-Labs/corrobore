// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! The committed `compatibility/nlq/v1` artifacts stay in step with the code:
//! every fixture replays through [`validate`] with the recorded outcome, and
//! the JSON schema names exactly the action kinds and rejection codes the
//! crate defines, so an adapter in another language cannot drift silently.
#![allow(clippy::unwrap_used)]

use std::{collections::BTreeSet, fs, path::PathBuf};

use corrobore_nlq::{ActionKind, RejectionCode, SCHEMA_VERSION, TrustBoundary, validate};
use serde_json::Value;

fn artifact(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../compatibility/nlq/v1")
        .join(name);
    serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display())),
    )
    .unwrap()
}

#[test]
fn fixtures_replay_with_their_recorded_outcomes() {
    let fixtures = artifact("fixtures.json");
    assert_eq!(fixtures["schema_version"], SCHEMA_VERSION);
    let boundary = TrustBoundary::new(
        fixtures["boundary"]["allowed_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned()),
        fixtures["boundary"]["allow_writes"].as_bool().unwrap(),
    );
    for case in fixtures["accepted"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let validated = validate(&case["envelope"], &boundary)
            .unwrap_or_else(|error| panic!("{name}: expected acceptance, got {error:?}"));
        assert_eq!(
            serde_json::to_value(validated.kind()).unwrap(),
            case["kind"],
            "{name}"
        );
        assert_eq!(
            validated.canonical(),
            case["canonical"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            validated.writes(),
            case["writes"].as_bool().unwrap(),
            "{name}"
        );
    }
    for case in fixtures["refused"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let rejection = validate(&case["envelope"], &boundary)
            .err()
            .unwrap_or_else(|| panic!("{name}: expected a refusal"));
        assert_eq!(
            serde_json::to_value(rejection.code).unwrap(),
            case["code"],
            "{name}"
        );
    }
}

#[test]
fn the_schema_names_exactly_the_kinds_and_codes_the_crate_defines() {
    let schema = artifact("action-envelope.schema.json");
    assert_eq!(
        schema["properties"]["schema_version"]["const"],
        SCHEMA_VERSION
    );

    let schema_kinds: BTreeSet<String> = schema["properties"]["action"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| {
            variant["properties"]["kind"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let crate_kinds: BTreeSet<String> = [
        ActionKind::MemoryOperation,
        ActionKind::CypherRead,
        ActionKind::CypherWriteProposal,
        ActionKind::Investigation,
        ActionKind::ClarificationRequired,
        ActionKind::Abstain,
        ActionKind::Unsupported,
    ]
    .iter()
    .map(|kind| kind.tag().to_owned())
    .collect();
    assert_eq!(schema_kinds, crate_kinds);

    let schema_codes: BTreeSet<String> = schema["x-corrobore"]["rejection_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code.as_str().unwrap().to_owned())
        .collect();
    let crate_codes: BTreeSet<String> = [
        RejectionCode::Malformed,
        RejectionCode::UnknownField,
        RejectionCode::UnsupportedSchemaVersion,
        RejectionCode::TrustedContextSupplied,
        RejectionCode::InventedEvidence,
        RejectionCode::Unbounded,
        RejectionCode::ReadEmitsWrite,
        RejectionCode::UnsupportedCypher,
        RejectionCode::ActionKindMismatch,
        RejectionCode::WriteNotAllowed,
        RejectionCode::UnsupportedInvestigation,
        RejectionCode::InvalidMemoryInput,
        RejectionCode::UnknownMemoryOperation,
    ]
    .iter()
    .map(|code| {
        serde_json::to_value(code)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    })
    .collect();
    assert_eq!(schema_codes, crate_codes);

    let schema_trusted: BTreeSet<String> =
        schema["x-corrobore"]["trusted_context_keys_refused_at_any_depth"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap().to_owned())
            .collect();
    let crate_trusted: BTreeSet<String> = corrobore_nlq::TRUSTED_CONTEXT_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    assert_eq!(schema_trusted, crate_trusted);

    // No property of the envelope is trusted context, by construction.
    let properties: BTreeSet<String> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert!(properties.is_disjoint(&crate_trusted));
    assert_eq!(schema["additionalProperties"], false);
}
