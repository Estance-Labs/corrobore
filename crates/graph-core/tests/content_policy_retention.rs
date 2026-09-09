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
//! Contract for retaining the storage policy that placed an observation's
//! content (issue #296).
//!
//! #294 made the deciding policy available on the returned decision. Nothing
//! wrote it down, so a graph read later could not say whether a span sits
//! inline because it was short or because the threshold was wider that day.
//! These tests pin that the applied policy is part of the record, and that
//! content nobody's policy placed cannot enter one.
use graph_core::{
    ContentHandle, ContentStoragePolicy, ContentStore, EpistemicStores, EvidenceSourceType, Graph,
    MemoryObjectStore, ObjectContentStore, Observation, ObservationId, ObservationInput,
    ObservationModality, ObservationStore, PropertyValue, SourceId, SourceInput, SourceStore,
    ingest_content,
};

const SPAN: &str = "Aster operates the North Relay.";
const DOCUMENT: &str = "Aster operates the North Relay, the South Relay, and the East Relay, \
     under a contract renewed every winter without public tender.";

fn content_store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

/// Wide enough that `SPAN` stays inline and `DOCUMENT` does not.
fn policy(version: &str, inline_max_bytes: u64) -> ContentStoragePolicy {
    ContentStoragePolicy::new(version, inline_max_bytes).expect("policy")
}

fn source_id() -> SourceId {
    SourceId::new("source--report").expect("source id")
}

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("observation id")
}

fn sources() -> SourceStore {
    let mut sources = SourceStore::default();
    sources
        .register_source(SourceInput::new(
            source_id(),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("register");
    sources
}

fn store_with(input: ObservationInput) -> ObservationStore {
    let sources = sources();
    let mut observations = ObservationStore::default();
    observations
        .create_observation(input, &sources)
        .expect("observation");
    observations
}

fn only(store: &ObservationStore) -> &Observation {
    store.observations().first().expect("observation")
}

// --- the applied policy is retained -----------------------------------------

#[test]
fn an_observation_records_the_policy_that_offloaded_its_content() {
    let mut content = content_store();
    let decision = ingest_content(
        DOCUMENT.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 64),
        &mut content,
    )
    .expect("ingest");

    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--document"),
        source_id(),
        decision,
        ObservationModality::Text,
    ));

    assert_eq!(only(&store).content_policy(), Some("content-policy-v1"));
}

#[test]
fn an_observation_records_the_policy_that_kept_its_content_inline() {
    let mut content = content_store();
    let decision = ingest_content(
        SPAN.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 4096),
        &mut content,
    )
    .expect("ingest");

    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--span"),
        source_id(),
        decision,
        ObservationModality::Text,
    ));

    // Inline is a decision too: without the version, nothing distinguishes
    // content that was short from a threshold that was generous.
    assert_eq!(only(&store).content_policy(), Some("content-policy-v1"));
    assert_eq!(only(&store).payload_text(), Some(SPAN));
}

#[test]
fn the_recorded_policy_survives_serialization_and_reload() {
    let mut content = content_store();
    let decision = ingest_content(
        DOCUMENT.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 64),
        &mut content,
    )
    .expect("ingest");
    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--document"),
        source_id(),
        decision,
        ObservationModality::Text,
    ));

    let encoded = serde_json::to_string(&store).expect("serialize");
    let reopened: ObservationStore = serde_json::from_str(&encoded).expect("reopen");

    assert_eq!(only(&reopened).content_policy(), Some("content-policy-v1"));
}

#[test]
fn the_retained_policy_is_the_one_that_decided_not_the_one_in_force_later() {
    let mut content = content_store();
    let decision = ingest_content(
        SPAN.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 4096),
        &mut content,
    )
    .expect("ingest");
    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--span"),
        source_id(),
        decision,
        ObservationModality::Text,
    ));

    // The threshold narrows afterwards. The record explains its own layout; it
    // is not re-derived from whatever policy happens to be current.
    let narrowed = policy("content-policy-v2", 8);
    assert_eq!(
        narrowed.placement(Some("text/plain"), SPAN.len() as u64),
        graph_core::ContentPlacement::Offloaded
    );

    assert_eq!(only(&store).content_policy(), Some("content-policy-v1"));
    assert_eq!(only(&store).payload_text(), Some(SPAN));
}

// --- content no policy placed cannot enter a record -------------------------

#[test]
fn offloaded_content_cannot_enter_a_record_without_a_retained_policy() {
    let mut content = content_store();
    let reference = content
        .store(DOCUMENT.as_bytes(), Some("text/plain"))
        .expect("store");
    let sources = sources();
    let mut observations = ObservationStore::default();

    // A reference to a store nobody was told to write to leaves the record
    // unable to explain why its content lives there.
    let refused = observations.create_observation(
        ObservationInput::with_content(
            observation_id("observation--document"),
            source_id(),
            ContentHandle::External(reference),
            ObservationModality::Text,
        ),
        &sources,
    );

    assert!(refused.is_err());
}

#[test]
fn inline_content_without_a_policy_is_still_accepted() {
    // The ordinary path stays open: short text that never touches a store is
    // not made harder to record than it was before the content plane existed.
    let store = store_with(ObservationInput::new(
        observation_id("observation--span"),
        source_id(),
        SPAN,
        ObservationModality::Text,
    ));

    assert_eq!(only(&store).content_policy(), None);
    assert_eq!(only(&store).payload_text(), Some(SPAN));
}

// --- wire compatibility -----------------------------------------------------

#[test]
fn an_observation_no_policy_placed_is_written_exactly_as_before() {
    let store = store_with(ObservationInput::new(
        observation_id("observation--span"),
        source_id(),
        SPAN,
        ObservationModality::Text,
    ));

    let encoded = serde_json::to_value(only(&store)).expect("serialize");

    // `Observation` goes straight into the claim audit response, so an archive
    // that gained no content plane must gain no field either.
    assert_eq!(encoded["payload"], SPAN);
    assert!(encoded.get("content_policy").is_none());
}

#[test]
fn a_legacy_observation_without_a_policy_still_reads() {
    let legacy = serde_json::json!({
        "id": { "value": "observation--span" },
        "source_id": { "value": "source--report" },
        "selector": null,
        "payload": SPAN,
        "modality": "Text",
        "observed_at": null,
        "payload_sha256": null,
        "supersedes": null,
        "derived_from_legacy": false
    });

    let observation: Observation = serde_json::from_value(legacy).expect("legacy observation");

    assert_eq!(observation.content_policy(), None);
    assert_eq!(observation.payload_text(), Some(SPAN));
}

#[test]
fn a_stored_observation_with_offloaded_content_names_its_policy_on_the_wire() {
    let mut content = content_store();
    let decision = ingest_content(
        DOCUMENT.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 64),
        &mut content,
    )
    .expect("ingest");
    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--document"),
        source_id(),
        decision,
        ObservationModality::Text,
    ));

    let encoded = serde_json::to_value(only(&store)).expect("serialize");

    assert_eq!(encoded["content_policy"], "content-policy-v1");
    assert!(encoded.get("content").is_some());
    assert!(encoded.get("payload").is_none());
}

// --- projection -------------------------------------------------------------

#[test]
fn the_projection_names_the_policy_that_placed_the_content() {
    let mut content = content_store();
    let decision = ingest_content(
        DOCUMENT.as_bytes(),
        Some("text/plain"),
        &policy("content-policy-v1", 64),
        &mut content,
    )
    .expect("ingest");
    let stores = EpistemicStores {
        sources: sources(),
        observations: store_with(ObservationInput::with_placement(
            observation_id("observation--document"),
            source_id(),
            decision,
            ObservationModality::Text,
        )),
        ..EpistemicStores::default()
    };
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);

    let projection = graph.epistemic_projection().expect("projection");
    let node = projection
        .list_nodes()
        .expect("nodes")
        .into_iter()
        .find(|node| node.has_label("Observation"))
        .expect("observation node");

    assert_eq!(
        node.property("observation_content_policy"),
        Some(&PropertyValue::String("content-policy-v1".to_owned()))
    );
}

#[test]
fn the_projection_omits_the_policy_when_none_placed_the_content() {
    let stores = EpistemicStores {
        sources: sources(),
        observations: store_with(ObservationInput::new(
            observation_id("observation--span"),
            source_id(),
            SPAN,
            ObservationModality::Text,
        )),
        ..EpistemicStores::default()
    };
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);

    let projection = graph.epistemic_projection().expect("projection");
    let node = projection
        .list_nodes()
        .expect("nodes")
        .into_iter()
        .find(|node| node.has_label("Observation"))
        .expect("observation node");

    // Naming a policy that decided nothing would be a claim the record cannot
    // support.
    assert!(!node.properties().contains_key("observation_content_policy"));
}
