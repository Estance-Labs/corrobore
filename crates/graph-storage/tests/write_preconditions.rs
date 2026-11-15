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

//! Issue #322: optimistic preconditions on writes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{NodeInput, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, GraphStorageError, MAIN_BRANCH, RecordFormat, StorageManifest, StorageRoot,
    StorageTimestamp, StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store() -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-322-{}-{unique}",
        std::process::id()
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-322".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

fn write(
    store: &mut CanonicalEngineStore,
    id: &str,
    expected_head: Option<graph_storage::GraphCommitId>,
) -> Result<graph_storage::AtomicPersistentMutationOutcome, GraphStorageError> {
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    current
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(id.to_owned())),
        )
        .unwrap();
    store.commit_transition_with_context(
        &previous,
        &current,
        DurableTransactionId::new(id).unwrap(),
        TransitionCommitContext {
            expected_head,
            ..TransitionCommitContext::default()
        },
        None,
    )
}

#[test]
fn main_write_from_a_stale_head_is_refused_without_writing() {
    let root = empty_store();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let read_head = store.graph_head(MAIN_BRANCH).unwrap();
    write(&mut store, "tx--issue-322-first", Some(read_head.clone())).unwrap();
    let after_first = store.graph_head(MAIN_BRANCH).unwrap();

    let refused = write(&mut store, "tx--issue-322-stale", Some(read_head));
    match refused {
        Err(GraphStorageError::GraphHeadMoved { branch, actual, .. }) => {
            assert_eq!(branch, MAIN_BRANCH);
            assert_eq!(actual, after_first.to_string());
        }
        other => panic!("expected GraphHeadMoved, got {other:?}"),
    }
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), after_first);
    assert_eq!(store.catalog().latest_node_records.len(), 1);

    write(&mut store, "tx--issue-322-fresh", Some(after_first)).unwrap();
    assert_eq!(store.catalog().latest_node_records.len(), 2);

    let _ = fs::remove_dir_all(root.path());
}
