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
//! Criterion parse benchmark (audit §6 — seeds Epic 0011 MVP metrics).
//!
//! Measures `cypher_parser::parse_query` throughput across representative MVP
//! queries (read, filtered read, aggregation, and mutation). These numbers give
//! Epic 0011 a parser baseline to track regressions against.

use criterion::{Criterion, criterion_group, criterion_main};
use cypher_parser::parse_query;

const QUERIES: &[(&str, &str)] = &[
    ("match_return", "MATCH (n:Indicator) RETURN n"),
    (
        "filtered_read",
        "MATCH (n:Indicator) WHERE n.score > 10 RETURN n ORDER BY n.score LIMIT 3",
    ),
    (
        "aggregation",
        "MATCH (a:Actor)-[:AMPLIFIES]->(n:Narrative) RETURN count(n), SUM(n.score)",
    ),
    (
        "mutation",
        "CREATE (n:Indicator {name: 'alpha', score: 10}) RETURN n",
    ),
];

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("cypher_parse");
    for (name, query) in QUERIES {
        group.bench_function(*name, |b| {
            b.iter(|| {
                let _ = parse_query(std::hint::black_box(query));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
