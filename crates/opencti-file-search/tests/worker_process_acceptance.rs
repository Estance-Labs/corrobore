// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{fs, path::PathBuf, process::Command, time::SystemTime};

use opencti_access::AccessMetadata;
use opencti_file_search::{FileDescriptor, FileJobStore};
use sha2::{Digest, Sha256};

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "corrobore-file-worker-{name}-{}",
        std::process::id()
    ))
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[test]
fn dedicated_worker_process_fetches_extracts_and_publishes_one_job() {
    let root = root("one-job");
    let metadata = root.join("metadata");
    let blobs = root.join("blobs");
    fs::create_dir_all(&blobs).unwrap();
    let content = b"process isolated malware.example.org";
    fs::write(blobs.join("sample.txt"), content).unwrap();

    let mut store = FileJobStore::open(metadata.clone(), 3, 5_000).unwrap();
    store
        .enqueue(
            FileDescriptor {
                file_id: "file--worker-process".to_owned(),
                source_object_id: "report--worker-process".to_owned(),
                blob_key: "sample.txt".to_owned(),
                name: "sample.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                content_hash: format!("{:x}", Sha256::digest(content)),
                version: 1,
                access: AccessMetadata::default(),
            },
            now_ms(),
        )
        .unwrap();
    drop(store);

    let status = Command::new(env!("CARGO_BIN_EXE_corrobore-file-worker"))
        .arg("--extract-once")
        .env("CORROBORE_FILE_METADATA_DIR", &metadata)
        .env("CORROBORE_FILE_BLOB_ROOT", &blobs)
        .env("CORROBORE_FILE_LEASE_MS", "5000")
        .env("CORROBORE_FILE_MAX_RUNTIME_MS", "1000")
        .status()
        .unwrap();
    assert!(status.success());

    let store = FileJobStore::open(metadata, 3, 5_000).unwrap();
    let artifacts = store.artifacts().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].descriptor.file_id, "file--worker-process");

    fs::remove_dir_all(root).unwrap();
}
