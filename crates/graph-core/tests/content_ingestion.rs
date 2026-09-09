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
//! Contract for applying the storage policy at ingestion (issue #294).
//!
//! The epic delivered every part of the content plane, but nothing decided
//! inline or offloaded, so no content was ever offloaded. Per ADR-0021 that
//! decision belongs to the engine under a recorded policy, not to the caller.
use std::ops::Range;

use graph_core::{
    ContentHandle, ContentPlacement, ContentRef, ContentStatus, ContentStoragePolicy, ContentStore,
    ContentStoreError, MemoryObjectStore, ObjectContentStore, ingest_content,
};

const SPAN: &[u8] = b"Aster operates the North Relay.";

fn policy() -> ContentStoragePolicy {
    ContentStoragePolicy::new("content-policy-v1", 1_024).expect("policy")
}

fn store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

/// Store that refuses every write, standing in for an unreachable backend.
struct UnreachableStore;

impl ContentStore for UnreachableStore {
    fn store(
        &mut self,
        _bytes: &[u8],
        _media_type: Option<&str>,
    ) -> Result<ContentRef, ContentStoreError> {
        Err(ContentStoreError::Backend(
            "content store unreachable".to_owned(),
        ))
    }

    fn load(&self, _reference: &ContentRef) -> Result<Vec<u8>, ContentStoreError> {
        Err(ContentStoreError::Backend(
            "content store unreachable".to_owned(),
        ))
    }

    fn load_range(
        &self,
        _reference: &ContentRef,
        _range: Range<u64>,
    ) -> Result<Vec<u8>, ContentStoreError> {
        Err(ContentStoreError::Backend(
            "content store unreachable".to_owned(),
        ))
    }

    fn describe(&self, _reference: &ContentRef) -> Result<ContentStatus, ContentStoreError> {
        Err(ContentStoreError::Backend(
            "content store unreachable".to_owned(),
        ))
    }

    fn erase(&mut self, _reference: &ContentRef, _reason: &str) -> Result<(), ContentStoreError> {
        Err(ContentStoreError::Backend(
            "content store unreachable".to_owned(),
        ))
    }
}

#[test]
fn a_short_span_stays_with_its_record() {
    let mut content = store();

    let decision =
        ingest_content(SPAN, Some("text/plain"), &policy(), &mut content).expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Inline);
    // A short span must cost no round trip, now or on every later read.
    assert_eq!(content.object_count(), 0);
    assert_eq!(decision.handle().inline_bytes().as_deref(), Some(SPAN));
}

#[test]
fn a_document_over_the_threshold_is_offloaded() {
    let mut content = store();
    let document = vec![b'x'; 4_096];

    let decision =
        ingest_content(&document, Some("text/plain"), &policy(), &mut content).expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Offloaded);
    assert_eq!(content.object_count(), 1);
    match decision.handle() {
        ContentHandle::External(reference) => {
            assert_eq!(reference.byte_length(), 4_096);
            // The record keeps a reference, not the bytes.
            assert!(decision.handle().inline_bytes().is_none());
            assert_eq!(content.load(reference).expect("load"), document);
        }
        other => panic!("expected offloaded content, found {other:?}"),
    }
}

#[test]
fn binary_and_media_are_offloaded_however_small() {
    for media_type in ["application/pdf", "image/png", "video/mp4"] {
        let mut content = store();

        let decision =
            ingest_content(b"tiny", Some(media_type), &policy(), &mut content).expect("ingest");

        // A small image must not become an inline blob.
        assert_eq!(
            decision.placement(),
            ContentPlacement::Offloaded,
            "{media_type}"
        );
        assert_eq!(content.object_count(), 1, "{media_type}");
    }
}

#[test]
fn content_of_unknown_type_is_offloaded_rather_than_assumed_textual() {
    let mut content = store();

    let decision = ingest_content(b"tiny", None, &policy(), &mut content).expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Offloaded);
}

#[test]
fn the_policy_that_decided_is_recorded_with_the_outcome() {
    let mut content = store();

    let decision =
        ingest_content(SPAN, Some("text/plain"), &policy(), &mut content).expect("ingest");

    // A threshold that is not recorded makes a store's layout irreproducible.
    assert_eq!(decision.policy_version(), "content-policy-v1");
}

#[test]
fn a_different_policy_places_the_same_document_differently_and_says_so() {
    let document = vec![b'x'; 4_096];
    let narrow = ContentStoragePolicy::new("content-policy-narrow", 1_024).expect("policy");
    let wide = ContentStoragePolicy::new("content-policy-wide", 128 * 1_024).expect("policy");
    let mut here = store();
    let mut there = store();

    let offloaded =
        ingest_content(&document, Some("text/plain"), &narrow, &mut here).expect("ingest");
    let inline = ingest_content(&document, Some("text/plain"), &wide, &mut there).expect("ingest");

    assert_eq!(offloaded.placement(), ContentPlacement::Offloaded);
    assert_eq!(inline.placement(), ContentPlacement::Inline);
    // Two stores holding identical content laid out differently must be able to
    // explain why.
    assert_ne!(offloaded.policy_version(), inline.policy_version());
}

#[test]
fn ingesting_identical_bytes_twice_offloads_them_once() {
    let mut content = store();
    let document = vec![b'x'; 4_096];

    let first =
        ingest_content(&document, Some("text/plain"), &policy(), &mut content).expect("first");
    let second =
        ingest_content(&document, Some("text/plain"), &policy(), &mut content).expect("second");

    assert_eq!(first.handle().sha256(), second.handle().sha256());
    assert_eq!(
        content.object_count(),
        1,
        "the second ingestion is not a second copy"
    );
}

#[test]
fn an_unreachable_store_fails_rather_than_falling_back_to_inline() {
    let document = vec![b'x'; 4_096];

    let failure = ingest_content(
        &document,
        Some("text/plain"),
        &policy(),
        &mut UnreachableStore,
    );

    // Falling back would place content the policy said to offload, and the
    // record would claim an inline copy nobody decided to keep.
    assert!(
        matches!(failure, Err(ContentStoreError::Backend(_))),
        "{failure:?}"
    );
}

#[test]
fn an_unreachable_store_does_not_block_content_that_stays_inline() {
    // Inline content never reaches the store, so an unreachable backend is
    // irrelevant to it.
    let decision =
        ingest_content(SPAN, Some("text/plain"), &policy(), &mut UnreachableStore).expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Inline);
}

#[test]
fn empty_content_is_refused_before_any_placement_is_decided() {
    let mut content = store();

    // Content that is nothing is the absence of content.
    assert!(ingest_content(b"", Some("text/plain"), &policy(), &mut content).is_err());
    assert_eq!(content.object_count(), 0);
}

#[test]
fn ingested_json_is_held_as_json_rather_than_opaque_text() {
    let mut content = store();

    let decision = ingest_content(
        b"{\"subject\":\"aster\"}",
        Some("application/json"),
        &policy(),
        &mut content,
    )
    .expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Inline);
    assert_eq!(decision.handle().media_type(), Some("application/json"));
}

#[test]
fn parameterised_json_is_held_as_json_like_the_policy_judged_it() {
    let mut content = store();

    // The policy admits this inline by prefix; ingestion must read it the same
    // way, or the handle would report text the caller never declared.
    let decision = ingest_content(
        b"{\"subject\":\"aster\"}",
        Some("application/json; charset=utf-8"),
        &policy(),
        &mut content,
    )
    .expect("ingest");

    assert_eq!(decision.placement(), ContentPlacement::Inline);
    assert_eq!(decision.handle().media_type(), Some("application/json"));
}
