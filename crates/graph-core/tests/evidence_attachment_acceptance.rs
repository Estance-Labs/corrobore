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
//! acceptance tests for explicit evidence records and attachment contracts.

use graph_core::{
    ClaimId, Confidence, EvidenceAttachmentTarget, EvidenceId, EvidenceInput, EvidenceRecordStore,
    EvidenceSourceType, ExtractionRunId, GraphError, NodeId, RelationshipId, TemporalTimestamp,
};

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn extraction_run_id(value: &str) -> ExtractionRunId {
    ExtractionRunId::new(value).expect("test extraction run ID should be valid")
}

//
// Verify evidence is represented as a first-class explicit record with stable
// ID and source reference metadata.
#[test]
fn evidence_record_is_explicit_and_queryable() {
    let mut store = EvidenceRecordStore::new();
    let evidence_id = evidence_id("evidence--epic-0007-1");

    let created_id = store
        .create_evidence(EvidenceInput::new(
            evidence_id.clone(),
            "source://report/2026-07-06",
            "explicit evidence payload for ",
        ))
        .expect("evidence record should be created");

    assert_eq!(created_id, evidence_id);

    let evidence = store
        .evidence_by_id(&evidence_id)
        .expect("evidence should be queryable by ID");

    assert_eq!(evidence.id(), &evidence_id);
    assert_eq!(evidence.source_ref(), "source://report/2026-07-06");
    assert_eq!(evidence.payload(), "explicit evidence payload for ");
}

//
// Verify evidence attachment contracts support node, relationship, claim, and
// future export record targets.
#[test]
fn evidence_can_be_attached_to_all_epic_0007_target_kinds() {
    let mut store = EvidenceRecordStore::new();
    let evidence_id = evidence_id("evidence--epic-0007-2");
    let node = node_id("node--epic-0007-target");
    let relationship = relationship_id("relationship--epic-0007-target");
    let claim = claim_id("claim--epic-0007-target");

    store
        .create_evidence(EvidenceInput::new(
            evidence_id.clone(),
            "source://report/epic-0007/targets",
            "target attachment coverage",
        ))
        .expect("evidence record should be created");

    store.register_node_target(node.clone());
    store.register_relationship_target(relationship.clone());
    store.register_claim_target(claim.clone());
    store.register_export_record_target("export-record--epic-0007-target");

    store
        .attach_evidence(
            evidence_id.clone(),
            EvidenceAttachmentTarget::node(node.clone()),
        )
        .expect("node attachment should succeed");

    store
        .attach_evidence(
            evidence_id.clone(),
            EvidenceAttachmentTarget::relationship(relationship.clone()),
        )
        .expect("relationship attachment should succeed");

    store
        .attach_evidence(
            evidence_id.clone(),
            EvidenceAttachmentTarget::claim(claim.clone()),
        )
        .expect("claim attachment should succeed");

    store
        .attach_evidence(
            evidence_id,
            EvidenceAttachmentTarget::export_record("export-record--epic-0007-target"),
        )
        .expect("export record attachment should succeed");

    assert_eq!(store.attachments().len(), 4);
}

//
// Verify unknown evidence references are rejected by typed deterministic errors.
#[test]
fn evidence_attachment_rejects_unknown_evidence_id() {
    let mut store = EvidenceRecordStore::new();
    let missing_evidence = evidence_id("evidence--missing");
    let node = node_id("node--epic-0007-missing-evidence");

    store.register_node_target(node.clone());

    let error = store
        .attach_evidence(
            missing_evidence.clone(),
            EvidenceAttachmentTarget::node(node),
        )
        .expect_err("missing evidence should be rejected");

    assert!(matches!(
    error,
    GraphError::EvidenceNotFound(id) if id == missing_evidence
    ));
}

//
// Verify unknown target references are rejected before attachment is recorded.
#[test]
fn evidence_attachment_rejects_unknown_target() {
    let mut store = EvidenceRecordStore::new();
    let evidence_id = evidence_id("evidence--epic-0007-3");
    let missing_claim = claim_id("claim--missing-epic-0007");

    store
        .create_evidence(EvidenceInput::new(
            evidence_id.clone(),
            "source://report/epic-0007/missing-target",
            "target validation coverage",
        ))
        .expect("evidence record should be created");

    let error = store
        .attach_evidence(
            evidence_id,
            EvidenceAttachmentTarget::claim(missing_claim.clone()),
        )
        .expect_err("missing claim target should be rejected");

    assert!(matches!(error, GraphError::ClaimNotFound(id) if id == missing_claim));
}

//
// Verify evidence records preserve extended provenance metadata required by the
// PRD 10.1 evidence primitive contract.
#[test]
fn evidence_record_preserves_extended_provenance_metadata() {
    let mut store = EvidenceRecordStore::new();
    let evidence_id = evidence_id("evidence--epic-0007-provenance");

    store
        .create_evidence(
            EvidenceInput::new(
                evidence_id.clone(),
                "source://report/epic-0007/provenance",
                "provenance payload",
            )
            .with_source_type(EvidenceSourceType::Document)
            .with_chunk_id("chunk-42")
            .with_offsets(12, 48)
            .with_source_url("https://example.org/intel/report-42")
            .with_extraction_run_id(extraction_run_id("extraction-run--42"))
            .with_extractor_id("extractor--nlp-v2")
            .with_model_version("model-v2026-07")
            .with_observed_at(
                TemporalTimestamp::new("2026-07-06T16:00:00Z").expect("timestamp should be valid"),
            )
            .with_language("en")
            .with_source_reliability(Confidence::new(0.82).expect("confidence should be valid"))
            .with_information_credibility(
                Confidence::new(0.74).expect("confidence should be valid"),
            ),
        )
        .expect("evidence record should be created");

    let evidence = store
        .evidence_by_id(&evidence_id)
        .expect("evidence should be queryable by ID");

    assert_eq!(evidence.source_type(), Some(EvidenceSourceType::Document));
    assert_eq!(evidence.chunk_id(), Some("chunk-42"));
    assert_eq!(evidence.offset_start(), Some(12));
    assert_eq!(evidence.offset_end(), Some(48));
    assert_eq!(
        evidence.source_url(),
        Some("https://example.org/intel/report-42")
    );
    assert_eq!(
        evidence.extraction_run_id().map(ExtractionRunId::as_str),
        Some("extraction-run--42")
    );
    assert_eq!(evidence.extractor_id(), Some("extractor--nlp-v2"));
    assert_eq!(evidence.model_version(), Some("model-v2026-07"));
    assert_eq!(
        evidence.observed_at().map(TemporalTimestamp::as_str),
        Some("2026-07-06T16:00:00Z")
    );
    assert_eq!(evidence.language(), Some("en"));
    assert_eq!(
        evidence.source_reliability().map(Confidence::value),
        Some(0.82)
    );
    assert_eq!(
        evidence.information_credibility().map(Confidence::value),
        Some(0.74)
    );
}
