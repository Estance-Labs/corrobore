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

//! Issue #327: object-storage backends and Parquet export.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs::{self, File},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use arrow_array::{Array, Float64Array, Int64Array, StringArray};
use graph_core::{
    ContentStoreError, Graph, NodeInput, ObjectStore, PropertyValue, RelationshipInput,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, MAIN_BRANCH, RecordFormat, SnapshotRequest, StorageManifest, StorageTimestamp,
    StorageVersion, create_consistent_snapshot, create_storage_root, export_snapshot_to_store,
    import_snapshot_from_store, open_storage_root, restore_consistent_snapshot,
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use storage_interchange::{UrlObjectStore, export_commit_parquet};

fn temp(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-327-{name}-{}-{unique}",
        std::process::id()
    ))
}

fn seeded_store(name: &str) -> (graph_storage::StorageRoot, CanonicalEngineStore) {
    let root = create_storage_root(
        temp(name),
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-327".to_owned(),
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
    .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current: Graph = previous.clone();
    let a = current
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("alpha".to_owned()))
                .with_property("score", PropertyValue::Integer(7))
                .with_property("weight", PropertyValue::Float(0.5))
                .with_property("tags", PropertyValue::StringList(vec!["c2".to_owned()])),
        )
        .unwrap();
    let b = current
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("beta".to_owned()))
                .with_property("score", PropertyValue::Integer(3))
                .with_property("weight", PropertyValue::Integer(2)),
        )
        .unwrap();
    let m = current
        .create_node(
            NodeInput::new(["Malware Family"])
                .with_property("name", PropertyValue::String("m".to_owned())),
        )
        .unwrap();
    current
        .create_relationship(RelationshipInput::new(a, "INDICATES", m.clone()).unwrap())
        .unwrap();
    current
        .create_relationship(RelationshipInput::new(b, "INDICATES", m).unwrap())
        .unwrap();
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new("tx--327").unwrap(),
            None,
        )
        .unwrap();
    (root, store)
}

#[test]
fn memory_and_file_urls_serve_the_content_plane() {
    for url in [
        "memory://",
        &format!("file://{}", temp("content").display()),
    ] {
        if let Some(path) = url.strip_prefix("file://") {
            fs::create_dir_all(path).unwrap();
        }
        let mut store = UrlObjectStore::open(url, Vec::<(String, String)>::new()).unwrap();
        store.put("aa/bb/digest", b"hello object store").unwrap();
        assert_eq!(store.get("aa/bb/digest").unwrap(), b"hello object store");
        assert_eq!(store.get_range("aa/bb/digest", 6..12).unwrap(), b"object");
        assert_eq!(store.head("aa/bb/digest").unwrap(), 18);
        assert!(matches!(
            store.get_range("aa/bb/digest", 10..40),
            Err(ContentStoreError::RangeOutOfBounds { .. })
        ));
        store.remove("aa/bb/digest").unwrap();
        assert!(matches!(
            store.get("aa/bb/digest"),
            Err(ContentStoreError::NotRetained(_))
        ));
        assert!(matches!(
            store.remove("aa/bb/digest"),
            Err(ContentStoreError::NotRetained(_))
        ));
        if let Some(path) = url.strip_prefix("file://") {
            let _ = fs::remove_dir_all(path);
        }
    }
    assert!(UrlObjectStore::open("ftp://nowhere", Vec::<(String, String)>::new()).is_err());
}

#[test]
fn snapshots_round_trip_through_an_object_store_with_their_history() {
    let (root, store) = seeded_store("snapshot");
    let head = {
        let mut store = store;
        let head = store.graph_head(MAIN_BRANCH).unwrap();
        let snapshot = temp("snapshot-local");
        create_consistent_snapshot(
            store.root(),
            &snapshot,
            SnapshotRequest {
                created_at: "2026-11-15T00:00:00Z".to_owned(),
                encryption_key_id: None,
                retention_hook: None,
            },
        )
        .unwrap();
        let bucket = temp("bucket");
        fs::create_dir_all(&bucket).unwrap();
        let mut remote = UrlObjectStore::open(
            &format!("file://{}", bucket.display()),
            Vec::<(String, String)>::new(),
        )
        .unwrap();
        export_snapshot_to_store(&snapshot, &mut remote, "backups/today").unwrap();

        let downloaded = temp("snapshot-downloaded");
        let report =
            import_snapshot_from_store(&remote, "backups/today", &downloaded, None).unwrap();
        assert!(report.verified_files > 0, "{report:?}");
        let restored = temp("restored");
        restore_consistent_snapshot(&downloaded, &restored, None).unwrap();
        let mut reopened = CanonicalEngineStore::open(
            open_storage_root(&restored).unwrap(),
            CanonicalStoreOptions::default(),
        )
        .unwrap();
        assert_eq!(reopened.graph_head(MAIN_BRANCH).unwrap(), head);

        assert!(
            import_snapshot_from_store(&remote, "backups/missing", temp("never"), None).is_err()
        );
        for path in [snapshot, bucket, downloaded, restored] {
            let _ = fs::remove_dir_all(path);
        }
        head
    };
    assert!(head.as_str().starts_with("gc"));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn a_commit_exports_to_typed_parquet_files_per_label_and_type() {
    let (root, mut store) = seeded_store("parquet");
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let destination = temp("parquet-out");
    let report = export_commit_parquet(&mut store, &head, &destination).unwrap();

    assert_eq!(report.total_rows, 5);
    let indicators = report
        .files
        .iter()
        .find(|file| file.name == "Indicator")
        .unwrap();
    assert_eq!(indicators.path, "nodes/Indicator.parquet");
    assert_eq!(indicators.rows, 2);
    assert!(indicators.typed_columns.contains(&"prop_score".to_owned()));
    assert!(indicators.typed_columns.contains(&"prop_weight".to_owned()));
    assert!(
        !indicators.typed_columns.contains(&"prop_tags".to_owned()),
        "lists stay in the JSON column"
    );
    assert!(
        report
            .files
            .iter()
            .any(|file| file.path == "nodes/Malware_Family.parquet")
    );
    assert!(destination.join("manifest.json").is_file());

    let reader = ParquetRecordBatchReaderBuilder::try_new(
        File::open(destination.join("nodes/Indicator.parquet")).unwrap(),
    )
    .unwrap()
    .build()
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let score = batch
        .column_by_name("prop_score")
        .unwrap()
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    let weight = batch
        .column_by_name("prop_weight")
        .unwrap()
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    let names = batch
        .column_by_name("prop_name")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let mut rows: Vec<(String, i64, f64)> = (0..batch.num_rows())
        .map(|row| {
            (
                names.value(row).to_owned(),
                score.value(row),
                weight.value(row),
            )
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        rows,
        vec![("alpha".to_owned(), 7, 0.5), ("beta".to_owned(), 3, 2.0)]
    );
    let properties = batch
        .column_by_name("properties")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!((0..batch.num_rows()).any(|row| properties.value(row).contains("\"tags\":[\"c2\"]")));

    let relationships = ParquetRecordBatchReaderBuilder::try_new(
        File::open(destination.join("relationships/INDICATES.parquet")).unwrap(),
    )
    .unwrap()
    .build()
    .unwrap()
    .next()
    .unwrap()
    .unwrap();
    assert_eq!(relationships.num_rows(), 2);
    assert!(relationships.column_by_name("source").unwrap().null_count() == 0);

    assert!(
        export_commit_parquet(&mut store, &head, &destination).is_err(),
        "never overwrites"
    );
    let _ = fs::remove_dir_all(destination);
    let _ = fs::remove_dir_all(root.path());
}
