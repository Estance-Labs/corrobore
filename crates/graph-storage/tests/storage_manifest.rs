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
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use graph_storage::{
    GraphId, GraphStorageError, RecordFormat, StorageManifest, StorageRoot, StorageTimestamp,
    StorageVersion, create_storage_root, open_storage_root, read_storage_manifest,
    validate_storage_manifest,
};

fn supported_manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--phase-2".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-06-30T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-06-30T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn unique_root(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();

    std::env::temp_dir().join(format!("graph-storage-{test_name}-{nanos}",))
}

fn valid_manifest_json(graph_id: &str) -> String {
    format!(
        r#"{{
 "storage_version": "V1",
 "graph_id": {{ "value": "{graph_id}" }},
 "created_at": {{ "value": "2026-06-30T00:00:00Z" }},
 "updated_at": {{ "value": "2026-06-30T00:00:00Z" }},
 "record_format": "JsonLinesV1"
}}"#,
    )
}

fn storage_root(path: PathBuf) -> StorageRoot {
    StorageRoot { path }
}

//
// Verify that the manifest shape preserves the minimum compatibility metadata
// required before any graph records, catalog entries, or adjacency
// payloads are loaded.
//
// Given a supported V1 storage manifest,
// when callers inspect its public fields,
// then storage version, graph ID, timestamps, and record format should remain explicit.
#[test]
fn storage_manifest_preserves_minimum_compatibility_metadata() {
    let manifest = supported_manifest();

    assert_eq!(manifest.storage_version, StorageVersion::V1);
    assert_eq!(manifest.graph_id.value, "graph--phase-2");
    assert_eq!(manifest.created_at.value, "2026-06-30T00:00:00Z");
    assert_eq!(manifest.updated_at.value, "2026-06-30T00:00:00Z");
    assert_eq!(manifest.record_format, RecordFormat::JsonLinesV1);
}

//
// Verify that manifest-related storage errors are typed and matchable without
// parsing display strings.
//
// Given representative storage errors for root and manifest failures,
// when callers pattern-match on them,
// then each error category should be identifiable by variant and payload.
#[test]
fn graph_storage_errors_are_matchable_by_variant() {
    let root_path = PathBuf::from("/tmp/graph-store");
    let version_error = GraphStorageError::UnsupportedStorageVersion {
        version: "999".to_owned(),
    };
    let corrupted_error = GraphStorageError::ManifestCorrupted {
        path: root_path.clone(),
        reason: "invalid json".to_owned(),
    };

    assert!(matches!(
    version_error,
    GraphStorageError::UnsupportedStorageVersion { version } if version == "999"
    ));
    assert!(matches!(
    corrupted_error,
    GraphStorageError::ManifestCorrupted { path, reason }
    if path == root_path && reason == "invalid json"
    ));
}

//
// Verify the supported happy path for manifest validation before storage root
// open loads catalog, node payloads, relationship payloads, or adjacency.
//
// Given a V1 manifest with JsonLinesV1 records and non-empty graph metadata,
// when the manifest is validated,
// then validation should succeed.
#[test]
fn validate_storage_manifest_accepts_supported_v1_json_lines_manifest() {
    let manifest = supported_manifest();

    validate_storage_manifest(&manifest).expect("supported manifest should validate");
}

//
// Verify that required manifest metadata is validated explicitly.
//
// Given a manifest with a whitespace-only graph ID,
// when the manifest is validated,
// then validation should fail with `InvalidManifest` rather than a generic error.
#[test]
fn validate_storage_manifest_rejects_empty_graph_id() {
    let mut manifest = supported_manifest();
    manifest.graph_id.value = " ".to_owned();

    let error = validate_storage_manifest(&manifest)
        .expect_err("empty graph ID should be rejected by manifest validation");

    assert!(matches!(
    error,
    GraphStorageError::InvalidManifest { reason } if reason.contains("graph_id")
    ));
}

//
// Verify the creation boundary for a local graph store root.
//
// Given a path that does not yet contain a graph store and a valid manifest,
// when a storage root is created,
// then the root handle should point to that path and a manifest should exist
// without requiring graph records to be loaded into memory.
#[test]
fn create_storage_root_writes_manifest_without_loading_graph_records() {
    let root_path = unique_root("create-root");
    let manifest = supported_manifest();

    let root = create_storage_root(root_path.clone(), manifest)
        .expect("storage root should be created from a supported manifest");

    assert_eq!(root.path(), root_path.as_path());
    assert!(root_path.join("manifest.json").is_file());
}

//
// Verify that an existing graph store can be reopened from only its root and
// manifest metadata.
//
// Given a storage root directory containing a valid manifest,
// when the root is opened,
// then the operation should return a root handle without reading full graph payloads.
#[test]
fn open_storage_root_reads_and_validates_manifest() {
    let root_path = unique_root("open-root");
    fs::create_dir_all(&root_path).expect("test root directory should be created");
    fs::write(
        root_path.join("manifest.json"),
        valid_manifest_json("graph--open-root"),
    )
    .expect("test manifest should be written");

    let root = open_storage_root(root_path.clone())
        .expect("existing storage root with valid manifest should open");

    assert_eq!(root.path(), root_path.as_path());
}

//
// Verify that manifest reads are separated from catalog and record reads.
//
// Given a storage root containing only a valid manifest,
// when the manifest is read,
// then the returned manifest should preserve compatibility metadata.
#[test]
fn read_storage_manifest_returns_manifest_metadata() {
    let root_path = unique_root("read-manifest");
    fs::create_dir_all(&root_path).expect("test root directory should be created");
    fs::write(
        root_path.join("manifest.json"),
        valid_manifest_json("graph--read-manifest"),
    )
    .expect("test manifest should be written");
    let root = storage_root(root_path);

    let manifest =
        read_storage_manifest(&root).expect("valid manifest should be readable from storage root");

    assert_eq!(manifest.storage_version, StorageVersion::V1);
    assert_eq!(manifest.graph_id.value, "graph--read-manifest");
    assert_eq!(manifest.record_format, RecordFormat::JsonLinesV1);
}

//
// Verify that missing manifest files produce an explicit manifest error.
//
// Given a storage root directory with no manifest file,
// when the manifest is read,
// then the storage layer should return `ManifestNotFound`.
#[test]
fn read_storage_manifest_reports_missing_manifest_explicitly() {
    let root_path = unique_root("missing-manifest");
    fs::create_dir_all(&root_path).expect("test root directory should be created");
    let root = storage_root(root_path.clone());

    let error = read_storage_manifest(&root)
        .expect_err("missing manifest should return a typed manifest error");

    assert!(matches!(
    error,
    GraphStorageError::ManifestNotFound { path } if path == root_path.join("manifest.json")
    ));
}

//
// Verify that corrupted manifest files produce an explicit corruption error.
//
// Given a storage root directory with invalid manifest content,
// when the manifest is read,
// then the storage layer should return `ManifestCorrupted`.
#[test]
fn read_storage_manifest_reports_corrupted_manifest_explicitly() {
    let root_path = unique_root("corrupted-manifest");
    fs::create_dir_all(&root_path).expect("test root directory should be created");
    fs::write(root_path.join("manifest.json"), "not a manifest")
        .expect("corrupted test manifest should be written");
    let root = storage_root(root_path);

    let error = read_storage_manifest(&root)
        .expect_err("corrupted manifest should return a typed manifest error");

    assert!(matches!(error, GraphStorageError::ManifestCorrupted { .. }));
}

//
// Verify that opening a missing storage root fails before any manifest or graph
// payload loading is attempted.
//
// Given a path with no storage root,
// when the root is opened,
// then the storage layer should return `StorageRootNotFound`.
#[test]
fn open_storage_root_reports_missing_root_explicitly() {
    let root_path = unique_root("missing-root");

    let error = open_storage_root(root_path.clone())
        .expect_err("missing storage root should return a typed root error");

    assert!(matches!(
    error,
    GraphStorageError::StorageRootNotFound { path } if path == root_path
    ));
}
