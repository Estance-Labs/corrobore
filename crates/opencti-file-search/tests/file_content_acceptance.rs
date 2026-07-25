// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use opencti_access::{AccessContext, AccessMetadata};
use opencti_file_search::{
    ChunkProvenance, ExtractionErrorCode, ExtractionLimits, FileContentIndex,
    FileContentIndexSettings, FileContentQuery, FileDescriptor, FileExtractionRequest,
    FileExtractionWorker, FileJobStore, FileLifecycleEvent, FilesystemBlobSource, JobDisposition,
    WorkerRunOutcome, extract_file,
};
use opencti_search::FullTextMatchMode;
use sha2::{Digest, Sha256};
use zip::{ZipWriter, write::SimpleFileOptions};

const CURSOR_KEY: &[u8] = b"issue-48-file-content-cursor-key-v1";

fn root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-48-{name}-{}-{unique}",
        std::process::id()
    ))
}

fn descriptor(
    file_id: &str,
    name: &str,
    mime_type: &str,
    content: &[u8],
    version: u64,
    marking: &str,
) -> FileDescriptor {
    FileDescriptor {
        file_id: file_id.to_owned(),
        source_object_id: "report--synthetic".to_owned(),
        blob_key: format!("opencti/{file_id}/{version}"),
        name: name.to_owned(),
        mime_type: mime_type.to_owned(),
        content_hash: format!("{:x}", Sha256::digest(content)),
        version,
        access: AccessMetadata {
            marking_ids: vec![marking.to_owned()],
            owner_ids: vec!["identity--owner".to_owned()],
            ..AccessMetadata::default()
        },
    }
}

fn request(descriptor: FileDescriptor, content: Vec<u8>) -> FileExtractionRequest {
    FileExtractionRequest {
        descriptor,
        content,
    }
}

fn limits() -> ExtractionLimits {
    ExtractionLimits {
        max_input_bytes: 1_000_000,
        max_extracted_bytes: 200_000,
        max_pages: 10,
        max_sheets: 10,
        max_rows_per_sheet: 100,
        max_cells: 1_000,
        max_chunks: 100,
        max_chunk_chars: 4_096,
    }
}

fn clear_access() -> AccessContext {
    AccessContext {
        subject_id: "identity--owner".to_owned(),
        marking_ids: vec!["marking--clear".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
        ..AccessContext::default()
    }
}

fn system_access() -> AccessContext {
    AccessContext {
        subject_id: "system".to_owned(),
        roles: vec!["system".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
        ..AccessContext::default()
    }
}

fn minimal_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn minimal_xlsx() -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default();
    archive.start_file("[Content_Types].xml", options).unwrap();
    archive
        .write_all(br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#)
        .unwrap();
    archive.start_file("_rels/.rels", options).unwrap();
    archive
        .write_all(br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)
        .unwrap();
    archive.start_file("xl/workbook.xml", options).unwrap();
    archive
        .write_all(br#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Indicators" sheetId="1" r:id="rId1"/></sheets></workbook>"#)
        .unwrap();
    archive
        .start_file("xl/_rels/workbook.xml.rels", options)
        .unwrap();
    archive
        .write_all(br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#)
        .unwrap();
    archive
        .start_file("xl/worksheets/sheet1.xml", options)
        .unwrap();
    archive
        .write_all(br#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>indicator</t></is></c><c r="B1" t="inlineStr"><is><t>malware.example.org</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>ipv4</t></is></c><c r="B2" t="inlineStr"><is><t>192.0.2.12</t></is></c></row></sheetData></worksheet>"#)
        .unwrap();
    archive.finish().unwrap().into_inner()
}

#[test]
fn representative_formats_extract_and_search_with_exact_provenance() {
    let cases = [
        (
            "report.txt",
            "text/plain",
            b"Synthetic report malware.example.org".to_vec(),
            "malware.example.org",
            ChunkProvenance::default(),
        ),
        (
            "report.csv",
            "text/csv",
            b"type,value\nindicator,malware.example.org\n".to_vec(),
            "malware.example.org",
            ChunkProvenance {
                row_start: Some(1),
                row_end: Some(2),
                ..ChunkProvenance::default()
            },
        ),
        (
            "report.html",
            "text/html",
            b"<html><body><h1>Report</h1><script>secret()</script><p>malware.example.org</p></body></html>".to_vec(),
            "malware.example.org",
            ChunkProvenance::default(),
        ),
        (
            "report.pdf",
            "application/pdf",
            minimal_pdf("PDF report malware.example.org"),
            "malware.example.org",
            ChunkProvenance {
                page: Some(1),
                ..ChunkProvenance::default()
            },
        ),
        (
            "report.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            minimal_xlsx(),
            "malware.example.org",
            ChunkProvenance {
                sheet: Some("Indicators".to_owned()),
                row_start: Some(1),
                row_end: Some(2),
                ..ChunkProvenance::default()
            },
        ),
    ];

    let path = root("representative-formats");
    let mut artifacts = Vec::new();
    let mut mime_types = Vec::new();
    for (index, (name, mime_type, content, needle, expected_provenance)) in
        cases.into_iter().enumerate()
    {
        let artifact = extract_file(
            request(
                descriptor(
                    &format!("file--format-{index}"),
                    name,
                    mime_type,
                    &content,
                    1,
                    "marking--clear",
                ),
                content,
            ),
            &limits(),
        )
        .unwrap();
        assert_eq!(artifact.descriptor.name, name);
        assert!(
            artifact
                .chunks
                .iter()
                .any(|chunk| chunk.text.contains(needle))
        );
        assert!(
            artifact
                .chunks
                .iter()
                .any(|chunk| chunk.provenance == expected_provenance)
        );
        mime_types.push(mime_type.to_owned());
        artifacts.push(artifact);
    }
    let index = FileContentIndex::open(
        path.clone(),
        FileContentIndexSettings::testing(CURSOR_KEY.to_vec()),
    )
    .unwrap();
    index.rebuild(artifacts).unwrap();
    for mime_type in mime_types {
        let page = index
            .search(
                &FileContentQuery {
                    text: "malware.example.org".to_owned(),
                    mime_types: vec![mime_type],
                    ..FileContentQuery::default()
                },
                &clear_access(),
            )
            .unwrap();
        assert_eq!(page.total, 1);
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn malformed_unsupported_encrypted_and_oversized_inputs_fail_with_bounded_diagnostics() {
    let cases = [
        (
            "application/octet-stream",
            b"unknown".to_vec(),
            ExtractionErrorCode::UnsupportedFormat,
        ),
        (
            "application/pdf",
            b"%PDF-1.4 malformed".to_vec(),
            ExtractionErrorCode::Malformed,
        ),
        (
            "application/pdf",
            b"%PDF-1.4\n1 0 obj << /Encrypt 2 0 R >>".to_vec(),
            ExtractionErrorCode::Encrypted,
        ),
        (
            "text/plain",
            vec![b'x'; 1_000_001],
            ExtractionErrorCode::InputLimitExceeded,
        ),
    ];
    for (mime_type, content, expected) in cases {
        let error = extract_file(
            request(
                descriptor(
                    "file--invalid",
                    "invalid",
                    mime_type,
                    &content,
                    1,
                    "marking--clear",
                ),
                content,
            ),
            &limits(),
        )
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(error.diagnostic.len() <= 256);
        assert!(!error.diagnostic.contains("unknown"));
    }
}

#[test]
fn durable_jobs_are_idempotent_resume_expired_leases_and_quarantine_bounded_failures() {
    let path = root("jobs");
    let content = b"durable malware.example.org".to_vec();
    let descriptor = descriptor(
        "file--durable",
        "durable.txt",
        "text/plain",
        &content,
        1,
        "marking--clear",
    );
    let mut store = FileJobStore::open(path.clone(), 3, 1_000).unwrap();
    let first = store.enqueue(descriptor.clone(), 10).unwrap();
    let replay = store.enqueue(descriptor, 11).unwrap();
    assert_eq!(first.job_id, replay.job_id);
    assert!(!first.duplicate);
    assert!(replay.duplicate);

    let lease = store.lease_next(20).unwrap().unwrap();
    drop(store);
    let mut reopened = FileJobStore::open(path.clone(), 3, 1_000).unwrap();
    assert!(reopened.lease_next(500).unwrap().is_none());
    let resumed = reopened.lease_next(1_021).unwrap().unwrap();
    assert_eq!(resumed.job_id, lease.job_id);
    assert_ne!(resumed.lease_token, lease.lease_token);

    let retry = reopened
        .fail(
            &resumed,
            ExtractionErrorCode::ResourceLimitExceeded,
            "parser resource limit",
            1_022,
        )
        .unwrap();
    assert_eq!(retry, JobDisposition::RetryScheduled);
    let second = reopened.lease_next(2_100).unwrap().unwrap();
    let quarantined = reopened
        .fail(
            &second,
            ExtractionErrorCode::ResourceLimitExceeded,
            "parser resource limit",
            2_101,
        )
        .unwrap();
    assert_eq!(quarantined, JobDisposition::Quarantined);
    let metrics = reopened.metrics(3_000);
    assert_eq!(metrics.queue_depth, 0);
    assert_eq!(metrics.retries, 2);
    assert_eq!(metrics.quarantines, 1);
    assert_eq!(metrics.failures, 3);

    fs::remove_dir_all(path).unwrap();
}

#[test]
fn concurrent_server_and_worker_updates_do_not_lose_jobs() {
    let path = root("concurrent-jobs");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let content = format!("concurrent content {index}").into_bytes();
                let mut store = FileJobStore::open(path, 3, 1_000).unwrap();
                barrier.wait();
                store
                    .enqueue(
                        descriptor(
                            &format!("file--concurrent-{index}"),
                            &format!("concurrent-{index}.txt"),
                            "text/plain",
                            &content,
                            1,
                            "marking--clear",
                        ),
                        10,
                    )
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }

    let store = FileJobStore::open(path.clone(), 3, 1_000).unwrap();
    assert_eq!(store.metrics(10).queue_depth, 2);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn indexed_content_is_access_aware_highlighted_filtered_and_rebuildable() {
    let path = root("search");
    let clear = b"Clear report containing malware.example.org and 192.0.2.12".to_vec();
    let amber = b"Amber report containing malware.example.org and a hidden detail".to_vec();
    let clear_artifact = extract_file(
        request(
            descriptor(
                "file--00000000-0000-4000-8000-000000000070",
                "synthetic-report.txt",
                "text/plain",
                &clear,
                1,
                "marking--clear",
            ),
            clear,
        ),
        &limits(),
    )
    .unwrap();
    let amber_artifact = extract_file(
        request(
            descriptor(
                "file--hidden",
                "hidden-report.txt",
                "text/plain",
                &amber,
                1,
                "marking--amber",
            ),
            amber,
        ),
        &limits(),
    )
    .unwrap();

    let mut store = FileJobStore::open(path.join("metadata"), 3, 1_000).unwrap();
    store.publish_artifact(clear_artifact, 10).unwrap();
    store.publish_artifact(amber_artifact, 11).unwrap();
    let index = FileContentIndex::open(
        path.join("index"),
        FileContentIndexSettings {
            schema_version: "opencti-file-content-v1".to_owned(),
            cursor_key: CURSOR_KEY.to_vec(),
            writer_memory_bytes: 15_000_000,
            max_candidates: 10_000,
            snippet_chars: 120,
        },
    )
    .unwrap();
    index.rebuild(store.artifacts().unwrap()).unwrap();

    let page = index
        .search(
            &FileContentQuery {
                text: "malware.example.org".to_owned(),
                mode: FullTextMatchMode::Term,
                mime_types: vec!["text/plain".to_owned()],
                owner_ids: vec!["identity--owner".to_owned()],
                source_object_ids: vec!["report--synthetic".to_owned()],
                limit: 10,
                cursor: None,
            },
            &clear_access(),
        )
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(
        page.hits[0].id,
        "file--00000000-0000-4000-8000-000000000070"
    );
    assert_eq!(page.authorization_denials, 1);
    assert!(page.hits[0].snippet.as_deref().unwrap().contains("<mark>"));
    assert_eq!(page.hits[0].highlights, vec!["malware.example.org"]);
    assert_eq!(
        page.hits[0]
            .metadata
            .get("source_object_id")
            .map(String::as_str),
        Some("report--synthetic")
    );

    fs::remove_dir_all(index.index_path()).unwrap();
    let rebuilt = index.rebuild(store.artifacts().unwrap()).unwrap();
    assert!(rebuilt.generation_changed);
    assert_eq!(
        index
            .search(
                &FileContentQuery {
                    text: "192.0.2.12".to_owned(),
                    ..FileContentQuery::default()
                },
                &clear_access(),
            )
            .unwrap()
            .total,
        1
    );

    fs::remove_dir_all(path).unwrap();
}

#[test]
fn replacement_delete_merge_and_policy_changes_remove_stale_visibility() {
    let path = root("lifecycle");
    let old = b"old-beacon.example".to_vec();
    let new = b"new-beacon.example".to_vec();
    let mut store = FileJobStore::open(path.join("metadata"), 3, 1_000).unwrap();
    store
        .publish_artifact(
            extract_file(
                request(
                    descriptor(
                        "file--lifecycle",
                        "lifecycle.txt",
                        "text/plain",
                        &old,
                        1,
                        "marking--clear",
                    ),
                    old,
                ),
                &limits(),
            )
            .unwrap(),
            10,
        )
        .unwrap();
    store
        .apply_lifecycle(
            FileLifecycleEvent::Replace {
                descriptor: descriptor(
                    "file--lifecycle",
                    "lifecycle.txt",
                    "text/plain",
                    &new,
                    2,
                    "marking--clear",
                ),
                content: new,
            },
            &limits(),
            20,
        )
        .unwrap();
    store
        .apply_lifecycle(
            FileLifecycleEvent::PolicyChange {
                file_id: "file--lifecycle".to_owned(),
                access: AccessMetadata {
                    marking_ids: vec!["marking--amber".to_owned()],
                    owner_ids: vec!["identity--owner".to_owned()],
                    ..AccessMetadata::default()
                },
            },
            &limits(),
            25,
        )
        .unwrap();
    store
        .apply_lifecycle(
            FileLifecycleEvent::Merge {
                source_file_id: "file--lifecycle".to_owned(),
                target_file_id: "file--merged".to_owned(),
            },
            &limits(),
            30,
        )
        .unwrap();

    let index = FileContentIndex::open(
        path.join("index"),
        FileContentIndexSettings::testing(CURSOR_KEY.to_vec()),
    )
    .unwrap();
    index.rebuild(store.artifacts().unwrap()).unwrap();
    assert_eq!(
        index
            .search(
                &FileContentQuery {
                    text: "new-beacon.example".to_owned(),
                    ..FileContentQuery::default()
                },
                &clear_access(),
            )
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        index
            .search(
                &FileContentQuery {
                    text: "new-beacon.example".to_owned(),
                    ..FileContentQuery::default()
                },
                &system_access(),
            )
            .unwrap()
            .hits[0]
            .id,
        "file--merged"
    );

    store
        .apply_lifecycle(
            FileLifecycleEvent::Delete {
                file_id: "file--merged".to_owned(),
            },
            &limits(),
            40,
        )
        .unwrap();
    index.rebuild(store.artifacts().unwrap()).unwrap();
    assert_eq!(
        index
            .search(
                &FileContentQuery {
                    text: "new-beacon.example".to_owned(),
                    ..FileContentQuery::default()
                },
                &system_access(),
            )
            .unwrap()
            .total,
        0
    );

    fs::remove_dir_all(path).unwrap();
}

#[test]
fn dedicated_worker_leases_fetches_extracts_and_publishes_with_metrics() {
    let path = root("worker");
    let blob_root = path.join("blobs");
    fs::create_dir_all(blob_root.join("opencti")).unwrap();
    let content = b"Worker extracted malware.example.org".to_vec();
    fs::write(blob_root.join("opencti/report.txt"), &content).unwrap();
    let mut descriptor = descriptor(
        "file--worker",
        "report.txt",
        "text/plain",
        &content,
        1,
        "marking--clear",
    );
    descriptor.blob_key = "opencti/report.txt".to_owned();
    let mut store = FileJobStore::open(path.join("metadata"), 3, 5_000).unwrap();
    store.enqueue(descriptor, 10).unwrap();
    let mut worker =
        FileExtractionWorker::new(FilesystemBlobSource::new(blob_root), limits(), 30_000).unwrap();
    let outcome = worker.run_once(&mut store, 20).unwrap();
    assert!(
        matches!(outcome, WorkerRunOutcome::Published { extracted_bytes, .. } if extracted_bytes > 0)
    );
    assert_eq!(store.artifacts().unwrap().len(), 1);
    let metrics = store.metrics(30);
    assert_eq!(metrics.completed_jobs, 1);
    assert_eq!(metrics.queue_depth, 0);
    assert!(metrics.extracted_bytes > 0);

    fs::remove_dir_all(path).unwrap();
}
