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
//! Regenerate the Epic 0017 reproducibility report table.
//!
//! The benchmark is fully deterministic: this example must print the exact
//! table committed in `project-documents/feature-0017-learned-working-set/artifacts/0017-reproducibility-report.md`.
//!
//! ```text
//! cargo run -p graph-core --example epic_0017_report
//! ```

use graph_core::{
    fimi_multi_hop_benchmark_workload, render_working_set_benchmark_markdown,
    run_working_set_benchmark,
};

fn main() {
    let workload = fimi_multi_hop_benchmark_workload();
    let report = run_working_set_benchmark(&workload).expect("benchmark should run");
    print!("{}", render_working_set_benchmark_markdown(&report));
}
