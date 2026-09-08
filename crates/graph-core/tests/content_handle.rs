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
//! Contract for content handles and the storage policy (issue #277, ADR-0021).
//!
//! A handle describes content whether the bytes are inline or offloaded, so a
//! reader never needs the bytes to describe what it is looking at. The
//! inline-or-offloaded decision belongs to the engine under a policy whose
//! identity is retained, because a configurable threshold that is not recorded
//! makes a store's layout irreproducible.
use graph_core::{
    ContentHandle, ContentPlacement, ContentRef, ContentStoragePolicy, InlineContent,
};
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn policy() -> ContentStoragePolicy {
    ContentStoragePolicy::new("content-policy-v1", 64 * 1024).expect("policy should be valid")
}

fn external_ref(bytes: &[u8], media_type: Option<&str>) -> ContentRef {
    ContentRef::new(
        "content--report",
        digest(bytes),
        bytes.len() as u64,
        "filesystem",
        "ab/cd/abcdef",
    )
    .expect("reference should be valid")
    .with_media_type_opt(media_type)
}

#[test]
fn an_inline_handle_describes_itself_without_being_asked_for_bytes() {
    let text = "Actor A operates Campaign B.";
    let handle = ContentHandle::inline(InlineContent::Text(text.to_owned()));

    assert_eq!(handle.sha256(), digest(text.as_bytes()));
    assert_eq!(handle.byte_length(), text.len() as u64);
    assert_eq!(handle.media_type(), Some("text/plain"));
}

#[test]
fn an_external_handle_describes_itself_without_the_bytes() {
    let bytes = vec![0u8; 4096];
    let handle = ContentHandle::External(external_ref(&bytes, Some("application/pdf")));

    assert_eq!(handle.sha256(), digest(&bytes));
    assert_eq!(handle.byte_length(), 4096);
    assert_eq!(handle.media_type(), Some("application/pdf"));
}

#[test]
fn an_inline_handle_cannot_disagree_with_its_own_bytes() {
    // The digest is derived, never declared, so a handle claiming a digest that
    // does not match the bytes it carries is unrepresentable.
    for content in [
        InlineContent::Text("Actor A operates Campaign B.".to_owned()),
        InlineContent::Json(serde_json::json!({"subject": "actor--a"})),
        InlineContent::Bytes(vec![1, 2, 3, 4]),
    ] {
        let handle = ContentHandle::inline(content);
        assert_eq!(
            handle.sha256(),
            digest(&handle.inline_bytes().expect("inline"))
        );
    }
}

#[test]
fn a_reference_refuses_a_digest_that_is_not_a_sha256() {
    assert!(ContentRef::new("content--a", "not-a-digest", 10, "filesystem", "key").is_err());
    assert!(ContentRef::new("content--a", "A".repeat(64), 10, "filesystem", "key").is_err());
    assert!(ContentRef::new("content--a", "a".repeat(63), 10, "filesystem", "key").is_err());
}

#[test]
fn a_reference_refuses_an_empty_identity_or_backend() {
    let good = digest(b"bytes");
    assert!(ContentRef::new("", &good, 5, "filesystem", "key").is_err());
    assert!(ContentRef::new("content--a", &good, 5, "", "key").is_err());
    assert!(ContentRef::new("content--a", &good, 5, "filesystem", "").is_err());
}

#[test]
fn the_policy_is_a_value_that_can_be_stated_compared_and_retained() {
    let a = policy();
    let b = policy();
    let other = ContentStoragePolicy::new("content-policy-v2", 1024).expect("policy");

    assert_eq!(a, b);
    assert_ne!(a, other);
    assert_eq!(a.version(), "content-policy-v1");
    assert_eq!(a.inline_max_bytes(), 64 * 1024);
    // Retained with the content, so a store's layout stays reproducible.
    let round_trip: ContentStoragePolicy =
        serde_json::from_str(&serde_json::to_string(&a).expect("serialize")).expect("deserialize");
    assert_eq!(round_trip, a);
}

#[test]
fn a_policy_without_an_identity_is_refused() {
    // An unnamed policy could not be retained, so the decision it made could
    // never be explained afterwards.
    assert!(ContentStoragePolicy::new("", 1024).is_err());
    assert!(ContentStoragePolicy::new("content-policy-v1", 0).is_err());
}

#[test]
fn small_text_stays_inline_and_large_text_is_offloaded() {
    let policy = policy();

    assert_eq!(
        policy.placement(Some("text/plain"), 1_000),
        ContentPlacement::Inline
    );
    assert_eq!(
        policy.placement(Some("text/plain"), 64 * 1024),
        ContentPlacement::Inline
    );
    assert_eq!(
        policy.placement(Some("text/plain"), 64 * 1024 + 1),
        ContentPlacement::Offloaded
    );
}

#[test]
fn binary_and_media_are_offloaded_however_small() {
    let policy = policy();

    for media_type in [
        "application/pdf",
        "image/png",
        "video/mp4",
        "audio/mpeg",
        "application/octet-stream",
    ] {
        assert_eq!(
            policy.placement(Some(media_type), 12),
            ContentPlacement::Offloaded,
            "{media_type} must not become an inline blob"
        );
    }
}

#[test]
fn json_and_text_subtypes_are_eligible_for_inline_storage() {
    let policy = policy();

    for media_type in ["text/plain", "text/html", "application/json"] {
        assert_eq!(
            policy.placement(Some(media_type), 128),
            ContentPlacement::Inline,
            "{media_type}"
        );
    }
}

#[test]
fn content_of_unknown_type_is_offloaded_rather_than_assumed_textual() {
    // Guessing that unlabelled bytes are text would inline arbitrary binary.
    assert_eq!(policy().placement(None, 12), ContentPlacement::Offloaded);
}

#[test]
fn the_same_policy_decides_the_same_way_every_time() {
    let policy = policy();
    let first = policy.placement(Some("text/plain"), 4_096);

    for _ in 0..8 {
        assert_eq!(policy.placement(Some("text/plain"), 4_096), first);
    }
}

#[test]
fn a_different_threshold_is_visible_in_the_decision_rather_than_silent() {
    let narrow = ContentStoragePolicy::new("content-policy-narrow", 1_024).expect("policy");
    let wide = ContentStoragePolicy::new("content-policy-wide", 128 * 1024).expect("policy");

    assert_eq!(
        narrow.placement(Some("text/plain"), 4_096),
        ContentPlacement::Offloaded
    );
    assert_eq!(
        wide.placement(Some("text/plain"), 4_096),
        ContentPlacement::Inline
    );
    // The policies differ as values, so the retained identity explains why two
    // stores holding identical content are laid out differently.
    assert_ne!(narrow, wide);
}

#[test]
fn a_handle_serializes_with_the_placement_it_was_given() {
    let inline = ContentHandle::inline(InlineContent::Text("short".to_owned()));
    let external = ContentHandle::External(external_ref(b"short", Some("text/plain")));

    for handle in [inline, external] {
        let round_trip: ContentHandle =
            serde_json::from_str(&serde_json::to_string(&handle).expect("serialize"))
                .expect("deserialize");
        // Whether content is inline or offloaded is a fact about the store, not
        // a rendering choice, so a round trip must preserve it.
        assert_eq!(round_trip, handle);
    }
}
