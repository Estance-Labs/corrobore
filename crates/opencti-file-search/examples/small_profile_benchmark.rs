// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Reproducible issue #48 file-content search benchmark.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use opencti_access::{AccessContext, AccessMetadata};
use opencti_file_search::{
    ChunkProvenance, ExtractedChunk, ExtractionArtifact, FileContentIndex,
    FileContentIndexSettings, FileContentQuery, FileDescriptor,
};
use serde_json::json;

const FILES: usize = 100_000;
const WARMUP_ITERATIONS: usize = 20;
const MEASUREMENT_ITERATIONS: usize = 60;
const WRITER_MEMORY_BYTES: usize = 50_000_000;
const MAX_CANDIDATES: usize = 100_000;

fn main() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "corrobore-file-search-small-profile-{}-{unique}",
        std::process::id()
    ));
    let _temporary_directory = TemporaryDirectory(root.clone());
    let index = FileContentIndex::open(
        root.clone(),
        FileContentIndexSettings {
            schema_version: "opencti-file-content-v1".to_owned(),
            cursor_key: b"issue-48-file-benchmark-cursor-key-v1".to_vec(),
            writer_memory_bytes: WRITER_MEMORY_BYTES,
            max_candidates: MAX_CANDIDATES,
            snippet_chars: 240,
        },
    )?;
    let artifacts = (0..FILES).map(synthetic_artifact).collect::<Vec<_>>();

    let ingestion_started = Instant::now();
    index.rebuild(artifacts)?;
    let ingestion_seconds = ingestion_started.elapsed().as_secs_f64();
    let query = FileContentQuery {
        text: "documentation beacon".to_owned(),
        limit: 100,
        ..FileContentQuery::default()
    };
    let access = AccessContext {
        subject_id: "benchmark--system".to_owned(),
        roles: vec!["system".to_owned()],
        ..AccessContext::default()
    };

    let first_page = index.search(&query, &access)?;
    for _ in 1..WARMUP_ITERATIONS {
        index.search(&query, &access)?;
    }
    let mut latencies_micros = Vec::with_capacity(MEASUREMENT_ITERATIONS);
    let measured_started = Instant::now();
    for _ in 0..MEASUREMENT_ITERATIONS {
        let started = Instant::now();
        index.search(&query, &access)?;
        latencies_micros.push(started.elapsed().as_micros() as u64);
    }
    let measured_seconds = measured_started.elapsed().as_secs_f64();
    latencies_micros.sort_unstable();
    let output = json!({
        "schema_version": 1,
        "engine": "corrobore-file-content-tantivy-0.26.1",
        "profile": {
            "id": "small-file-content",
            "files": FILES,
            "matching_files": first_page.total,
            "workload": "phrase-file-content",
            "warmup_iterations": WARMUP_ITERATIONS,
            "measurement_iterations": MEASUREMENT_ITERATIONS,
            "concurrency": 1
        },
        "ingestion": {
            "elapsed_seconds": ingestion_seconds,
            "throughput_files_per_second": FILES as f64 / ingestion_seconds
        },
        "metrics": {
            "latency_p50_ms": percentile_millis(&latencies_micros, 50),
            "latency_p95_ms": percentile_millis(&latencies_micros, 95),
            "latency_p99_ms": percentile_millis(&latencies_micros, 99),
            "throughput_ops_per_second": MEASUREMENT_ITERATIONS as f64 / measured_seconds,
            "resident_memory_bytes": process_resident_bytes(),
            "disk_bytes": directory_bytes(&root),
            "writer_memory_bytes": WRITER_MEMORY_BYTES,
            "max_candidates": MAX_CANDIDATES
        }
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn synthetic_artifact(number: usize) -> ExtractionArtifact {
    let text = if number.is_multiple_of(1_024) {
        format!("Synthetic documentation beacon in file {number:06}")
    } else {
        format!("Synthetic attachment observation {number:06}")
    };
    ExtractionArtifact {
        descriptor: FileDescriptor {
            file_id: format!("file--{number:06}"),
            source_object_id: format!("report--{:06}", number / 4),
            blob_key: format!("benchmark/{number:06}.txt"),
            name: format!("attachment-{number:06}.txt"),
            mime_type: "text/plain".to_owned(),
            content_hash: format!("{number:064x}"),
            version: 1,
            access: AccessMetadata::default(),
        },
        extracted_bytes: text.len() as u64,
        chunks: vec![ExtractedChunk {
            ordinal: 0,
            text,
            provenance: ChunkProvenance::default(),
        }],
    }
}

fn percentile_millis(sorted_micros: &[u64], percentile: usize) -> f64 {
    let rank = sorted_micros
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1);
    sorted_micros[rank] as f64 / 1_000.0
}

fn process_resident_bytes() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kilobytes| kilobytes.saturating_mul(1_024))
}

fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            entry.metadata().map_or(0, |metadata| {
                if metadata.is_dir() {
                    directory_bytes(&entry.path())
                } else {
                    metadata.len()
                }
            })
        })
        .sum()
}
