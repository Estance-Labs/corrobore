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
use graph_core::{
    ClaimAnalyticalTarget, ClaimConfidenceTarget, ClaimEvidenceTargetRef, ClaimSourceTargetRef,
    ClaimTarget, ClaimTargetKind, ClaimTargetMetadata, ClaimTargetValidationContext,
    ClaimTemporalTarget, GraphError, NodeId, RelationshipId,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

//
// Verify that claim targets can represent the bounded target vocabulary required
// by issue #88 without falling back to unstructured payloads.
#[test]
fn claim_target_variants_cover_required_target_kinds() {
    let node_target = ClaimTarget::Node(node_id("node--campaign-1"));
    let relationship_target = ClaimTarget::Relationship(relationship_id("relationship--1"));
    let evidence_target = ClaimTarget::Evidence(ClaimEvidenceTargetRef::new("evidence://doc/42"));
    let source_target = ClaimTarget::Source(ClaimSourceTargetRef::new("source://intel/report-9"));
    let temporal_target = ClaimTarget::TemporalAssertion(ClaimTemporalTarget::new(
        "observed_at",
        "2026-07-06T10:00:00Z",
    ));
    let confidence_target =
        ClaimTarget::ConfidenceAssertion(ClaimConfidenceTarget::new("entity-link", 0.82));
    let analytical_target = ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
        "Competing attribution hypothesis",
        Some("analysis/branch-a".to_owned()),
    ));

    assert_eq!(node_target.kind(), ClaimTargetKind::Node);
    assert_eq!(relationship_target.kind(), ClaimTargetKind::Relationship);
    assert_eq!(evidence_target.kind(), ClaimTargetKind::Evidence);
    assert_eq!(source_target.kind(), ClaimTargetKind::Source);
    assert_eq!(temporal_target.kind(), ClaimTargetKind::TemporalAssertion);
    assert_eq!(
        confidence_target.kind(),
        ClaimTargetKind::ConfidenceAssertion
    );
    assert_eq!(
        analytical_target.kind(),
        ClaimTargetKind::AnalyticalAssertion
    );
}

//
// Verify that reference-based targets return explicit missing-target errors when
// the referenced record is absent from the validation context.
#[test]
fn validate_references_returns_claim_target_not_found_for_missing_references() {
    let context = ClaimTargetValidationContext::new();
    let target = ClaimTarget::Node(node_id("node--missing"));

    let error = target
        .validate_references(&context)
        .expect_err("missing node target should produce typed target-not-found error");

    assert!(matches!(error, GraphError::ClaimTargetNotFound(missing) if missing == target));
}

//
// Verify that target validation accepts references that are present in the
// explicit context and that metadata resolution remains deterministic.
#[test]
fn validate_references_accepts_present_targets_and_resolves_metadata() {
    let node = node_id("node--present");
    let relationship = relationship_id("relationship--present");
    let mut context = ClaimTargetValidationContext::new();
    context.register_node(node.clone());
    context.register_relationship(relationship.clone());
    context.register_evidence("evidence://doc/42");
    context.register_source("source://intel/report-9");

    let cases = [
        ClaimTarget::Node(node),
        ClaimTarget::Relationship(relationship),
        ClaimTarget::Evidence(ClaimEvidenceTargetRef::new("evidence://doc/42")),
        ClaimTarget::Source(ClaimSourceTargetRef::new("source://intel/report-9")),
        ClaimTarget::TemporalAssertion(ClaimTemporalTarget::new(
            "valid_from",
            "2026-01-01T00:00:00Z",
        )),
        ClaimTarget::ConfidenceAssertion(ClaimConfidenceTarget::new("source-score", 0.44)),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
            "Uncertain but plausible analytical linkage",
            None,
        )),
    ];

    for target in cases {
        target
            .validate_references(&context)
            .expect("target should validate with present context");

        let metadata = target
            .resolve_target_metadata(&context)
            .expect("metadata resolution should succeed when target is valid");

        assert_eq!(metadata.kind, target.kind());
    }
}

//
// Verify that unknown target kinds are represented explicitly and rejected with
// a dedicated unsupported-kind error.
#[test]
fn validate_references_rejects_unsupported_target_kind() {
    let target = ClaimTarget::Unsupported {
        kind: "custom-json-pointer".to_owned(),
        raw_reference: "#/claims/0/target".to_owned(),
    };

    let error = target
        .validate_references(&ClaimTargetValidationContext::new())
        .expect_err("unsupported target kind should fail with typed error");

    assert!(matches!(
    error,
    GraphError::UnsupportedClaimTargetKind(kind) if kind == "custom-json-pointer"
    ));
}

//
// Verify the metadata value object shape expected by downstream working-set and
// storage layers.
#[test]
fn target_metadata_exposes_kind_and_stable_reference() {
    let metadata = ClaimTargetMetadata {
        kind: ClaimTargetKind::Source,
        stable_reference: Some("source://intel/report-9".to_owned()),
    };

    assert_eq!(metadata.kind, ClaimTargetKind::Source);
    assert_eq!(
        metadata.stable_reference.as_deref(),
        Some("source://intel/report-9")
    );
}
