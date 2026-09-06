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
//! Audit archives restore retained provenance without exporting unrelated claims.
use graph_core::*;
#[path = "support/ingestion_quality.rs"]
mod fixtures;
fn fixture() -> (Graph, ClaimId) {
    let mut graph = fixtures::seeded();
    let root = ClaimId::new("archive-root").expect("valid audit fixture or archive");
    for name in ["secret-unrelated-claim", "archive-root"] {
        let id = ClaimId::new(name).expect("valid audit fixture or archive");
        let stores = graph.epistemic_stores_mut();
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                id.clone(),
                ClaimStatement::new(name).expect("valid audit fixture or archive"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(name, None)),
            ))
            .expect("valid audit fixture or archive");
        let obs = ObservationId::new(if name == "archive-root" {
            "observation--left-0"
        } else {
            "observation--left-4"
        })
        .expect("valid audit fixture or archive");
        stores.claims.register_observation(obs.clone());
        stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(obs),
                id,
                ClaimLinkKind::Supports,
            ))
            .expect("valid audit fixture or archive");
    }
    graph
        .link_claim_audit_record(
            &root,
            ClaimAuditReference::Candidate(
                CandidateId::new("repair-1").expect("valid audit fixture or archive"),
            ),
        )
        .expect("valid audit fixture or archive");
    graph
        .link_claim_audit_record(
            &root,
            ClaimAuditReference::Reconciliation(
                ReconciliationRecordId::new("decision-0").expect("valid audit fixture or archive"),
            ),
        )
        .expect("valid audit fixture or archive");
    graph
        .record_analyst_decision(
            AnalystDecision::new(
                "analyst-note",
                root.clone(),
                ActorId::new("analyst").expect("valid audit fixture or archive"),
                TemporalTimestamp::new("2026-09-06T12:00:00Z")
                    .expect("valid audit fixture or archive"),
                AnalystDecisionAction::Annotation {
                    text: "Human review".into(),
                },
            )
            .expect("valid audit fixture or archive"),
        )
        .expect("valid audit fixture or archive");
    graph
        .record_analyst_decision(
            AnalystDecision::new(
                "withdraw-note",
                root.clone(),
                ActorId::new("analyst").expect("valid audit fixture or archive"),
                TemporalTimestamp::new("2026-09-06T13:00:00Z")
                    .expect("valid audit fixture or archive"),
                AnalystDecisionAction::Reversal {
                    decision_id: "analyst-note".into(),
                    rationale: "Reconsidered".into(),
                },
            )
            .expect("valid audit fixture or archive"),
        )
        .expect("valid audit fixture or archive");
    let merge = ReconciliationRecordId::new("decision-0").expect("valid audit fixture or archive");
    graph
        .apply_reconciliation_merge(&merge)
        .expect("valid audit fixture or archive");
    graph
        .undo_reconciliation_merge(
            MergeUndo::new(
                "withdraw-merge",
                merge,
                ActorId::new("reviewer").expect("valid audit fixture or archive"),
                TemporalTimestamp::new("2026-09-06T13:00:00Z")
                    .expect("valid audit fixture or archive"),
                "Identity review",
            )
            .expect("valid audit fixture or archive"),
        )
        .expect("valid audit fixture or archive");
    (graph, root)
}
#[test]
fn scoped_archive_preserves_audit_and_human_records_without_unrelated_material() {
    let (graph, root) = fixture();
    let before = graph
        .claim_audit_path(&root)
        .expect("valid audit fixture or archive");
    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("valid audit fixture or archive");
    assert!(!archive.to_string().contains("secret-unrelated-claim"));
    assert!(!archive.to_string().contains("repair-3"));
    assert_eq!(
        archive,
        graph
            .export_claim_audit_archive(std::slice::from_ref(&root))
            .expect("valid audit fixture or archive")
    );
    let restored =
        Graph::from_claim_audit_archive(&archive).expect("valid audit fixture or archive");
    assert_eq!(
        before,
        restored
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive")
    );
    assert_eq!(restored.epistemic_stores().claims.claims().len(), 1);
    assert_eq!(
        restored
            .epistemic_stores()
            .analyst_decisions
            .records_for_claim(&root)
            .len(),
        2
    );
}
#[test]
fn memory_export_round_trips_and_ungoverned_bytes_match_the_native_snapshot() {
    let mut bare = Graph::new();
    bare.create_node(
        NodeInput::new(["Entity"])
            .with_property("name", PropertyValue::String("legacy memory".into())),
    )
    .expect("valid audit fixture or archive");
    assert_eq!(
        bare.export_memory_json()
            .expect("valid audit fixture or archive"),
        serde_json::to_string(&bare.persistence_snapshot())
            .expect("valid audit fixture or archive")
    );
    let (graph, root) = fixture();
    let bytes = graph
        .export_memory_json()
        .expect("valid audit fixture or archive");
    let restored = Graph::from_memory_json(&bytes).expect("valid audit fixture or archive");
    assert_eq!(
        graph
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive"),
        restored
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive")
    );
}

#[test]
fn archive_rejects_mismatched_views_unknown_schema_and_fails_serialization_on_bad_bindings() {
    let (mut graph, root) = fixture();
    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("valid audit fixture or archive");
    let mut forged = archive.clone();
    forged["audits"][root.as_str()]["claim"]["statement"]["text"] =
        serde_json::json!("forged view");
    assert!(Graph::from_claim_audit_archive(&forged).is_err());
    forged = archive.clone();
    forged["schema"] = serde_json::json!("unknown-v2");
    assert!(Graph::from_claim_audit_archive(&forged).is_err());
    let bundle = serde_json::json!({"x_corrobore_audit_archive":archive});
    assert_eq!(
        Graph::from_exported_audit_bundle(&bundle)
            .expect("valid audit fixture or archive")
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive"),
        graph
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive")
    );
    let target = graph
        .epistemic_stores()
        .claims
        .claim_by_id(&root)
        .expect("valid audit fixture or archive")
        .target()
        .clone();
    graph.epistemic_stores_mut().audit_bindings = serde_json::from_value(serde_json::json!({"links":[[root, ClaimAuditReference::Candidate(CandidateId::new("missing").expect("valid audit fixture or archive"))]]})).expect("valid audit fixture or archive");
    assert!(serde_json::to_value(graph.audit_archive_for_export_targets(&[target])).is_err());
}

#[test]
fn archive_keeps_required_merge_dependencies_without_binding_them_to_the_claim() {
    let mut graph = fixtures::seeded();
    let merge = ReconciliationRecordId::new("decision-0").expect("valid audit fixture or archive");
    graph
        .apply_reconciliation_merge(&merge)
        .expect("valid audit fixture or archive");
    let dependent = graph
        .record_reconciliation(fixtures::input(
            "dependent",
            &EntityMentionId::new("left-0").expect("valid audit fixture or archive"),
            &EntityMentionId::new("right-1").expect("valid audit fixture or archive"),
            ReconciliationOutcome::Distinct,
            ReconciliationFeature::SourceContext,
        ))
        .expect("valid audit fixture or archive");
    let root = ClaimId::new("dependent-claim").expect("valid audit fixture or archive");
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            root.clone(),
            ClaimStatement::new("Dependent judgment").expect("valid audit fixture or archive"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("subject", None)),
        ))
        .expect("valid audit fixture or archive");
    graph
        .link_claim_audit_record(&root, ClaimAuditReference::Reconciliation(dependent))
        .expect("valid audit fixture or archive");
    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("valid audit fixture or archive");
    let restored =
        Graph::from_claim_audit_archive(&archive).expect("valid audit fixture or archive");
    assert_eq!(
        graph
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive"),
        restored
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive")
    );
    assert!(restored.epistemic_stores().merges.is_active(&merge));
}
