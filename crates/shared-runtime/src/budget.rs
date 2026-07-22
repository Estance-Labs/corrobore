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
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime budget.
pub struct RuntimeBudget {
    /// Max query length.
    pub max_query_length: usize,
    /// Max parameter count.
    pub max_parameter_count: usize,
    /// Max loaded records.
    pub max_loaded_records: usize,
    /// Max returned records.
    pub max_returned_records: usize,
    /// Max mutation count.
    pub max_mutation_count: usize,
    /// Max payload bytes.
    pub max_payload_bytes: usize,
    /// Max execution time ms.
    pub max_execution_time_ms: u64,
}

impl RuntimeBudget {
    // Runtime budgets gate request execution at the runtime boundary and are
    // intentionally distinct from graph-core working-set expansion budgets.
    /// Strict default.
    pub fn strict_default() -> Self {
        Self {
            // Max query length.
            max_query_length: 8_192,
            // Max parameter count.
            max_parameter_count: 128,
            // Max loaded records.
            max_loaded_records: 50_000,
            // Max returned records.
            max_returned_records: 10_000,
            // Max mutation count.
            max_mutation_count: 5_000,
            // Max payload bytes.
            max_payload_bytes: 4 * 1024 * 1024,
            // Max execution time ms.
            max_execution_time_ms: 20_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime budget usage.
pub struct RuntimeBudgetUsage {
    /// Query length.
    pub query_length: usize,
    /// Parameter count.
    pub parameter_count: usize,
    /// Loaded record count.
    pub loaded_record_count: usize,
    /// Returned record count.
    pub returned_record_count: usize,
    /// Mutation count.
    pub mutation_count: usize,
    /// Payload bytes.
    pub payload_bytes: usize,
    /// Execution time ms.
    pub execution_time_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime budget exceeded.
pub struct RuntimeBudgetExceeded {
    /// Dimension.
    pub dimension: &'static str,
    /// Limit.
    pub limit: usize,
    /// Actual.
    pub actual: usize,
    /// Fix hint.
    pub fix_hint: String,
}
