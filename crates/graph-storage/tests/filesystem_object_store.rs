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
//! Contract for the durable object store backing the content plane (issue #286).
//!
//! `graph-core` states the storage-neutral contract and implements none of it,
//! so the durable backend lives here. Because the content-addressed semantics
//! live once in `ObjectContentStore`, this backend only has to move bytes
//! correctly — and correctly includes never serving a partial write as content.
use std::fs;

use graph_core::{
    ContentStore, ContentStoreError, MemoryObjectStore, ObjectContentStore, ObjectStore,
};
use graph_storage::FilesystemObjectStore;

const TEXT: &[u8] = b"Aster operates the North Relay.";

fn root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "corrobore-object-store-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("root");
    root
}

fn store() -> (std::path::PathBuf, FilesystemObjectStore) {
    let directory = root();
    let store = FilesystemObjectStore::open(&directory).expect("open");
    (directory, store)
}

#[test]
fn bytes_round_trip_through_the_filesystem() {
    let (_directory, mut store) = store();

    store.put("ab/cd/object", TEXT).expect("put");

    assert_eq!(store.get("ab/cd/object").expect("get"), TEXT);
    assert_eq!(store.head("ab/cd/object").expect("head"), TEXT.len() as u64);
}

#[test]
fn content_keeps_the_same_identity_it_would_have_in_any_backend() {
    let (_directory, disk) = store();
    let mut durable = ObjectContentStore::new("filesystem", disk);
    let mut memory = ObjectContentStore::new("memory", MemoryObjectStore::default());

    let here = durable.store(TEXT, Some("text/plain")).expect("store");
    let there = memory.store(TEXT, Some("text/plain")).expect("store");

    // The backend is a deployment choice, not a semantic one.
    assert_eq!(here.content_id(), there.content_id());
    assert_eq!(here.sha256(), there.sha256());
    assert_eq!(durable.load(&here).expect("load"), TEXT);
}

#[test]
fn a_ranged_read_touches_only_the_window() {
    let (_directory, mut store) = store();
    let large = vec![b'x'; 4_000_000];
    store.put("ab/cd/large", &large).expect("put");

    let window = store.get_range("ab/cd/large", 1_000..1_032).expect("range");

    // Reading the whole object and slicing it would defeat the reason the
    // ranged read exists.
    assert_eq!(window.len(), 32);
    assert!(window.iter().all(|byte| *byte == b'x'));
}

#[test]
fn a_range_outside_the_object_fails_rather_than_being_clamped() {
    let (_directory, mut store) = store();
    store.put("ab/cd/object", TEXT).expect("put");
    let length = TEXT.len() as u64;
    let (start, end) = (10u64, 5u64);

    for range in [length..length + 8, 0..length + 1, start..end] {
        assert!(
            store.get_range("ab/cd/object", range.clone()).is_err(),
            "{range:?} must be refused"
        );
    }
}

#[test]
fn reopening_over_an_existing_directory_finds_the_content() {
    let (directory, mut store) = store();
    store.put("ab/cd/object", TEXT).expect("put");
    drop(store);

    let reopened = FilesystemObjectStore::open(&directory).expect("reopen");

    assert_eq!(reopened.get("ab/cd/object").expect("get"), TEXT);
}

#[test]
fn an_interrupted_write_leaves_nothing_that_would_be_served() {
    let (directory, mut store) = store();
    store.put("ab/cd/object", TEXT).expect("put");

    // A partial file left behind by a crashed write must not become evidence.
    let stray = directory.join("ab").join("cd").join("object.next");
    fs::write(&stray, b"half a doc").expect("stray");
    let reopened = FilesystemObjectStore::open(&directory).expect("reopen");

    assert_eq!(reopened.get("ab/cd/object").expect("get"), TEXT);
    assert!(matches!(
        reopened.get("ab/cd/object.next"),
        Err(ContentStoreError::Invalid(_))
    ));
}

#[test]
fn an_object_that_was_never_written_is_absent_rather_than_empty() {
    let (_directory, store) = store();

    let absent = store.get("ab/cd/missing");

    // Empty bytes would read as an observation that said nothing.
    assert!(
        matches!(absent, Err(ContentStoreError::NotRetained(_))),
        "{absent:?}"
    );
    assert!(matches!(
        store.head("ab/cd/missing"),
        Err(ContentStoreError::NotRetained(_))
    ));
}

#[test]
fn a_store_creates_the_root_it_was_given() {
    let nested = root().join("nested").join("deep");

    let mut store = FilesystemObjectStore::open(&nested).expect("open");
    store.put("ab/cd/object", TEXT).expect("put");

    assert_eq!(store.get("ab/cd/object").expect("get"), TEXT);
    assert_eq!(store.root(), nested);
}

#[test]
fn a_truncated_object_is_reported_rather_than_served_short() {
    let (directory, mut store) = store();
    store.put("ab/cd/object", TEXT).expect("put");

    // Ask for a window the object can no longer satisfy.
    fs::write(directory.join("ab").join("cd").join("object"), b"short").expect("truncate");

    assert!(matches!(
        store.get_range("ab/cd/object", 0..TEXT.len() as u64),
        Err(ContentStoreError::RangeOutOfBounds { .. })
    ));
}

#[test]
fn a_key_cannot_escape_the_configured_root() {
    let (_directory, mut store) = store();

    // Keys are opaque, so a traversal attempt is a malformed key rather than a
    // path to follow.
    for key in ["../escape", "ab/../../escape", "/absolute", "ab/./cd"] {
        assert!(
            matches!(store.put(key, TEXT), Err(ContentStoreError::Invalid(_))),
            "{key}"
        );
        assert!(
            matches!(store.get(key), Err(ContentStoreError::Invalid(_))),
            "{key}"
        );
    }
}

#[test]
fn removing_an_object_that_was_never_written_says_so() {
    let (_directory, mut store) = store();

    assert!(matches!(
        store.remove("ab/cd/missing"),
        Err(ContentStoreError::NotRetained(_))
    ));
}

#[test]
fn erasure_through_the_content_store_removes_the_file() {
    let (directory, disk) = store();
    let mut content = ObjectContentStore::new("filesystem", disk);
    let reference = content.store(TEXT, Some("text/plain")).expect("store");

    content.erase(&reference, "subject request").expect("erase");

    assert!(!directory.join(reference.key()).exists());
    assert!(matches!(
        content.load(&reference),
        Err(ContentStoreError::Erased { .. })
    ));
}
