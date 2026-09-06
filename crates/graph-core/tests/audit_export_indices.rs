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
//! Scoped archives must preserve source ledger indices without retaining unrelated links.
use graph_core::*;
use serde_json::json;
fn resolve(graph: &mut Graph, claim: &ClaimId) {
    let stores = graph.epistemic_stores_mut();
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    resolve_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        claim,
        BitemporalStamp::new(
            TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("valid audit fixture or archive"),
            TemporalTimestamp::new("2026-09-06T12:00:01Z").expect("valid audit fixture or archive"),
        )
        .expect("valid audit fixture or archive"),
        "ws-a-minimal-v1",
    )
    .expect("valid audit fixture or archive");
}
#[test]
fn sparse_link_ledger_restores_identical_audit_and_resolves_with_stable_clusters() {
    let mut graph = Graph::new();
    let root = ClaimId::new("root").expect("valid audit fixture or archive");
    for name in ["excluded", "root"] {
        let claim = ClaimId::new(name).expect("valid audit fixture or archive");
        let observation = ObservationId::new(format!("observation-{name}"))
            .expect("valid audit fixture or archive");
        let stores = graph.epistemic_stores_mut();
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim.clone(),
                ClaimStatement::new(name).expect("valid audit fixture or archive"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(name, None)),
            ))
            .expect("valid audit fixture or archive");
        stores.claims.register_observation(observation.clone());
        stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim,
                ClaimLinkKind::Supports,
            ))
            .expect("valid audit fixture or archive");
    }
    resolve(&mut graph, &root);
    let expected = graph
        .claim_audit_path(&root)
        .expect("valid audit fixture or archive");
    let mut snapshot =
        serde_json::to_value(graph.persistence_snapshot()).expect("valid audit fixture or archive");
    snapshot["epistemic"]["claims"]["claim_links"]
        .as_array_mut()
        .expect("array")
        .remove(0);
    snapshot["epistemic"]["claims"]["claim_link_indices"] = json!([1]);
    let mut restored = Graph::from_persistence_snapshot(
        serde_json::from_value(snapshot).expect("valid audit fixture or archive"),
    )
    .expect("valid audit fixture or archive");
    assert_eq!(
        restored
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive"),
        expected
    );
    resolve(&mut restored, &root);
    assert_eq!(
        restored
            .claim_audit_path(&root)
            .expect("valid audit fixture or archive")["explanation"]["clusters"],
        expected["explanation"]["clusters"]
    );
    let other = ObservationId::new("new-observation").expect("valid audit fixture or archive");
    restored
        .epistemic_stores_mut()
        .claims
        .register_observation(other.clone());
    restored
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(other),
            root.clone(),
            ClaimLinkKind::Refutes,
        ))
        .expect("valid audit fixture or archive");
    let audit = restored
        .claim_audit_path(&root)
        .expect("valid audit fixture or archive");
    assert_eq!(
        audit["link_membership"]
            .as_array()
            .expect("array")
            .iter()
            .map(|m| m["store_index"].as_u64().expect("ledger index"))
            .collect::<std::collections::BTreeSet<_>>(),
        [1, 2].into_iter().collect()
    );
}
#[test]
fn malformed_sparse_link_indices_are_rejected_on_restore() {
    let mut snapshot = serde_json::to_value(Graph::new().persistence_snapshot())
        .expect("valid audit fixture or archive");
    snapshot["epistemic"] = json!({"claims":{"claim_link_indices":[1]}});
    assert!(
        Graph::from_persistence_snapshot(
            serde_json::from_value(snapshot).expect("valid audit fixture or archive")
        )
        .is_err()
    );
}
