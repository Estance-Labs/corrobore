// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Compatibility guard for records serialized before Epic 0029 WS-A
//! (issue #153).
//!
//! The fixtures under `compatibility/epistemic/v1/` were captured from the
//! pre-WS-A serialization of claims, claim links, evidence records, and the
//! graph persistence snapshot. Each one must deserialize with the current
//! types and serialize back to exactly the same JSON, so stored payloads stay
//! byte-stable across the workstream.
use std::path::PathBuf;

use graph_core::{Claim, ClaimLink, EvidenceRecord, GraphPersistenceSnapshot};

fn fixture(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../compatibility/epistemic/v1")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_slice(&bytes).expect("fixture should be JSON")
}

fn assert_round_trip<T>(name: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let original = fixture(name);
    let decoded: T = serde_json::from_value(original.clone())
        .unwrap_or_else(|error| panic!("{name} should deserialize: {error}"));
    let encoded = serde_json::to_value(&decoded).expect("re-serialization should succeed");
    assert_eq!(encoded, original, "{name} must round-trip unchanged");
}

#[test]
fn pre_ws_a_claim_round_trips_unchanged() {
    assert_round_trip::<Claim>("claim.json");
}

#[test]
fn pre_ws_a_claim_link_round_trips_unchanged() {
    assert_round_trip::<ClaimLink>("claim_link.json");
}

#[test]
fn pre_ws_a_evidence_record_round_trips_unchanged() {
    assert_round_trip::<EvidenceRecord>("evidence_record.json");
}

#[test]
fn pre_ws_a_graph_persistence_snapshot_round_trips_unchanged() {
    // The snapshot fixture predates the `epistemic` key; empty stores are
    // skipped on serialization so the payload stays identical.
    assert_round_trip::<GraphPersistenceSnapshot>("graph_persistence_snapshot.json");
}
