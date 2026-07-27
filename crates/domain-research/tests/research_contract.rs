// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Consumer-level contract for the RESEARCH public surface.

use domain_research::{
    CitationRecord, CitationStance, IdentifierSystem, ReplicationAttemptRecord, ReplicationOutcome,
    ReplicationState, ReproducibilityArtifacts, ResearchNodeRecord, ResearchNodeType,
    RetractionOverride, SignalPresence, SupportingWork, research_citation_stance,
    research_claim_attribution, research_identifier_is_valid, research_identifier_normalize,
    research_replication_status, research_reproducibility_signals, research_support_count,
    validate_research_citation, validate_research_node,
};

#[test]
fn public_surface_exposes_no_quality_or_authority_score() {
    // The pack surfaces evidence and structure. Bibliometric ranking is an
    // explicit non-goal, so no scoring built-in may be added. This test reads
    // the crate source as the enforcement point.
    let sources = [
        include_str!("../src/lib.rs"),
        include_str!("../src/export.rs"),
        include_str!("../src/identifier.rs"),
        include_str!("../src/provider_abi.rs"),
    ];

    let forbidden = [
        "pub fn research_authority_score",
        "pub fn research_quality_score",
        "pub fn research_prestige",
        "pub fn research_impact_factor",
        "pub fn research_h_index",
        "pub fn research_rank",
    ];

    for source in sources {
        for symbol in forbidden {
            assert!(
                !source.contains(symbol),
                "the public surface must not expose {symbol}"
            );
        }
    }
}

#[test]
fn reproducibility_signals_are_observations_not_a_score() {
    let record = ResearchNodeRecord::new(ResearchNodeType::Finding).with_reproducibility(
        ReproducibilityArtifacts {
            dataset_refs: vec!["dataset--1".to_owned()],
            code_refs: Vec::new(),
            method_ref: None,
        },
    );

    let signals = research_reproducibility_signals(&record);
    // Present artifacts are Present, absent ones are Absent. Nothing is folded
    // into a single number a consumer could mistake for a verdict.
    assert_eq!(signals.dataset, SignalPresence::Present);
    assert_eq!(signals.code, SignalPresence::Absent);
    assert_eq!(signals.method, SignalPresence::Absent);
}

#[test]
fn untyped_citation_is_stored_as_cites_and_not_counted_as_support() {
    let citations = vec![
        CitationRecord::new("work--a", "work--z"),
        CitationRecord::new("work--b", "work--z").with_stance(CitationStance::Supports),
    ];

    assert_eq!(
        research_citation_stance(&citations[0]),
        CitationStance::Cites
    );
    assert_eq!(research_support_count(&citations), 1);
}

#[test]
fn stance_bearing_citation_needs_a_locator_and_evidence_to_validate() {
    let incomplete = CitationRecord::new("work--a", "work--z")
        .with_stance(CitationStance::Refutes)
        .intended_validated();
    assert!(!validate_research_citation(&incomplete).is_valid());

    let complete = incomplete
        .with_locator("section 3")
        .with_evidence_ref("evidence--1");
    assert!(validate_research_citation(&complete).is_valid());
}

#[test]
fn claim_attribution_is_enforced_and_reported() {
    let orphan = ResearchNodeRecord::new(ResearchNodeType::Claim);
    assert!(!validate_research_node(&orphan).is_valid());
    assert!(research_claim_attribution(&orphan).is_none());

    let attributed = ResearchNodeRecord::new(ResearchNodeType::Claim)
        .with_attribution("publication--1", "person--1");
    assert!(validate_research_node(&attributed).is_valid());
    assert_eq!(
        research_claim_attribution(&attributed)
            .unwrap()
            .credited_actor,
        "person--1"
    );
}

#[test]
fn retracted_support_requires_a_recorded_override() {
    let claim = ResearchNodeRecord::new(ResearchNodeType::Claim)
        .with_attribution("publication--1", "person--1")
        .with_evidence_ref("evidence--1")
        .with_supporting_work(SupportingWork::new("publication--retracted").retracted())
        .intended_validated();
    assert!(!validate_research_node(&claim).is_valid());

    let overridden = ResearchNodeRecord::new(ResearchNodeType::Claim)
        .with_attribution("publication--1", "person--1")
        .with_evidence_ref("evidence--1")
        .with_supporting_work(
            SupportingWork::new("publication--retracted")
                .retracted()
                .with_override(RetractionOverride {
                    justification: "cited for its method, not its retracted result".to_owned(),
                    recorded_by: "reviewer--1".to_owned(),
                }),
        )
        .intended_validated();
    assert!(validate_research_node(&overridden).is_valid());
}

#[test]
fn replication_status_is_available_to_consumers() {
    let record = ResearchNodeRecord::new(ResearchNodeType::Finding)
        .with_result_ref("result--1")
        .with_replication_attempt(ReplicationAttemptRecord {
            target_work: "publication--1".to_owned(),
            reporting_work: "publication--2".to_owned(),
            outcome: ReplicationOutcome::Failed,
        });
    assert!(validate_research_node(&record).is_valid());
    assert_eq!(
        research_replication_status(&record).state,
        ReplicationState::FailedToReplicate
    );
}

#[test]
fn identifier_normalization_is_offline_and_deterministic() {
    assert_eq!(
        research_identifier_normalize(IdentifierSystem::Doi, "https://doi.org/10.1000/ABC"),
        Some("10.1000/abc".to_owned())
    );
    assert!(research_identifier_is_valid(
        IdentifierSystem::Orcid,
        "0000-0002-1825-0097"
    ));
    assert!(!research_identifier_is_valid(
        IdentifierSystem::Orcid,
        "0000-0002-1825-0098"
    ));
}
