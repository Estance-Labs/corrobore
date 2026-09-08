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
//! Contract for the content store (issue #278, ADR-0021).
//!
//! Content is addressed by digest, so storing the same bytes twice stores them
//! once and no identity can ever name different bytes. Retrieval verifies what
//! it read, because content served as evidence must be the content the
//! reference named. Erasure is recorded rather than silent: erased content is
//! retrievable *as erased*, which is a different answer from content that was
//! never retained.
use std::cell::RefCell;
use std::collections::BTreeMap;

use graph_core::{
    ContentStatus, ContentStore, ContentStoreError, MemoryObjectStore, ObjectContentStore,
    ObjectStore,
};

const TEXT: &[u8] = b"Actor A operates Campaign B.";
const OTHER: &[u8] = b"Actor C operates Campaign D.";

fn store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

/// Object store recording which operations a caller reached for.
#[derive(Default)]
struct RecordingObjectStore {
    inner: MemoryObjectStore,
    calls: RefCell<Vec<String>>,
}

impl RecordingObjectStore {
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl ObjectStore for RecordingObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        self.calls.borrow_mut().push("put".to_owned());
        self.inner.put(key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        self.calls.borrow_mut().push("get".to_owned());
        self.inner.get(key)
    }

    fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Vec<u8>, ContentStoreError> {
        self.calls.borrow_mut().push("get_range".to_owned());
        self.inner.get_range(key, range)
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        self.calls.borrow_mut().push("head".to_owned());
        self.inner.head(key)
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        self.calls.borrow_mut().push("remove".to_owned());
        self.inner.remove(key)
    }
}

#[test]
fn stored_content_is_retrievable_by_the_reference_it_returned() {
    let mut store = store();

    let reference = store.store(TEXT, Some("text/plain")).expect("store");

    assert_eq!(store.load(&reference).expect("load"), TEXT);
    assert_eq!(reference.byte_length(), TEXT.len() as u64);
    assert_eq!(reference.media_type(), Some("text/plain"));
    assert_eq!(reference.backend(), "memory");
}

#[test]
fn the_same_content_is_stored_once_whoever_stores_it() {
    let mut store = store();

    let first = store.store(TEXT, Some("text/plain")).expect("first");
    let second = store.store(TEXT, Some("text/plain")).expect("second");

    assert_eq!(first.content_id(), second.content_id());
    assert_eq!(first.sha256(), second.sha256());
    assert_eq!(
        store.object_count(),
        1,
        "identical bytes must not be duplicated"
    );
}

#[test]
fn different_content_never_shares_an_identity() {
    let mut store = store();

    let first = store.store(TEXT, None).expect("first");
    let second = store.store(OTHER, None).expect("second");

    assert_ne!(first.content_id(), second.content_id());
    assert_eq!(store.object_count(), 2);
}

#[test]
fn the_same_content_has_the_same_identity_in_any_backend() {
    let mut here = ObjectContentStore::new("memory", MemoryObjectStore::default());
    let mut there = ObjectContentStore::new("elsewhere", MemoryObjectStore::default());

    let a = here.store(TEXT, None).expect("store");
    let b = there.store(TEXT, None).expect("store");

    // The backend is a deployment choice, not a semantic one.
    assert_eq!(a.sha256(), b.sha256());
    assert_eq!(a.content_id(), b.content_id());
    assert_ne!(a.backend(), b.backend());
}

#[test]
fn retrieval_verifies_what_it_read() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");

    store.corrupt_for_test(&reference, b"Actor A operates Campaign Z.");

    // Content served as evidence must be the content the reference named.
    let failure = store
        .load(&reference)
        .expect_err("substituted bytes must not be served");
    assert!(
        matches!(failure, ContentStoreError::DigestMismatch { .. }),
        "{failure:?}"
    );
}

#[test]
fn a_ranged_read_returns_exactly_the_window_requested() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");

    assert_eq!(store.load_range(&reference, 0..5).expect("range"), b"Actor");
    assert_eq!(store.load_range(&reference, 6..7).expect("range"), b"A");
}

#[test]
fn a_range_outside_the_content_fails_rather_than_being_clamped() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");
    let length = TEXT.len() as u64;
    // Bounds come from values so the backwards window is testable rather than
    // rejected as a literal before it reaches the store.
    let (start, end) = (10u64, 5u64);

    // A clamped range would answer a different question convincingly.
    for range in [length..length + 10, 0..length + 1, start..end] {
        assert!(
            store.load_range(&reference, range.clone()).is_err(),
            "range {range:?} must be refused"
        );
    }
}

#[test]
fn describing_content_does_not_transfer_it() {
    let mut store = ObjectContentStore::new("recording", RecordingObjectStore::default());
    let reference = store.store(TEXT, None).expect("store");

    let status = store.describe(&reference).expect("describe");

    assert_eq!(
        status,
        ContentStatus::Present {
            byte_length: TEXT.len() as u64
        }
    );
    // A caller must be able to size a decision before paying for it.
    assert!(
        !store.objects().calls().contains(&"get".to_owned()),
        "describe read the bytes: {:?}",
        store.objects().calls()
    );
}

#[test]
fn content_that_was_never_retained_says_so() {
    let mut source = store();
    let reference = source.store(TEXT, None).expect("store");
    let empty = store();

    let failure = empty
        .load(&reference)
        .expect_err("absent content must not load");
    assert!(
        matches!(failure, ContentStoreError::NotRetained(_)),
        "{failure:?}"
    );
    assert!(matches!(
        empty.describe(&reference),
        Err(ContentStoreError::NotRetained(_))
    ));
}

#[test]
fn erased_content_is_retrievable_as_erased() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");

    store
        .erase(&reference, "subject request 2026-09-08")
        .expect("erase");

    let failure = store
        .load(&reference)
        .expect_err("erased content must not load");
    match failure {
        ContentStoreError::Erased { reason, .. } => {
            assert_eq!(reason, "subject request 2026-09-08");
        }
        other => panic!("expected an erasure, found {other:?}"),
    }
}

#[test]
fn erasure_is_distinguishable_from_never_having_been_retained() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");
    store
        .erase(&reference, "legal hold released")
        .expect("erase");

    let erased = store.describe(&reference).expect("describe");

    // An evidence store must be able to say that something was there.
    match erased {
        ContentStatus::Erased {
            byte_length,
            reason,
        } => {
            assert_eq!(byte_length, TEXT.len() as u64);
            assert_eq!(reason, "legal hold released");
        }
        other => panic!("expected an erasure record, found {other:?}"),
    }
}

#[test]
fn erasure_removes_the_bytes_it_records() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");

    store.erase(&reference, "subject request").expect("erase");

    assert_eq!(store.object_count(), 0, "erasure must remove the bytes");
}

#[test]
fn erasing_content_that_was_never_retained_is_refused() {
    let mut source = store();
    let reference = source.store(TEXT, None).expect("store");
    let mut empty = store();

    // Recording an erasure that never happened would falsify the audit trail.
    assert!(matches!(
        empty.erase(&reference, "reason"),
        Err(ContentStoreError::NotRetained(_))
    ));
}

#[test]
fn an_erasure_must_state_a_reason() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");

    assert!(store.erase(&reference, "   ").is_err());
    assert!(store.erase(&reference, "").is_err());
}

#[test]
fn storing_erased_content_again_makes_it_present_again() {
    let mut store = store();
    let reference = store.store(TEXT, None).expect("store");
    store.erase(&reference, "subject request").expect("erase");

    let restored = store.store(TEXT, None).expect("store again");

    // Erasure removes bytes; it is not a permanent ban on the same bytes
    // arriving again from a legitimate source.
    assert_eq!(restored.content_id(), reference.content_id());
    assert_eq!(
        store.describe(&restored).expect("describe"),
        ContentStatus::Present {
            byte_length: TEXT.len() as u64
        }
    );
}

#[test]
fn empty_content_is_refused() {
    let mut store = store();

    // Content that is nothing is the absence of content, and storing it would
    // make an empty retrieval indistinguishable from a missing one.
    assert!(store.store(b"", None).is_err());
}

#[test]
fn a_reference_from_another_backend_is_not_served_silently() {
    let mut here = ObjectContentStore::new("memory", MemoryObjectStore::default());
    let mut there = ObjectContentStore::new("elsewhere", MemoryObjectStore::default());
    let elsewhere = there.store(TEXT, None).expect("store");
    here.store(TEXT, None).expect("store");

    // The bytes happen to be present, but the reference names another store.
    let failure = here
        .load(&elsewhere)
        .expect_err("foreign reference must be refused");
    assert!(
        matches!(failure, ContentStoreError::ForeignBackend { .. }),
        "{failure:?}"
    );
}

#[test]
fn the_object_store_reports_absence_rather_than_empty_bytes() {
    let store = MemoryObjectStore::default();

    assert!(matches!(
        store.get("missing"),
        Err(ContentStoreError::NotRetained(_))
    ));
    assert!(matches!(
        store.head("missing"),
        Err(ContentStoreError::NotRetained(_))
    ));
    assert!(matches!(
        store.get_range("missing", 0..1),
        Err(ContentStoreError::NotRetained(_))
    ));
}

#[test]
fn the_object_store_round_trips_bytes_and_reports_their_length() {
    let mut store = MemoryObjectStore::default();
    store.put("a/b/c", TEXT).expect("put");

    assert_eq!(store.get("a/b/c").expect("get"), TEXT);
    assert_eq!(store.head("a/b/c").expect("head"), TEXT.len() as u64);
    assert_eq!(store.get_range("a/b/c", 0..5).expect("range"), b"Actor");
}

#[test]
fn keys_are_derived_from_the_digest_so_an_identity_cannot_name_other_bytes() {
    let mut store = store();
    let first = store.store(TEXT, None).expect("store");
    let second = store.store(OTHER, None).expect("store");

    let keys: BTreeMap<&str, &str> = [
        (first.content_id(), first.key()),
        (second.content_id(), second.key()),
    ]
    .into_iter()
    .collect();

    assert_eq!(keys.len(), 2);
    for (content_id, key) in keys {
        assert!(
            key.ends_with(content_id.trim_start_matches("content--")),
            "{content_id} is not addressed by its digest"
        );
    }
}
