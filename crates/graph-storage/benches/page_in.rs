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
//! Criterion page-in benchmark (audit §6 — seeds Epic 0011 MVP metrics).
//!
//! The dominant per-record cost of paging a graph record in from a cataloged
//! file offset is decoding the checksummed JSON-Lines envelope back into a typed
//! record (checksum verification + deserialization). The full file-backed pager
//! fixture depends on crate-internal catalog indexing helpers that are not part
//! of the public API, so this bench exercises the same hydration work through the
//! public codec: encode a node/relationship record once, then measure repeated
//! `decode_persisted_record_envelope` calls (the page-in hot path).

#![allow(clippy::unwrap_used)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use graph_core::{Graph, NodeInput, RelationshipInput};
use graph_storage::{
    EncodedRecord, JsonLinesRecordCodec, PersistedRecordKind, RecordFormat, StorageRef,
    StorageSegment, StorageVersion, create_node_record_envelope,
    create_relationship_record_envelope, decode_persisted_record_envelope,
    encode_persisted_record_envelope,
};

fn storage_ref(segment: StorageSegment) -> StorageRef {
    StorageRef {
        segment,
        offset: 128,
        length: 256,
        checksum: None,
    }
}

fn encoded_node_record(codec: &JsonLinesRecordCodec) -> EncodedRecord {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]))
        .expect("node should be created");
    let node = graph.get_node(&node_id).unwrap().unwrap();
    let envelope = create_node_record_envelope(
        &node,
        storage_ref(StorageSegment::NodeRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("node envelope should build");
    encode_persisted_record_envelope(codec, &envelope).expect("node record should encode")
}

fn encoded_relationship_record(codec: &JsonLinesRecordCodec) -> EncodedRecord {
    let mut graph = Graph::new();
    let source = graph.create_node(NodeInput::new(["Actor"])).unwrap();
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    let relationship_id = graph
        .create_relationship(RelationshipInput::new(source, "USES", target).unwrap())
        .unwrap();
    let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();
    let envelope = create_relationship_record_envelope(
        &relationship,
        storage_ref(StorageSegment::RelationshipRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("relationship envelope should build");
    encode_persisted_record_envelope(codec, &envelope).expect("relationship record should encode")
}

fn bench_page_in(c: &mut Criterion) {
    let codec = JsonLinesRecordCodec::default();
    let node_record = encoded_node_record(&codec);
    let relationship_record = encoded_relationship_record(&codec);

    let mut group = c.benchmark_group("record_page_in");

    group.bench_function("node", |b| {
        b.iter(|| {
            let _ = decode_persisted_record_envelope(
                &codec,
                black_box(&node_record),
                Some(PersistedRecordKind::Node),
            );
        });
    });

    group.bench_function("relationship", |b| {
        b.iter(|| {
            let _ = decode_persisted_record_envelope(
                &codec,
                black_box(&relationship_record),
                Some(PersistedRecordKind::Relationship),
            );
        });
    });

    group.finish();
}

criterion_group!(benches, bench_page_in);
criterion_main!(benches);
