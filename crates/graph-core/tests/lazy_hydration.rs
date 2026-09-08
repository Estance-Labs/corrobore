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
//! Contract for lazy hydration (issue #282, ADR-0021).
//!
//! Three levels of disclosure, and a caller that moves between them
//! deliberately:
//!
//! ```text
//! METADATA  →  PREVIEW  →  FULL CONTENT
//! ```
//!
//! This is where the epic's invariant becomes observable: a caller pays for
//! what it reads, not for what it traverses.
use std::cell::RefCell;

use graph_core::{
    ContentHandle, ContentStore, ContentStoreError, HydrationError, InlineContent,
    MemoryObjectStore, ObjectContentStore, ObjectStore, describe_handle, hydrate_handle,
    hydrate_handle_range,
};

const LARGE: usize = 200_000;

fn content_store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

/// Object store recording how many bytes each call transferred.
#[derive(Default)]
struct MeteredObjectStore {
    inner: MemoryObjectStore,
    transferred: RefCell<u64>,
}

impl MeteredObjectStore {
    fn transferred(&self) -> u64 {
        *self.transferred.borrow()
    }
}

impl ObjectStore for MeteredObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        self.inner.put(key, bytes)
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        let bytes = self.inner.get(key)?;
        *self.transferred.borrow_mut() += bytes.len() as u64;
        Ok(bytes)
    }

    fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Vec<u8>, ContentStoreError> {
        let bytes = self.inner.get_range(key, range)?;
        *self.transferred.borrow_mut() += bytes.len() as u64;
        Ok(bytes)
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        self.inner.head(key)
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        self.inner.remove(key)
    }
}

fn metered_with_large_content() -> (ObjectContentStore<MeteredObjectStore>, ContentHandle) {
    let mut store = ObjectContentStore::new("memory", MeteredObjectStore::default());
    let reference = store
        .store(&vec![b'x'; LARGE], Some("application/pdf"))
        .expect("store");
    (store, ContentHandle::External(reference))
}

#[test]
fn describing_a_handle_transfers_nothing() {
    let (store, handle) = metered_with_large_content();

    let disclosure = describe_handle(&handle, &store).expect("describe");

    assert_eq!(disclosure.byte_length(), LARGE as u64);
    assert_eq!(disclosure.media_type(), Some("application/pdf"));
    // Traversing a node must never cost megabytes.
    assert_eq!(
        store.objects().transferred(),
        0,
        "metadata disclosure transferred content"
    );
}

#[test]
fn a_ranged_hydration_transfers_only_that_range() {
    let (store, handle) = metered_with_large_content();

    let window = hydrate_handle_range(&handle, &store, 10..42).expect("range");

    assert_eq!(window.len(), 32);
    assert_eq!(
        store.objects().transferred(),
        32,
        "a window of a large artifact must not read the whole artifact"
    );
}

#[test]
fn full_hydration_transfers_the_content_the_caller_asked_for() {
    let (store, handle) = metered_with_large_content();

    let bytes = hydrate_handle(&handle, &store).expect("hydrate");

    assert_eq!(bytes.len(), LARGE);
    assert_eq!(store.objects().transferred(), LARGE as u64);
}

#[test]
fn each_level_costs_only_what_it_discloses() {
    let (store, handle) = metered_with_large_content();

    describe_handle(&handle, &store).expect("describe");
    let after_metadata = store.objects().transferred();
    hydrate_handle_range(&handle, &store, 0..64).expect("preview");
    let after_preview = store.objects().transferred();
    hydrate_handle(&handle, &store).expect("full");
    let after_full = store.objects().transferred();

    assert_eq!(after_metadata, 0);
    assert_eq!(after_preview, 64);
    assert_eq!(after_full, 64 + LARGE as u64);
}

#[test]
fn inline_content_is_served_without_a_store_round_trip() {
    let store = content_store();
    let handle = ContentHandle::inline(InlineContent::Text("Actor A operates B.".to_owned()));

    // The bytes are already present, so no store is consulted.
    assert_eq!(
        hydrate_handle(&handle, &store).expect("hydrate"),
        b"Actor A operates B."
    );
    assert_eq!(
        hydrate_handle_range(&handle, &store, 0..5).expect("range"),
        b"Actor"
    );
    let disclosure = describe_handle(&handle, &store).expect("describe");
    assert!(disclosure.is_inline());
    assert_eq!(disclosure.byte_length(), 19);
    assert_eq!(disclosure.media_type(), Some("text/plain"));
    assert_eq!(disclosure.sha256(), handle.sha256());
    assert!(disclosure.content_id().is_none());
}

#[test]
fn hydration_verifies_the_reference_that_named_the_content() {
    let mut store = content_store();
    let reference = store
        .store(b"Actor A operates B.", Some("text/plain"))
        .expect("store");
    let handle = ContentHandle::External(reference.clone());
    store.corrupt_for_test(&reference, b"Actor A operates Z.");

    // Hydration must not silently return different bytes than the evidence
    // recorded.
    assert!(matches!(
        hydrate_handle(&handle, &store),
        Err(HydrationError::Unavailable(
            ContentStoreError::DigestMismatch { .. }
        ))
    ));
}

#[test]
fn hydrating_the_same_reference_twice_yields_identical_bytes() {
    let (store, handle) = metered_with_large_content();

    let first = hydrate_handle(&handle, &store).expect("first");
    let second = hydrate_handle(&handle, &store).expect("second");

    assert_eq!(first, second);
}

#[test]
fn requesting_content_that_was_never_retained_fails_with_that_reason() {
    let mut source = content_store();
    let reference = source
        .store(b"Actor A operates B.", Some("text/plain"))
        .expect("store");
    let handle = ContentHandle::External(reference);
    let empty = content_store();

    // Returning empty content would read as an absence of evidence.
    assert!(matches!(
        hydrate_handle(&handle, &empty),
        Err(HydrationError::Unavailable(ContentStoreError::NotRetained(
            _
        )))
    ));
}

#[test]
fn requesting_erased_content_says_it_was_erased() {
    let mut store = content_store();
    let reference = store
        .store(b"Actor A operates B.", Some("text/plain"))
        .expect("store");
    let handle = ContentHandle::External(reference.clone());
    store.erase(&reference, "subject request").expect("erase");

    assert!(matches!(
        hydrate_handle(&handle, &store),
        Err(HydrationError::Unavailable(
            ContentStoreError::Erased { .. }
        ))
    ));
    // Metadata still describes what was there, which is the point of a tombstone.
    let disclosure = describe_handle(&handle, &store).expect("describe");
    assert!(disclosure.is_erased());
    assert_eq!(disclosure.byte_length(), 19);
}

#[test]
fn a_range_outside_the_content_fails_rather_than_being_clamped() {
    let (store, handle) = metered_with_large_content();
    let (start, end) = (100u64, 10u64);

    for range in [0..LARGE as u64 + 1, start..end] {
        assert!(
            hydrate_handle_range(&handle, &store, range.clone()).is_err(),
            "{range:?}"
        );
    }
    assert_eq!(store.objects().transferred(), 0);
}

#[test]
fn a_disclosure_describes_content_without_carrying_it() {
    let (store, handle) = metered_with_large_content();

    let disclosure = describe_handle(&handle, &store).expect("describe");

    assert_eq!(disclosure.sha256(), handle.sha256());
    assert_eq!(
        disclosure.content_id(),
        Some("content--".to_owned() + &handle.sha256()).as_deref()
    );
    assert!(!disclosure.is_inline());
}
