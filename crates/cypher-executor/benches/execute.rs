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
//! Criterion execute benchmark (audit §6 — seeds Epic 0011 MVP metrics).
//!
//! Measures end-to-end `CypherPipelineExecutor::execute` cost (parse + plan +
//! run) over a small seeded graph for a read query and a filtered relationship
//! traversal. Gives Epic 0011 an execution baseline separate from raw parsing.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy};
use graph_core::{Graph, NodeInput, PropertyValue, RecordStatus, RelationshipInput};

fn seeded_graph() -> Graph {
    let mut graph = Graph::new();
    let actor = graph
        .create_node(
            NodeInput::new(["Actor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("alpha".to_owned())),
        )
        .expect("actor node should be created");
    let narrative = graph
        .create_node(
            NodeInput::new(["Narrative"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("n1".to_owned()))
                .with_property("score", PropertyValue::Integer(42)),
        )
        .expect("narrative node should be created");
    graph
        .create_relationship(
            RelationshipInput::new(actor, "AMPLIFIES", narrative)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("relationship should be created");
    graph
}

fn bench_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("cypher_execute");

    group.bench_function("scan_return", |b| {
        b.iter_batched(
            || {
                CypherPipelineExecutor::with_graph(
                    ExecutionPolicy::strict_default(),
                    seeded_graph(),
                )
            },
            |mut executor| {
                let _ = executor.execute(black_box("MATCH (n) RETURN n LIMIT 1"));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.bench_function("relationship_traversal", |b| {
        b.iter_batched(
            || CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), seeded_graph()),
            |mut executor| {
                let _ = executor.execute(black_box(
                    "MATCH (a:Actor)-[:AMPLIFIES]->(n:Narrative) WHERE a.name = 'alpha' RETURN a, n",
                ));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_execute);
criterion_main!(benches);
