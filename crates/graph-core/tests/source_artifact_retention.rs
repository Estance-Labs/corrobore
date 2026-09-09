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
//! Contract for retaining the source artifact (issue #279, ADR-0021).
//!
//! `artifact_sha256` proves an artifact has not changed; it cannot reproduce
//! it. When a URI rots or serves other bytes later, an evidence chain that
//! ends at a hash nobody can resolve is the situation an evidence graph exists
//! to survive.
//!
//! Retaining the artifact is storage. Whether two sources are independent is an
//! epistemic judgement, and sharing a stored blob must not touch it.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimStore, ClaimTarget, ContentPlacementDecision, ContentRef,
    ContentStoragePolicy, ContentStore, EvidenceRecordStore, EvidenceSourceType, MemoryObjectStore,
    ObjectContentStore, ObservationId, ObservationInput, ObservationModality, ObservationStore,
    PropertyValue, SourceId, SourceInput, SourceStore, TemporalTimestamp, VerdictAsOf,
    ingest_content,
};
use sha2::{Digest, Sha256};

const ARTIFACT: &[u8] = b"%PDF-1.7 vendor report bytes";

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("source id should be valid")
}

fn content_store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

/// Place an artifact under a policy narrow enough that anything is offloaded,
/// which is the only shape a source version can retain.
fn placed(
    bytes: &[u8],
    media_type: Option<&str>,
    content: &mut ObjectContentStore<MemoryObjectStore>,
) -> ContentPlacementDecision {
    let policy = ContentStoragePolicy::new("content-policy-v1", 1).expect("policy");
    ingest_content(bytes, media_type, &policy, content).expect("ingest")
}

fn input(id: &str) -> SourceInput {
    SourceInput::new(
        source_id(id),
        "https://vendor.example/report.pdf",
        EvidenceSourceType::Document,
    )
}

#[test]
fn a_source_can_retain_the_artifact_it_was_ingested_from() {
    let mut content = content_store();

    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
        )
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");
    let retained = source.artifact_content().expect("artifact reference");
    assert_eq!(content.load(retained).expect("load"), ARTIFACT);
}

#[test]
fn a_retained_artifact_verifies_against_the_digest_the_source_declared() {
    let mut content = content_store();

    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
        )
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");
    assert_eq!(
        source.artifact_content().expect("reference").sha256(),
        source.artifact_sha256().expect("digest")
    );
}

#[test]
fn a_retained_artifact_that_contradicts_the_declared_digest_is_refused() {
    let mut content = content_store();

    let mut sources = SourceStore::default();
    // Retaining bytes under a source that declares a different artifact would
    // make the retained content misrepresent what was ingested.
    let failure = sources.register_source(
        input("source--report")
            .with_artifact_sha256(digest(b"different bytes entirely"))
            .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
    );

    assert!(
        failure.is_err(),
        "a contradicted artifact must not register"
    );
}

#[test]
fn a_source_may_declare_a_digest_without_retaining_the_artifact() {
    let mut sources = SourceStore::default();
    sources
        .register_source(input("source--report").with_artifact_sha256(digest(ARTIFACT)))
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");

    // Retention is not always possible, and pretending otherwise would
    // misrepresent what can be audited.
    assert_eq!(source.artifact_sha256(), Some(digest(ARTIFACT).as_str()));
    assert!(source.artifact_content().is_none());
}

#[test]
fn retaining_an_artifact_leaves_the_declared_digest_untouched() {
    let mut content = content_store();
    let mut sources = SourceStore::default();

    sources
        .register_source(input("source--plain").with_artifact_sha256(digest(ARTIFACT)))
        .expect("register");
    sources
        .register_source(
            input("source--retained")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
        )
        .expect("register");

    // Dependency signalling reads artifact_sha256, so retention must not move it.
    let plain = sources
        .current_source(&source_id("source--plain"))
        .expect("source");
    let retained = sources
        .current_source(&source_id("source--retained"))
        .expect("source");
    assert_eq!(plain.artifact_sha256(), retained.artifact_sha256());
}

#[test]
fn two_sources_ingesting_identical_bytes_do_not_each_store_a_copy() {
    let mut content = content_store();

    let first = content
        .store(ARTIFACT, Some("application/pdf"))
        .expect("store");
    let second = content
        .store(ARTIFACT, Some("application/pdf"))
        .expect("store");

    assert_eq!(first.content_id(), second.content_id());
    assert_eq!(content.object_count(), 1);
}

fn stamp() -> BitemporalStamp {
    let time = TemporalTimestamp::new("2026-09-08T00:00:00Z").expect("timestamp");
    BitemporalStamp::new(time.clone(), time).expect("stamp")
}

/// Cluster two observations, one per source, supporting one claim.
///
/// Clustering is what ADR-0021 protects: if the content plane reached it, two
/// independent outlets sharing a stored blob would collapse into one cluster
/// and, through ADR-0018, lower the verdict's confidence.
fn supporting_clusters(sources: &SourceStore) -> usize {
    let mut observations = ObservationStore::default();
    let mut claims = ClaimStore::default();
    let claim = ClaimId::new("claim--attribution").expect("claim id");
    claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("APT28 exploited CVE-2026-0001").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("attribution", None)),
        ))
        .expect("claim");

    for (observation, source) in [
        ("observation--a", "source--outlet-a"),
        ("observation--b", "source--outlet-b"),
    ] {
        let id = ObservationId::new(observation).expect("observation id");
        observations
            .create_observation(
                ObservationInput::new(
                    id.clone(),
                    source_id(source),
                    "APT28 exploited CVE-2026-0001.",
                    ObservationModality::Text,
                ),
                sources,
            )
            .expect("observation");
        claims.register_observation(id.clone());
        claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Observation(id),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_bitemporal(stamp()),
            )
            .expect("link");
    }

    let structure = claims
        .assign_independence_clusters(
            &claim,
            &VerdictAsOf::new(stamp().valid_from, stamp().transaction_time),
            &EvidenceRecordStore::default(),
            &observations,
            sources,
        )
        .expect("clusters");
    structure.clusters().len()
}

#[test]
fn sharing_a_stored_blob_does_not_make_two_sources_dependent() {
    let mut content = content_store();
    // The same evidence sentence appears in two genuinely independent outlets.

    let mut sources = SourceStore::default();
    for (id, uri, publisher) in [
        ("source--outlet-a", "https://a.example/story", "Outlet A"),
        ("source--outlet-b", "https://b.example/story", "Outlet B"),
    ] {
        sources
            .register_source(
                SourceInput::new(source_id(id), uri, EvidenceSourceType::Document)
                    .with_publisher(publisher)
                    .with_artifact_placement(placed(
                        b"APT28 exploited CVE-2026-0001.",
                        Some("text/plain"),
                        &mut content,
                    )),
            )
            .expect("register");
    }

    // Storage identity is not epistemic identity, per ADR-0021.
    assert_eq!(
        supporting_clusters(&sources),
        2,
        "a shared blob must not collapse two independent outlets"
    );
}

#[test]
fn two_sources_declaring_the_same_artifact_digest_are_still_dependent() {
    let mut sources = SourceStore::default();
    let shared = digest(ARTIFACT);
    for (id, uri, publisher) in [
        (
            "source--outlet-a",
            "https://a.example/report.pdf",
            "Outlet A",
        ),
        (
            "source--outlet-b",
            "https://b.example/report.pdf",
            "Outlet B",
        ),
    ] {
        sources
            .register_source(
                SourceInput::new(source_id(id), uri, EvidenceSourceType::Document)
                    .with_publisher(publisher)
                    .with_artifact_sha256(&shared),
            )
            .expect("register");
    }

    // This issue must not weaken the signal it protects.
    assert_eq!(
        supporting_clusters(&sources),
        1,
        "the same declared artifact still indicates dependence"
    );
}

#[test]
fn reading_a_source_never_transfers_the_artifact() {
    let mut content = content_store();
    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
        )
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");
    let properties = source.to_property_map();

    // The projection describes the artifact; it never carries it.
    assert_eq!(
        properties.get("source_artifact_content_size"),
        Some(&PropertyValue::Integer(ARTIFACT.len() as i64))
    );
    for (key, value) in &properties {
        if let PropertyValue::String(text) = value {
            assert!(
                !text.as_bytes().windows(4).any(|window| window == b"%PDF"),
                "{key} carries the artifact"
            );
        }
    }
}

#[test]
fn a_retained_artifact_is_visible_in_the_projection_without_its_bytes() {
    let mut content = content_store();
    let reference: ContentRef = content
        .store(ARTIFACT, Some("application/pdf"))
        .expect("store");
    let mut sources = SourceStore::default();
    sources
        .register_source(
            input("source--report")
                .with_artifact_sha256(digest(ARTIFACT))
                .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
        )
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");
    let properties = source.to_property_map();

    assert_eq!(
        properties.get("source_artifact_content_id"),
        Some(&PropertyValue::String(reference.content_id().to_owned()))
    );
    assert_eq!(
        properties.get("source_artifact_media_type"),
        Some(&PropertyValue::String("application/pdf".to_owned()))
    );
}

#[test]
fn a_source_without_a_retained_artifact_says_nothing_about_one() {
    let mut sources = SourceStore::default();
    sources
        .register_source(input("source--report"))
        .expect("register");

    let source = sources
        .current_source(&source_id("source--report"))
        .expect("source");
    let properties = source.to_property_map();

    assert!(!properties.contains_key("source_artifact_content_id"));
    assert!(!properties.contains_key("source_artifact_content_size"));
}

#[test]
fn retention_cannot_be_bolted_onto_an_already_registered_version() {
    let mut content = content_store();
    let mut sources = SourceStore::default();

    sources
        .register_source(input("source--report").with_artifact_sha256(digest(ARTIFACT)))
        .expect("register");
    let conflict = sources.register_source(
        input("source--report")
            .with_artifact_sha256(digest(ARTIFACT))
            .with_artifact_placement(placed(ARTIFACT, Some("application/pdf"), &mut content)),
    );

    // A source version is identified by its artifact, so what a version holds
    // cannot change under the same digest. Accepting it would mutate an
    // immutable version; ignoring it would silently drop the retained bytes.
    // Retention is therefore declared when the version is registered.
    assert!(
        conflict.is_err(),
        "retention must not mutate an already registered version"
    );
}
