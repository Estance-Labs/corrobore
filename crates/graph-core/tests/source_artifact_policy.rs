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
//! Contract for retaining the policy that placed a source artifact (issue #299).
//!
//! #296 closed this for observations. A source artifact still took a reference
//! the caller built, so the same graph could explain where an observation's
//! content lives and not explain it for the artifact that observation came
//! from.
use graph_core::{
    ContentPlacementDecision, ContentStoragePolicy, EpistemicStores, EvidenceSourceType,
    MemoryObjectStore, ObjectContentStore, PropertyValue, Source, SourceId, SourceInput,
    SourceStore, ingest_content,
};
use sha2::{Digest, Sha256};

const ARTIFACT: &[u8] = b"%PDF-1.7 vendor report bytes";

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn content_store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

fn input(id: &str) -> SourceInput {
    SourceInput::new(
        SourceId::new(id).expect("source id"),
        "https://vendor.example/report.pdf",
        EvidenceSourceType::Document,
    )
}

/// Place an artifact the way an ingesting caller would.
fn placed(bytes: &[u8], media_type: Option<&str>) -> ContentPlacementDecision {
    let policy = ContentStoragePolicy::new("content-policy-v1", 4096).expect("policy");
    let mut content = content_store();
    ingest_content(bytes, media_type, &policy, &mut content).expect("ingest")
}

fn registered(id: &str, input: SourceInput) -> Source {
    let mut sources = SourceStore::default();
    sources.register_source(input).expect("register");
    sources
        .current_source(&SourceId::new(id).expect("source id"))
        .expect("current")
        .clone()
}

// --- the policy is retained --------------------------------------------------

#[test]
fn a_retained_artifact_records_the_policy_that_placed_it() {
    let source = registered(
        "source--report",
        input("source--report")
            .with_artifact_sha256(digest(ARTIFACT))
            .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"))),
    );

    assert_eq!(source.artifact_content_policy(), Some("content-policy-v1"));
    assert_eq!(
        source.artifact_content().expect("content").sha256(),
        digest(ARTIFACT)
    );
}

#[test]
fn the_recorded_policy_survives_reload() {
    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"))),
        )
        .expect("register");

    let encoded = serde_json::to_string(&sources).expect("serialize");
    let reopened: SourceStore = serde_json::from_str(&encoded).expect("reopen");

    let source = reopened
        .current_source(&SourceId::new("source--report").expect("id"))
        .expect("source");
    assert_eq!(source.artifact_content_policy(), Some("content-policy-v1"));
}

#[test]
fn the_projection_names_the_policy_that_retained_the_artifact() {
    let source = registered(
        "source--report",
        input("source--report")
            .with_artifact_sha256(digest(ARTIFACT))
            .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"))),
    );

    assert_eq!(
        source
            .to_property_map()
            .get("source_artifact_content_policy"),
        Some(&PropertyValue::String("content-policy-v1".to_owned()))
    );
}

// --- content no policy placed cannot be registered ---------------------------

#[test]
fn an_artifact_reference_no_policy_placed_cannot_be_registered() {
    // `SourceInput` is deserializable, so the builder is not the only way in.
    // Strip the policy from a valid input to get exactly what a hand-written
    // one would look like.
    let valid = input("source--report")
        .with_artifact_sha256(digest(ARTIFACT))
        .with_artifact_placement(placed(ARTIFACT, Some("application/pdf")));
    let mut wire = serde_json::to_value(&valid).expect("serialize");
    wire.as_object_mut()
        .expect("object")
        .remove("artifact_content_policy")
        .expect("policy was recorded");
    let unplaced: SourceInput = serde_json::from_value(wire).expect("input");
    let mut sources = SourceStore::default();

    // A reference to a store nobody was asked about leaves the source version
    // unable to explain why its artifact lives there.
    let refused = sources.register_source(unplaced);

    assert!(refused.is_err());
}

#[test]
fn an_artifact_the_policy_kept_inline_is_refused() {
    let mut sources = SourceStore::default();

    // A source describes its artifact and never carries it, so there is no
    // shape for an inline one. Refusing says so instead of dropping it.
    let refused = sources.register_source(
        input("source--note")
            .with_artifact_sha256(digest(b"A short note."))
            .with_artifact_placement(placed(b"A short note.", Some("text/plain"))),
    );

    assert!(refused.is_err());
}

#[test]
fn retained_bytes_that_are_not_the_declared_artifact_are_still_refused() {
    let mut sources = SourceStore::default();

    // The check that predates this issue must survive it.
    let refused = sources.register_source(
        input("source--report")
            .with_artifact_sha256(digest(b"different bytes entirely"))
            .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"))),
    );

    assert!(refused.is_err());
}

// --- nothing changes for a source that retains nothing -----------------------

#[test]
fn a_source_that_retains_nothing_is_unchanged() {
    let source = registered(
        "source--report",
        input("source--report").with_artifact_sha256(digest(ARTIFACT)),
    );

    assert_eq!(source.artifact_content_policy(), None);
    let encoded = serde_json::to_value(&source).expect("serialize");
    assert!(encoded.get("artifact_content_policy").is_none());
    assert!(
        !source
            .to_property_map()
            .contains_key("source_artifact_content_policy")
    );
}

#[test]
fn a_source_version_written_before_this_still_reads() {
    let legacy = serde_json::json!({
        "id": { "value": "source--report" },
        "version_id": { "value": "source--report@1" },
        "version": 1,
        "uri": "https://vendor.example/report.pdf",
        "source_type": "Document",
        "publisher": null,
        "authority_domain": null,
        "acquired_at": null,
        "artifact_sha256": null,
        "signature": null,
        "parent_source": null,
        "supersedes": null,
        "derived_from_legacy": false
    });

    let source: Source = serde_json::from_value(legacy).expect("legacy source");

    assert_eq!(source.artifact_content_policy(), None);
    assert!(source.artifact_content().is_none());
}

// --- interaction with the declared bundle shape ------------------------------

#[test]
fn a_store_holding_a_retained_artifact_policy_declares_the_newer_shape() {
    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"))),
        )
        .expect("register");
    let stores = EpistemicStores {
        sources,
        ..EpistemicStores::default()
    };

    // #291 declares the shape a bundle holds. A field only sources carry counts
    // just as much as one only observations carry.
    assert!(stores.declared_schema().is_some());
}
