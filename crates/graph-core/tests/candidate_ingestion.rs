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
use graph_core::*;
fn input(raw: &str) -> CandidateInput {
    CandidateInput::new(
        "candidate--1",
        ExtractionRunId::new("run--1").expect("run"),
        raw,
        ActorId::new("actor--extractor").expect("actor"),
    )
    .expect("input")
}
#[test]
fn submission_preserves_raw_bytes_and_never_materializes_canonical_records() {
    let mut graph = Graph::new();
    let raw = " { \"name\": \"évidence\" } \n";
    let candidate = graph.submit_candidate(input(raw)).expect("submit");
    assert_eq!(candidate.raw_payload(), raw);
    assert_eq!(candidate.extraction_run_id().as_str(), "run--1");
    assert_eq!(
        graph.epistemic_stores().candidates.tier_of(candidate.id()),
        Some(GraphTier::Shadow)
    );
    assert!(graph.list_nodes().expect("nodes").is_empty());
    assert!(
        graph
            .list_relationships()
            .expect("relationships")
            .is_empty()
    );
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
}
#[test]
fn submission_is_idempotent_and_conflicting_raw_payload_cannot_rewrite_history() {
    let mut graph = Graph::new();
    graph.submit_candidate(input("original")).expect("submit");
    let before = graph.epistemic_stores().clone();
    graph.submit_candidate(input("original")).expect("replay");
    assert!(graph.submit_candidate(input("changed")).is_err());
    assert_eq!(graph.epistemic_stores(), &before);
    assert!(
        graph
            .submit_candidate(input("original").with_tier(GraphTier::Canonical))
            .is_err()
    );
}
#[test]
fn promotion_is_explicit_audited_and_does_not_replace_raw_candidate() {
    let mut graph = Graph::new();
    let candidate = graph.submit_candidate(input("original")).expect("submit");
    let promoted = graph
        .promote_candidate(
            candidate.id(),
            ActorId::new("actor--reviewer").expect("actor"),
            "reviewed",
            CandidatePromotionInput::Node(
                NodeInput::new(["Entity"])
                    .with_property("name", PropertyValue::String("reviewed name".into())),
            ),
        )
        .expect("promote");
    assert_eq!(graph.list_nodes().expect("nodes").len(), 1);
    assert_eq!(
        graph.epistemic_stores().candidates.tier_of(candidate.id()),
        Some(GraphTier::Canonical)
    );
    assert_eq!(promoted.actor().as_str(), "actor--reviewer");
    assert_eq!(promoted.reason(), "reviewed");
    assert_eq!(
        graph
            .epistemic_stores()
            .candidates
            .get(candidate.id())
            .expect("candidate")
            .raw_payload(),
        "original"
    );
    assert_eq!(
        graph.list_nodes().expect("nodes")[0]
            .extraction_run_id()
            .expect("run")
            .as_str(),
        "run--1"
    );
    let json = serde_json::to_value(graph.persistence_snapshot()).expect("serialize");
    let restored = Graph::from_persistence_snapshot(serde_json::from_value(json).expect("decode"))
        .expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
}
#[test]
fn failed_promotion_is_atomic_and_hypothesis_is_an_allowed_landing_tier() {
    let mut graph = Graph::new();
    let candidate = graph
        .submit_candidate(input("proposal").with_tier(GraphTier::Hypothesis))
        .expect("submit");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    assert!(
        graph
            .promote_candidate(
                candidate.id(),
                ActorId::new("actor--reviewer").expect("actor"),
                "",
                CandidatePromotionInput::Node(NodeInput::new(["Entity"]))
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot"),
        before
    );
    assert_eq!(
        graph.epistemic_stores().candidates.tier_of(candidate.id()),
        Some(GraphTier::Hypothesis)
    );
}

#[test]
fn restoration_preserves_interleaved_audit_order_and_rejects_tampering() {
    let mut graph = Graph::new();
    let first = graph.submit_candidate(input("first")).expect("submit");
    graph
        .promote_candidate(
            first.id(),
            ActorId::new("reviewer").expect("reviewer ID"),
            "checked",
            CandidatePromotionInput::Node(NodeInput::new(["Entity"])),
        )
        .expect("promote");
    graph
        .submit_candidate(
            CandidateInput::new(
                "candidate--2",
                ExtractionRunId::new("run--2").expect("run ID"),
                "second",
                ActorId::new("extractor").expect("extractor ID"),
            )
            .expect("valid input"),
        )
        .expect("submit second");
    let store = &graph.epistemic_stores().candidates;
    let json = serde_json::to_value(store).expect("encode");
    let restored: CandidateStore = serde_json::from_value(json.clone()).expect("restore");
    assert_eq!(&restored, store);
    let mut tampered = json;
    tampered["transitions"][1]["actor_ref"] = serde_json::json!("impostor");
    assert!(serde_json::from_value::<CandidateStore>(tampered).is_err());
}

#[test]
fn missing_relationship_endpoint_does_not_promote_or_mutate_candidate() {
    let mut graph = Graph::new();
    let candidate = graph
        .submit_candidate(input("raw relationship"))
        .expect("submit");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("serialize snapshot");
    let relationship = RelationshipInput::new(
        NodeId::new("missing-source").expect("source ID"),
        "LINKS",
        NodeId::new("missing-target").expect("target ID"),
    )
    .expect("valid input");
    assert!(
        graph
            .promote_candidate(
                candidate.id(),
                ActorId::new("reviewer").expect("reviewer ID"),
                "checked",
                CandidatePromotionInput::Relationship(relationship)
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(graph.persistence_snapshot()).expect("serialize snapshot"),
        before
    );
}
