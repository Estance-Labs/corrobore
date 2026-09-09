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
//! Contract for observation content held behind a handle (issue #281).
//!
//! This is the only step of the content-plane epic that changes what is on
//! disk, so backward compatibility is the first thing asserted: a store written
//! with `payload: String` must still read, and its observations must behave
//! exactly as before.
use graph_core::{
    ContentHandle, ContentPlacementDecision, ContentStoragePolicy, EpistemicStores,
    EvidenceSourceType, Graph, InlineContent, MemoryObjectStore, ObjectContentStore, Observation,
    ObservationId, ObservationInput, ObservationModality, ObservationStore, PropertyValue,
    SourceId, SourceInput, SourceStore, ingest_content,
};
use sha2::{Digest, Sha256};

const SPAN: &str = "Aster operates the North Relay.";

/// Place `bytes` under a policy narrow enough that anything is offloaded.
fn offloaded(bytes: &[u8]) -> ContentPlacementDecision {
    let policy = ContentStoragePolicy::new("content-policy-test", 1).expect("policy");
    let mut content = ObjectContentStore::new("memory", MemoryObjectStore::default());
    ingest_content(bytes, Some("text/plain"), &policy, &mut content).expect("ingest")
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

// --- backward compatibility -------------------------------------------------

#[test]
fn an_observation_written_as_a_plain_payload_still_reads() {
    // Exactly the shape a store written before this change contains.
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

    assert_eq!(observation.payload_text(), Some(SPAN));
    assert_eq!(observation.content().byte_length(), SPAN.len() as u64);
    assert!(observation.content().inline_bytes().is_some());
}

#[test]
fn a_legacy_store_round_trips_through_the_new_shape() {
    let legacy = serde_json::json!({
        "observations": [{
            "id": { "value": "observation--span" },
            "source_id": { "value": "source--report" },
            "selector": null,
            "payload": SPAN,
            "modality": "Text",
            "observed_at": null,
            "payload_sha256": null,
            "supersedes": null,
            "derived_from_legacy": false
        }],
        "superseded_by": {}
    });

    let store: ObservationStore = serde_json::from_value(legacy).expect("legacy store");
    let reserialized = serde_json::to_string(&store).expect("serialize");
    let reopened: ObservationStore = serde_json::from_str(&reserialized).expect("reopen");

    assert_eq!(reopened, store);
    assert_eq!(only(&reopened).payload_text(), Some(SPAN));
}

#[test]
fn a_legacy_observation_keeps_its_declared_digest() {
    let recorded = digest(SPAN.as_bytes());
    let legacy = serde_json::json!({
        "id": { "value": "observation--span" },
        "source_id": { "value": "source--report" },
        "selector": null,
        "payload": SPAN,
        "modality": "Text",
        "observed_at": null,
        "payload_sha256": recorded,
        "supersedes": null,
        "derived_from_legacy": false
    });

    let observation: Observation = serde_json::from_value(legacy).expect("legacy observation");

    // Moving where bytes live must not change an observation's identity.
    assert_eq!(observation.payload_sha256(), Some(recorded.as_str()));
}

// --- the new shape ----------------------------------------------------------

#[test]
fn a_text_observation_is_still_created_from_a_string() {
    let store = store_with(ObservationInput::new(
        observation_id("observation--span"),
        source_id(),
        SPAN,
        ObservationModality::Text,
    ));

    // The existing constructor keeps working, which is why this change does not
    // ripple through every caller.
    assert_eq!(only(&store).payload_text(), Some(SPAN));
}

#[test]
fn an_observation_can_hold_offloaded_content() {
    let store = store_with(
        ObservationInput::with_placement(
            observation_id("observation--span"),
            source_id(),
            offloaded(SPAN.as_bytes()),
            ObservationModality::Text,
        )
        .with_payload_sha256(digest(SPAN.as_bytes())),
    );

    let observation = only(&store);
    // Reading metadata must never transfer offloaded content.
    assert!(observation.payload_text().is_none());
    assert_eq!(observation.content().byte_length(), SPAN.len() as u64);
    assert_eq!(observation.content().sha256(), digest(SPAN.as_bytes()));
}

#[test]
fn a_round_trip_preserves_whether_content_was_inline_or_offloaded() {
    let inputs = [
        ObservationInput::with_content(
            observation_id("observation--span"),
            source_id(),
            ContentHandle::inline(InlineContent::Text(SPAN.to_owned())),
            ObservationModality::Text,
        ),
        ObservationInput::with_placement(
            observation_id("observation--span"),
            source_id(),
            offloaded(SPAN.as_bytes()),
            ObservationModality::Text,
        ),
    ];

    for input in inputs {
        let store = store_with(input);
        let expected = only(&store).content().clone();
        let encoded = serde_json::to_string(&store).expect("serialize");
        let reopened: ObservationStore = serde_json::from_str(&encoded).expect("reopen");

        // Where content lives is a fact about the store, not a rendering choice.
        assert_eq!(only(&reopened).content(), &expected);
    }
}

#[test]
fn an_observation_still_refuses_empty_content() {
    let sources = sources();
    let mut observations = ObservationStore::default();

    let empty = observations.create_observation(
        ObservationInput::new(
            observation_id("observation--empty"),
            source_id(),
            "",
            ObservationModality::Text,
        ),
        &sources,
    );

    assert!(empty.is_err());
}

#[test]
fn the_projection_describes_offloaded_content_without_a_preview() {
    let stores = EpistemicStores {
        sources: sources(),
        observations: store_with(ObservationInput::with_placement(
            observation_id("observation--span"),
            source_id(),
            offloaded(SPAN.as_bytes()),
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

    // A preview needs the bytes, so its absence is the honest answer rather
    // than an empty string that would read as empty content.
    assert!(!node.properties().contains_key("observation_preview"));
    assert!(!node.properties().contains_key("observation_payload"));
    assert_eq!(
        node.property("observation_content_size"),
        Some(&PropertyValue::Integer(SPAN.len() as i64))
    );
}

#[test]
fn inline_text_is_still_written_under_the_field_readers_expect() {
    let store = store_with(ObservationInput::new(
        observation_id("observation--span"),
        source_id(),
        SPAN,
        ObservationModality::Text,
    ));

    let encoded = serde_json::to_value(only(&store)).expect("serialize");

    // `Observation` is serialized straight into the audit response and the
    // claim audit archive, so renaming this field would break consumers that
    // never asked for a content plane.
    assert_eq!(encoded["payload"], SPAN);
    assert!(encoded.get("content").is_none());
}

#[test]
fn offloaded_content_is_written_as_content_rather_than_a_missing_payload() {
    let store = store_with(ObservationInput::with_placement(
        observation_id("observation--span"),
        source_id(),
        offloaded(SPAN.as_bytes()),
        ObservationModality::Text,
    ));

    let encoded = serde_json::to_value(only(&store)).expect("serialize");

    // An empty `payload` would tell an old reader the observation said nothing.
    assert!(encoded.get("payload").is_none());
    assert!(encoded.get("content").is_some());
}

// --- supersession -----------------------------------------------------------

#[test]
fn supersession_works_across_inline_and_offloaded_content() {
    let sources = sources();
    let mut observations = ObservationStore::default();
    let first = observations
        .create_observation(
            ObservationInput::new(
                observation_id("observation--first"),
                source_id(),
                SPAN,
                ObservationModality::Text,
            ),
            &sources,
        )
        .expect("first");

    let second = observations
        .supersede_observation(
            &first,
            ObservationInput::with_placement(
                observation_id("observation--second"),
                source_id(),
                offloaded(b"Aster no longer operates the North Relay."),
                ObservationModality::Text,
            ),
            &sources,
        )
        .expect("supersede");

    // The append-only history must stay readable whichever side is offloaded.
    assert_eq!(observations.observations().len(), 2);
    assert_eq!(observations.superseded_by(&first), Some(&second));
    assert_eq!(
        observations
            .observation_by_id(&first)
            .expect("first")
            .payload_text(),
        Some(SPAN)
    );
    assert!(
        observations
            .observation_by_id(&second)
            .expect("second")
            .payload_text()
            .is_none()
    );
}

#[test]
fn recreating_an_observation_with_different_content_is_still_a_conflict() {
    let sources = sources();
    let mut observations = ObservationStore::default();
    observations
        .create_observation(
            ObservationInput::new(
                observation_id("observation--span"),
                source_id(),
                SPAN,
                ObservationModality::Text,
            ),
            &sources,
        )
        .expect("first");

    let conflict = observations.create_observation(
        ObservationInput::new(
            observation_id("observation--span"),
            source_id(),
            "Something else entirely.",
            ObservationModality::Text,
        ),
        &sources,
    );

    assert!(conflict.is_err(), "observations have no update path");
}

#[test]
fn recreating_an_identical_observation_stays_idempotent() {
    let sources = sources();
    let mut observations = ObservationStore::default();
    let input = || {
        ObservationInput::new(
            observation_id("observation--span"),
            source_id(),
            SPAN,
            ObservationModality::Text,
        )
    };

    let first = observations
        .create_observation(input(), &sources)
        .expect("first");
    let second = observations
        .create_observation(input(), &sources)
        .expect("second");

    assert_eq!(first, second);
    assert_eq!(observations.observations().len(), 1);
}
