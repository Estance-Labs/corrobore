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
//! Contract for declaring the shape a store or archive actually holds
//! (issue #291).
//!
//! #281 kept inline text under `payload`, so a bundle of inline observations
//! still contains exactly what it always contained and must keep saying so.
//! What changed is that a bundle *can* now hold fields the original shape never
//! described. These tests pin that the newer shape is declared rather than
//! inferred field by field, and that an unknown declaration is refused instead
//! of partially read.
use graph_core::*;
use serde_json::Value;

#[path = "support/ingestion_quality.rs"]
mod fixtures;

const DOCUMENT: &str = "Aster operates the North Relay, the South Relay, and the East Relay, \
     under a contract renewed every winter without public tender.";

/// Place `bytes` under a policy narrow enough that anything is offloaded.
fn offloaded(bytes: &[u8]) -> ContentPlacementDecision {
    let policy = ContentStoragePolicy::new("content-policy-test", 1).expect("policy");
    let mut content = ObjectContentStore::new("memory", MemoryObjectStore::default());
    ingest_content(bytes, Some("text/plain"), &policy, &mut content).expect("ingest")
}

/// A graph whose observations are all inline text no policy placed, which is
/// what every store written before the content plane contains.
fn inline_graph() -> (Graph, ClaimId) {
    let mut graph = fixtures::seeded();
    let root = ClaimId::new("archive-root").expect("claim id");
    let observation = ObservationId::new("observation--left-0").expect("observation id");
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            root.clone(),
            ClaimStatement::new("archive-root").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("archive-root", None)),
        ))
        .expect("claim");
    stores.claims.register_observation(observation.clone());
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation),
            root.clone(),
            ClaimLinkKind::Supports,
        ))
        .expect("link");
    (graph, root)
}

/// The same graph, plus one observation whose content the engine offloaded.
fn offloaded_graph() -> (Graph, ClaimId) {
    let (mut graph, root) = inline_graph();
    let source = SourceId::new("source--offloaded").expect("source id");
    let observation = ObservationId::new("observation--offloaded").expect("observation id");
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    stores
        .observations
        .create_observation(
            ObservationInput::with_placement(
                observation.clone(),
                source,
                offloaded(DOCUMENT.as_bytes()),
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores.claims.register_observation(observation.clone());
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation),
            root.clone(),
            ClaimLinkKind::Supports,
        ))
        .expect("link");
    (graph, root)
}

fn snapshot_json(graph: &Graph) -> Value {
    serde_json::from_str(&graph.export_memory_json().expect("export")).expect("json")
}

// --- the store bundle declares its shape -------------------------------------

#[test]
fn a_store_of_inline_observations_declares_nothing_new() {
    let (graph, _) = inline_graph();

    // Nothing about this bundle changed, so it must not start announcing a
    // shape it does not hold.
    assert!(snapshot_json(&graph).get("epistemic_schema").is_none());
}

#[test]
fn a_store_holding_offloaded_content_declares_the_newer_shape() {
    let (graph, _) = offloaded_graph();

    assert_eq!(
        snapshot_json(&graph)["epistemic_schema"],
        Value::String("corrobore-epistemic-v2".to_owned())
    );
}

#[test]
fn a_store_holding_a_retained_policy_declares_the_newer_shape() {
    let (mut graph, _) = inline_graph();
    let source = SourceId::new("source--inline-policy").expect("source id");
    let policy = ContentStoragePolicy::new("content-policy-test", 4096).expect("policy");
    let mut content = ObjectContentStore::new("memory", MemoryObjectStore::default());
    let decision = ingest_content(b"A short span.", Some("text/plain"), &policy, &mut content)
        .expect("ingest");
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://vendor.example/note.txt",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    stores
        .observations
        .create_observation(
            ObservationInput::with_placement(
                ObservationId::new("observation--inline-policy").expect("observation id"),
                source,
                decision,
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");

    // The content stayed inline, but `content_policy` is still a field the
    // original shape never described.
    assert_eq!(
        snapshot_json(&graph)["epistemic_schema"],
        Value::String("corrobore-epistemic-v2".to_owned())
    );
}

#[test]
fn a_store_reloads_through_the_declared_shape() {
    let (graph, _) = offloaded_graph();

    let json = graph.export_memory_json().expect("export");
    let reopened = Graph::from_memory_json(&json).expect("reopen");

    let observation = reopened
        .epistemic_stores()
        .observations
        .observation_by_id(&ObservationId::new("observation--offloaded").expect("id"))
        .expect("observation");
    assert!(observation.payload_text().is_none());
    assert_eq!(observation.content_policy(), Some("content-policy-test"));
}

#[test]
fn a_store_declaring_an_unknown_shape_is_refused_rather_than_partially_read() {
    let (graph, _) = offloaded_graph();
    let mut json = snapshot_json(&graph);
    json["epistemic_schema"] = Value::String("corrobore-epistemic-v3".to_owned());

    // A newer writer may have used fields this reader would drop in silence.
    let refused = Graph::from_memory_json(&json.to_string());

    assert!(refused.is_err());
}

// --- the audit archive declares its shape ------------------------------------

#[test]
fn an_archive_of_inline_observations_still_declares_v1() {
    let (graph, root) = inline_graph();

    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("archive");

    // A v1 consumer is told v1 because the bytes are what v1 always contained.
    assert_eq!(
        archive["schema"],
        Value::String("corrobore-claim-audit-v1".to_owned())
    );
    // And they really are: an archive of this graph is the old format, not the
    // new one with its additions left out.
    let text = archive.to_string();
    assert!(!text.contains("epistemic_schema"));
    assert!(!text.contains("content_policy"));
    assert!(!text.contains("\"content\""));
}

#[test]
fn an_archive_containing_offloaded_content_declares_v2() {
    let (graph, root) = offloaded_graph();

    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("archive");

    // Continuing to say v1 would tell a consumer something untrue about bytes
    // that now carry a field v1 never described.
    assert_eq!(
        archive["schema"],
        Value::String("corrobore-claim-audit-v2".to_owned())
    );
}

#[test]
fn a_v1_archive_still_imports() {
    let (graph, root) = inline_graph();
    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("archive");

    let restored = Graph::from_claim_audit_archive(&archive).expect("import");

    assert_eq!(
        restored.claim_audit_path(&root).expect("audit"),
        graph.claim_audit_path(&root).expect("audit")
    );
}

#[test]
fn a_v2_archive_round_trips_with_its_offloaded_observation() {
    let (graph, root) = offloaded_graph();
    let archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("archive");

    let restored = Graph::from_claim_audit_archive(&archive).expect("import");

    let observation = restored
        .epistemic_stores()
        .observations
        .observation_by_id(&ObservationId::new("observation--offloaded").expect("id"))
        .expect("observation");
    assert!(observation.payload_text().is_none());
    assert_eq!(observation.content_policy(), Some("content-policy-test"));
    assert_eq!(
        restored.claim_audit_path(&root).expect("audit"),
        graph.claim_audit_path(&root).expect("audit")
    );
}

#[test]
fn an_unknown_future_archive_version_is_refused() {
    let (graph, root) = inline_graph();
    let mut archive = graph
        .export_claim_audit_archive(std::slice::from_ref(&root))
        .expect("archive");
    archive["schema"] = Value::String("corrobore-claim-audit-v3".to_owned());

    let refused = Graph::from_claim_audit_archive(&archive);

    assert!(refused.is_err());
}
