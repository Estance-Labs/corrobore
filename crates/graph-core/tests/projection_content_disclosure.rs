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
//! Contract for content disclosure in the epistemic projection (issue #276).
//!
//! `Observation::to_property_map()` deliberately excludes the verbatim payload,
//! but `Graph::epistemic_projection()` used to add it back, so projecting a
//! graph materialized every payload regardless of what the caller intended to
//! read. The projection now describes the content, and a caller that genuinely
//! needs the bytes asks for them.
use graph_core::{
    EpistemicNodeKind, EpistemicStores, Graph, Node, ObservationId, ObservationInput,
    ObservationModality, PropertyValue, SourceId, SourceInput,
};

const LARGE_PAYLOAD_CHARS: usize = 40_000;

fn source_id() -> SourceId {
    SourceId::new("source--report").expect("source id should be valid")
}

/// Build one store holding a single observation with the supplied payload.
fn stores_with_payload(payload: &str, digest: Option<&str>) -> EpistemicStores {
    let mut stores = EpistemicStores::default();
    stores
        .sources
        .register_source(SourceInput::new(
            source_id(),
            "https://vendor.example/report.pdf",
            graph_core::EvidenceSourceType::Document,
        ))
        .expect("source should register");

    let mut input = ObservationInput::new(
        ObservationId::new("observation--span").expect("observation id should be valid"),
        source_id(),
        payload,
        ObservationModality::Text,
    );
    if let Some(digest) = digest {
        input = input.with_payload_sha256(digest);
    }
    stores
        .observations
        .create_observation(input, &stores.sources)
        .expect("observation should be created");
    stores
}

fn projected_observation(graph: &Graph, hydrated: bool) -> Node {
    let projection = if hydrated {
        graph
            .epistemic_projection_hydrated()
            .expect("hydrated projection should build")
    } else {
        graph
            .epistemic_projection()
            .expect("projection should build")
    };
    projection
        .list_nodes()
        .expect("projection nodes")
        .into_iter()
        .find(|node| node.has_label(EpistemicNodeKind::Observation.canonical_label()))
        .expect("observation node")
}

fn graph_with_payload(payload: &str, digest: Option<&str>) -> Graph {
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores_with_payload(payload, digest));
    graph
}

fn string_property(node: &Node, key: &str) -> String {
    match node.property(key) {
        Some(PropertyValue::String(value)) => value.clone(),
        other => panic!("{key} should be a string, found {other:?}"),
    }
}

#[test]
fn the_projection_does_not_carry_the_verbatim_payload() {
    let payload = "A".repeat(LARGE_PAYLOAD_CHARS);
    let graph = graph_with_payload(&payload, None);

    let node = projected_observation(&graph, false);

    assert!(
        !node.properties().contains_key("observation_payload"),
        "traversing an observation must not cost the payload"
    );
    // The preview is the only content-bearing property, and it is bounded.
    for (key, value) in node.properties() {
        if let PropertyValue::String(text) = value {
            assert!(
                text.len() < payload.len(),
                "{key} carries the whole payload under another name"
            );
        }
    }
}

#[test]
fn the_projection_describes_the_content_it_withholds() {
    let payload = "Actor A operates Campaign B.";
    let graph = graph_with_payload(payload, Some(&"a".repeat(64)));

    let node = projected_observation(&graph, false);

    assert_eq!(
        node.property("observation_content_size"),
        Some(&PropertyValue::Integer(payload.len() as i64))
    );
    assert_eq!(
        node.property("observation_payload_sha256"),
        Some(&PropertyValue::String("a".repeat(64)))
    );
    assert_eq!(string_property(&node, "observation_preview"), payload);
}

#[test]
fn a_short_payload_is_previewed_whole_and_marked_untruncated() {
    let payload = "Actor A operates Campaign B.";
    let graph = graph_with_payload(payload, None);

    let node = projected_observation(&graph, false);

    assert_eq!(string_property(&node, "observation_preview"), payload);
    assert_eq!(
        node.property("observation_preview_truncated"),
        Some(&PropertyValue::Bool(false))
    );
}

#[test]
fn a_truncated_preview_says_so_and_stays_bounded() {
    let payload = "B".repeat(LARGE_PAYLOAD_CHARS);
    let graph = graph_with_payload(&payload, None);

    let node = projected_observation(&graph, false);
    let preview = string_property(&node, "observation_preview");

    assert!(
        preview.len() < payload.len(),
        "a preview that is the payload is not a preview"
    );
    // A truncated preview read as the full passage would misrepresent evidence.
    assert_eq!(
        node.property("observation_preview_truncated"),
        Some(&PropertyValue::Bool(true))
    );
    assert!(payload.starts_with(&preview));
}

#[test]
fn a_multibyte_payload_is_previewed_without_splitting_a_character() {
    // Truncating on a byte index would panic or emit invalid UTF-8.
    let payload = "é".repeat(LARGE_PAYLOAD_CHARS);
    let graph = graph_with_payload(&payload, None);

    let node = projected_observation(&graph, false);
    let preview = string_property(&node, "observation_preview");

    assert!(payload.starts_with(&preview));
    assert!(preview.chars().all(|character| character == 'é'));
}

#[test]
fn absent_provenance_is_distinguishable_from_absent_content() {
    let graph = graph_with_payload("Actor A operates Campaign B.", None);

    let node = projected_observation(&graph, false);

    // The digest is optional, so it stays absent; the size is always known and
    // proves content exists. Without the size, a reader could not tell an
    // undigested observation from an empty one.
    assert!(!node.properties().contains_key("observation_payload_sha256"));
    assert_eq!(
        node.property("observation_content_size"),
        Some(&PropertyValue::Integer(28))
    );
}

#[test]
fn an_observation_can_never_carry_empty_content() {
    let mut stores = EpistemicStores::default();
    stores
        .sources
        .register_source(SourceInput::new(
            source_id(),
            "https://vendor.example/report.pdf",
            graph_core::EvidenceSourceType::Document,
        ))
        .expect("source should register");

    let empty = stores.observations.create_observation(
        ObservationInput::new(
            ObservationId::new("observation--empty").expect("observation id should be valid"),
            source_id(),
            "",
            ObservationModality::Text,
        ),
        &stores.sources,
    );

    // The projected size is therefore always positive, which is what makes an
    // absent digest readable as absent provenance rather than absent content.
    assert!(
        empty.is_err(),
        "an empty observation would be no observation"
    );
}

#[test]
fn hydration_restores_the_verbatim_payload_for_callers_that_need_it() {
    let payload = "C".repeat(LARGE_PAYLOAD_CHARS);
    let graph = graph_with_payload(&payload, None);

    let node = projected_observation(&graph, true);

    assert_eq!(
        node.property("observation_payload"),
        Some(&PropertyValue::String(payload.clone())),
        "an existing consumer needs a path that is not a change of meaning"
    );
    // Hydration adds the payload; it does not remove the description.
    assert_eq!(
        node.property("observation_content_size"),
        Some(&PropertyValue::Integer(payload.len() as i64))
    );
}

#[test]
fn hydration_is_the_only_way_to_reach_the_payload() {
    let graph = graph_with_payload("Actor A operates Campaign B.", None);

    let plain = projected_observation(&graph, false);
    let hydrated = projected_observation(&graph, true);

    assert!(!plain.properties().contains_key("observation_payload"));
    assert!(hydrated.properties().contains_key("observation_payload"));
}
