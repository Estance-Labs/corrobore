// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Reproducible issue #46 benchmark for the OpenCTI small profile.

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::PathBuf,
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use opencti_access::{AccessContext, AccessMetadata};
use opencti_search::{
    FullTextDocument, FullTextIndex, FullTextIndexSettings, FullTextMatchMode, FullTextQuery,
    FullTextRecordClass,
};
use serde_json::json;

const OBJECTS: usize = 100_000;
const RELATIONSHIPS: usize = 500_000;
const WARMUP_ITERATIONS: usize = 20;
const MEASUREMENT_ITERATIONS: usize = 60;
const WRITER_MEMORY_BYTES: usize = 50_000_000;
const MAX_CANDIDATES: usize = 100_000;

fn main() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "corrobore-opencti-search-small-profile-{}-{unique}",
        std::process::id()
    ));
    let _temporary_directory = TemporaryDirectory(root.clone());
    let index = FullTextIndex::open(
        root.clone(),
        FullTextIndexSettings {
            schema_version: "opencti-full-text-v1".to_owned(),
            cursor_key: b"issue-46-small-profile-cursor-key-32-bytes".to_vec(),
            writer_memory_bytes: WRITER_MEMORY_BYTES,
            max_candidates: MAX_CANDIDATES,
        },
    )?;
    let documents = (0..OBJECTS.saturating_add(RELATIONSHIPS))
        .map(synthetic_document)
        .collect::<Vec<_>>();

    let ingestion_started = Instant::now();
    index.rebuild(&documents)?;
    let ingestion_seconds = ingestion_started.elapsed().as_secs_f64();
    let query = FullTextQuery {
        text: "documentation beacon".to_owned(),
        mode: FullTextMatchMode::Phrase,
        fields: vec!["name".to_owned()],
        kinds: Vec::new(),
        filters: Vec::new(),
        limit: 100,
        cursor: None,
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
    let stats = index.storage_stats()?;
    let output = json!({
        "schema_version": 1,
        "engine": "corrobore-tantivy-0.26.1",
        "profile": "small",
        "objects": OBJECTS,
        "relationships": RELATIONSHIPS,
        "documents": OBJECTS.saturating_add(RELATIONSHIPS),
        "matching_documents": first_page.total,
        "workload": "phrase-full-text",
        "warmup_iterations": WARMUP_ITERATIONS,
        "measurement_iterations": MEASUREMENT_ITERATIONS,
        "ingestion": {
            "elapsed_seconds": ingestion_seconds,
            "throughput_docs_per_second":
                (OBJECTS.saturating_add(RELATIONSHIPS) as f64) / ingestion_seconds
        },
        "metrics": {
            "latency_p50_ms": percentile_millis(&latencies_micros, 50),
            "latency_p95_ms": percentile_millis(&latencies_micros, 95),
            "latency_p99_ms": percentile_millis(&latencies_micros, 99),
            "throughput_ops_per_second":
                (MEASUREMENT_ITERATIONS as f64) / measured_seconds,
            "resident_memory_bytes": process_resident_bytes(),
            "writer_memory_bytes": stats.writer_memory_bytes,
            "disk_bytes": stats.disk_bytes,
            "max_candidates": stats.max_candidates
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

fn synthetic_document(number: usize) -> FullTextDocument {
    let record_class = if number < OBJECTS {
        FullTextRecordClass::Object
    } else {
        FullTextRecordClass::Relationship
    };
    let kind = if record_class == FullTextRecordClass::Object {
        "indicator"
    } else {
        "indicates"
    };
    let name = if number.is_multiple_of(4_096) {
        format!("Synthetic documentation beacon {number:06}")
    } else {
        format!("Synthetic observation {number:06}")
    };
    FullTextDocument {
        id: format!(
            "{}--{number:06}",
            if record_class == FullTextRecordClass::Object {
                "indicator"
            } else {
                "relationship"
            }
        ),
        record_class,
        kind: kind.to_owned(),
        revision: 1,
        fields: BTreeMap::from([("name".to_owned(), vec![name])]),
        access: AccessMetadata::default(),
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
