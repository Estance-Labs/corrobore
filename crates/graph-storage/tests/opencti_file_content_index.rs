// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_storage::{
    CanonicalEngineStore, CanonicalStoreOptions, GraphId, RecordFormat, StorageManifest,
    StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_access::{AccessContext, AccessMetadata};
use opencti_file_search::{
    ExtractionLimits, FileContentQuery, FileDescriptor, FileExtractionRequest, FileJobStore,
    extract_file,
};
use sha2::{Digest, Sha256};

fn root() -> graph_storage::StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    create_storage_root(
        std::env::temp_dir().join(format!("corrobore-issue-48-store-{unique}")),
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-48".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

#[test]
fn canonical_store_publishes_searches_deletes_and_rebuilds_file_content() {
    let root = root();
    let content = b"Persistent file malware.example.org".to_vec();
    let descriptor = FileDescriptor {
        file_id: "file--persistent".to_owned(),
        source_object_id: "report--persistent".to_owned(),
        blob_key: "opencti/persistent.txt".to_owned(),
        name: "persistent.txt".to_owned(),
        mime_type: "text/plain".to_owned(),
        content_hash: format!("{:x}", Sha256::digest(&content)),
        version: 1,
        access: AccessMetadata {
            marking_ids: vec!["marking--clear".to_owned()],
            ..AccessMetadata::default()
        },
    };
    let artifact = extract_file(
        FileExtractionRequest {
            descriptor: descriptor.clone(),
            content,
        },
        &ExtractionLimits {
            max_input_bytes: 100_000,
            max_extracted_bytes: 100_000,
            max_pages: 10,
            max_sheets: 10,
            max_rows_per_sheet: 100,
            max_cells: 1_000,
            max_chunks: 100,
            max_chunk_chars: 4_096,
        },
    )
    .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let enqueued = store.enqueue_file_extraction(descriptor, 9).unwrap();
    assert!(!enqueued.duplicate);
    assert_eq!(store.file_extraction_metrics(9).unwrap().queue_depth, 1);
    store.publish_file_content(artifact, 10).unwrap();
    let access = AccessContext {
        subject_id: "user--clear".to_owned(),
        marking_ids: vec!["marking--clear".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
        ..AccessContext::default()
    };
    let query = FileContentQuery {
        text: "malware.example.org".to_owned(),
        ..FileContentQuery::default()
    };
    assert_eq!(store.search_file_content(&query, &access).unwrap().total, 1);

    let worker_content = b"Worker-published content worker-only.example".to_vec();
    let worker_artifact = extract_file(
        FileExtractionRequest {
            descriptor: FileDescriptor {
                file_id: "file--worker-published".to_owned(),
                source_object_id: "report--worker-published".to_owned(),
                blob_key: "opencti/worker.txt".to_owned(),
                name: "worker.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                content_hash: format!("{:x}", Sha256::digest(&worker_content)),
                version: 1,
                access: AccessMetadata {
                    marking_ids: vec!["marking--clear".to_owned()],
                    ..AccessMetadata::default()
                },
            },
            content: worker_content,
        },
        &ExtractionLimits::default(),
    )
    .unwrap();
    FileJobStore::open(root.path().join("file-content/metadata"), 3, 60_000)
        .unwrap()
        .publish_artifact(worker_artifact, 11)
        .unwrap();
    assert_eq!(
        store
            .search_file_content(
                &FileContentQuery {
                    text: "worker-only.example".to_owned(),
                    ..FileContentQuery::default()
                },
                &access,
            )
            .unwrap()
            .total,
        1
    );

    fs::remove_dir_all(root.path().join("search/file-content-v1/published")).unwrap();
    assert_eq!(store.search_file_content(&query, &access).unwrap().total, 1);

    store.delete_file_content("file--persistent").unwrap();
    assert_eq!(store.search_file_content(&query, &access).unwrap().total, 0);
    fs::remove_dir_all(root.path()).unwrap();
}
