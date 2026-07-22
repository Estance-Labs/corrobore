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
    ActorId, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement, ClaimStatus, ClaimStore,
    ClaimTarget, Confidence, EvidenceId, ExtractionRunId, GraphError, TemporalMetadata,
    WorkspaceId,
};

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence should be valid")
}

fn actor_id(value: &str) -> ActorId {
    ActorId::new(value).expect("test actor ID should be valid")
}

fn workspace_id(value: &str) -> WorkspaceId {
    WorkspaceId::new(value).expect("test workspace ID should be valid")
}

fn extraction_run_id(value: &str) -> ExtractionRunId {
    ExtractionRunId::new(value).expect("test extraction run ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

//
// Verify that the full claim lifecycle status vocabulary is available as typed
// enum variants so callers do not collapse epistemic state into plain booleans
// or ad hoc strings.
#[test]
fn claim_status_exposes_expected_lifecycle_variants() {
    let statuses = [
        ClaimStatus::Candidate,
        ClaimStatus::Asserted,
        ClaimStatus::Supported,
        ClaimStatus::Disputed,
        ClaimStatus::Contradicted,
        ClaimStatus::Superseded,
        ClaimStatus::Retracted,
        ClaimStatus::Rejected,
        ClaimStatus::Validated,
        ClaimStatus::Unresolved,
    ];

    assert_eq!(statuses.len(), 10);
    assert_ne!(ClaimStatus::Candidate, ClaimStatus::Validated);
}

//
// Verify that explicitly invalid transitions are rejected by the typed
// transition checker used by claim lifecycle operations.
#[test]
fn claim_status_transition_rejects_retracted_to_validated() {
    let result =
        ClaimStatus::ensure_valid_transition(ClaimStatus::Retracted, ClaimStatus::Validated);

    assert!(matches!(
    result,
    Err(GraphError::InvalidClaimStatusTransition { from, to })
    if from == ClaimStatus::Retracted && to == ClaimStatus::Validated
    ));
}

//
// Verify that candidate claim creation reserves a stable typed creation
// boundary and persists the record in the in-memory claim store.
#[test]
fn claim_store_creates_candidate_claim() {
    let mut store = ClaimStore::new();
    let claim_id = ClaimId::new("claim--candidate-1").expect("test claim ID should be valid");
    let claim_input = ClaimInput::new(
        claim_id.clone(),
        ClaimStatement::new("Observed actor A communicates with endpoint X")
            .expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
            "Observed actor A communicates with endpoint X",
            None,
        )),
    );

    let created_id = store
        .create_candidate_claim(claim_input)
        .expect("candidate claim creation should succeed");
    let claim = store
        .claim_by_id(&created_id)
        .expect("created claim should be readable");

    assert_eq!(created_id, claim_id);
    assert_eq!(claim.id(), &claim_id);
    assert_eq!(claim.status(), ClaimStatus::Candidate);
}

//
// Verify that asserted claim creation reserves a dedicated typed creation path
// and writes the expected asserted lifecycle status.
#[test]
fn claim_store_creates_asserted_claim() {
    let mut store = ClaimStore::new();
    let claim_id = ClaimId::new("claim--asserted-1").expect("test claim ID should be valid");
    let claim_input = ClaimInput::new(
        claim_id.clone(),
        ClaimStatement::new("Actor A controls infrastructure B")
            .expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
            "Actor A controls infrastructure B",
            None,
        )),
    );

    let created_id = store
        .create_asserted_claim(claim_input)
        .expect("asserted claim creation should succeed");
    let claim = store
        .claim_by_id(&created_id)
        .expect("created claim should be readable");

    assert_eq!(created_id, claim_id);
    assert_eq!(claim.status(), ClaimStatus::Asserted);
}

//
// Verify that missing-claim reads surface the explicit typed branch instead of
// returning string-only errors.
#[test]
fn claim_store_read_missing_claim_returns_claim_not_found() {
    let store = ClaimStore::new();
    let missing = ClaimId::new("claim--missing").expect("test claim ID should be valid");

    let error = store
        .claim_by_id(&missing)
        .expect_err("missing claim should fail with typed error");

    assert!(matches!(error, GraphError::ClaimNotFound(id) if id == missing));
}

//
// Verify that claims carry the required epistemic foundation metadata: typed
// confidence, provenance, temporal slots, evidence references, and version
// metadata.
#[test]
fn claim_records_carry_confidence_provenance_temporal_evidence_and_version_metadata() {
    let mut store = ClaimStore::new();
    let claim_id = ClaimId::new("claim--metadata-1").expect("test claim ID should be valid");
    let created_by = actor_id("actor--analyst-1");
    let workspace = workspace_id("workspace--incident-42");
    let extraction_run = extraction_run_id("extraction-run--batch-1");
    let supporting_evidence = evidence_id("evidence--17");
    let claim_input = ClaimInput::new(
        claim_id.clone(),
        ClaimStatement::new("Infrastructure B is linked to campaign C")
            .expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
            "Infrastructure B is linked to campaign C",
            Some("analysis/incident-42".to_owned()),
        )),
    )
    .with_confidence(confidence(0.82))
    .with_created_by(created_by.clone())
    .with_workspace_id(workspace.clone())
    .with_extraction_run_id(extraction_run.clone())
    .with_source_ref("report://intel/source-22")
    .with_evidence_ref(supporting_evidence.clone())
    .with_temporal(TemporalMetadata {
        created_at: Some("2026-07-06T10:00:00Z".to_owned()),
        observed_at: Some("2026-07-03T18:12:00Z".to_owned()),
        ..TemporalMetadata::default()
    });

    let created_id = store
        .create_asserted_claim(claim_input)
        .expect("asserted claim creation should succeed");
    let claim = store
        .claim_by_id(&created_id)
        .expect("created claim should be readable");

    assert_eq!(claim.id(), &claim_id);
    assert_eq!(claim.version(), 1);
    assert_eq!(
        claim.version_id().as_str(),
        "claim-version--claim--metadata-1--1"
    );
    assert_eq!(claim.confidence(), Some(confidence(0.82)));
    assert_eq!(claim.created_by(), Some(&created_by));
    assert_eq!(claim.workspace_id(), Some(&workspace));
    assert_eq!(claim.extraction_run_id(), Some(&extraction_run));
    assert_eq!(
        claim.source_refs(),
        &["report://intel/source-22".to_owned()]
    );
    assert_eq!(claim.evidence_refs(), &[supporting_evidence]);
    assert_eq!(
        claim.temporal().created_at.as_deref(),
        Some("2026-07-06T10:00:00Z")
    );
    assert_eq!(
        claim.temporal().observed_at.as_deref(),
        Some("2026-07-03T18:12:00Z")
    );
}
