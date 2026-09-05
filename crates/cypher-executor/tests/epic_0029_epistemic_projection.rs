// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Cypher read projection of Epic 0029 governed records (WS-A item 7,
//! issue #153): claims, sources, observations, verdicts, verification records,
//! and state transitions are readable through ordinary `MATCH` over the
//! epistemic projection graph, so the HTTP `/v1/cypher/read` route inherits the
//! same view once the runtime serves the projection.
#![allow(clippy::unwrap_used)]

use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy, ExecutionResultData};
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimTarget, EpistemicStores, EvidenceRecordStore,
    EvidenceSourceType, Graph, ObservationId, ObservationInput, ObservationModality,
    ResolutionInputs, SourceId, SourceInput, TemporalTimestamp, resolve_claim_verdict,
};

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).unwrap()
}

fn projection() -> Graph {
    let mut stores = EpistemicStores::default();
    stores
        .sources
        .register_source(SourceInput::new(
            SourceId::new("source--report").unwrap(),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    for (index, kind) in [ClaimLinkKind::Supports, ClaimLinkKind::Refutes]
        .into_iter()
        .enumerate()
    {
        let observation = ObservationId::new(format!("observation--{index}")).unwrap();
        stores
            .observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    SourceId::new("source--report").unwrap(),
                    format!("span {index}"),
                    ObservationModality::Text,
                ),
                &stores.sources,
            )
            .unwrap();
        let claim = ClaimId::new(format!("claim--{index}")).unwrap();
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim.clone(),
                ClaimStatement::new(format!("claim {index}")).unwrap(),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("x", None)),
            ))
            .unwrap();
        stores.claims.register_observation(observation.clone());
        stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim.clone(),
                kind,
            ))
            .unwrap();
        let evidence = EvidenceRecordStore::new();
        let mut claims = std::mem::take(&mut stores.claims);
        let mut verdicts = std::mem::take(&mut stores.verdicts);
        resolve_claim_verdict(
            &mut claims,
            &mut verdicts,
            &ResolutionInputs::new(
                &stores.verifications,
                &evidence,
                &stores.observations,
                &stores.sources,
            ),
            &claim,
            BitemporalStamp::new(
                ts("2026-08-01T00:00:00Z"),
                ts(&format!("2026-08-30T10:0{index}:00Z")),
            )
            .unwrap(),
            "ws-a-minimal-v1",
        )
        .unwrap();
        stores.claims = claims;
        stores.verdicts = verdicts;
    }
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);
    graph.epistemic_projection().unwrap()
}

fn records(query: &str) -> Vec<std::collections::HashMap<String, serde_json::Value>> {
    let mut executor =
        CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), projection());
    let result = executor.execute(query).expect("read should execute");
    let ExecutionResultData::Records(records) = result.data else {
        panic!("expected records");
    };
    records
        .into_iter()
        .map(|record| {
            record
                .fields
                .into_iter()
                .map(|(key, value)| (key, serde_json::to_value(value).unwrap()))
                .collect()
        })
        .collect()
}

#[test]
fn claims_expose_their_verdict_state_and_lifecycle_projection_through_match() {
    let rows = records(
        "MATCH (c:Claim) RETURN c.claim_id, c.verdict_state, c.claim_status ORDER BY c.claim_id ASC",
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["c.claim_id"], "claim--0");
    assert_eq!(rows[0]["c.verdict_state"], "supported");
    assert_eq!(rows[0]["c.claim_status"], "supported");
    assert_eq!(rows[1]["c.verdict_state"], "refuted");
    assert_eq!(rows[1]["c.claim_status"], "contradicted");
}

#[test]
fn verdicts_and_sources_are_readable_as_vocabulary_nodes() {
    let verdicts = records(
        "MATCH (v:Verdict) RETURN v.verdict_claim, v.verdict_state ORDER BY v.verdict_claim ASC",
    );
    assert_eq!(verdicts.len(), 2);
    assert_eq!(verdicts[1]["v.verdict_state"], "refuted");

    let sources = records("MATCH (s:Source) RETURN s.source_uri");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]["s.source_uri"],
        "https://vendor.example/report.pdf"
    );

    let observations =
        records("MATCH (o:Observation) RETURN o.observation_source ORDER BY o.observation_id ASC");
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0]["o.observation_source"], "source--report");

    let transitions = records(
        "MATCH (t:StateTransition) RETURN t.transition_to_state ORDER BY t.transition_to_state ASC",
    );
    assert_eq!(transitions.len(), 2);
    assert_eq!(transitions[0]["t.transition_to_state"], "refuted");
}
