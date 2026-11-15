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

//! Criterion benchmark of vector query latency (issue #342): the exact index
//! against the HNSW graph, top-10 cosine and L2 queries over seeded random
//! 64-dimension vectors, at 10,000 and 50,000 vectors.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use graph_storage::{ExactVectorIndex, HnswParams, HnswVectorIndex, VectorIndex, VectorMetric};

const DIMENSIONS: usize = 64;
const K: usize = 10;

/// SplitMix64, so every run benchmarks the same vectors.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn vector(&mut self) -> Vec<f32> {
        (0..DIMENSIONS)
            .map(|_| ((self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0) as f32)
            .collect()
    }
}

fn vector_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("vector_search");
    group.sample_size(30);
    for size in [10_000_usize, 50_000] {
        let mut rng = Rng(342);
        let vectors: Vec<(String, Vec<f32>)> = (0..size)
            .map(|index| (format!("node--{index:06}"), rng.vector()))
            .collect();
        let queries: Vec<Vec<f32>> = (0..64).map(|_| rng.vector()).collect();
        let mut exact = ExactVectorIndex::default();
        for (id, vector) in &vectors {
            exact.upsert(id.clone(), vector.clone());
        }
        for metric in [VectorMetric::Cosine, VectorMetric::L2] {
            let mut hnsw = HnswVectorIndex::new(metric, HnswParams::default());
            for (id, vector) in &vectors {
                hnsw.upsert(id.clone(), vector.clone());
            }
            let label = format!("{metric:?}/{size}");
            let mut next = 0_usize;
            group.bench_function(BenchmarkId::new("exact", &label), |bencher| {
                bencher.iter(|| {
                    next = (next + 1) % queries.len();
                    black_box(exact.search(black_box(&queries[next]), K, metric))
                });
            });
            group.bench_function(BenchmarkId::new("hnsw", &label), |bencher| {
                bencher.iter(|| {
                    next = (next + 1) % queries.len();
                    black_box(hnsw.search(black_box(&queries[next]), K, metric))
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, vector_search);
criterion_main!(benches);
