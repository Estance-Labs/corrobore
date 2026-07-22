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
use super::*;

/// Completion state for a budgeted expansion attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionResultStatus {
    /// Expansion completed within the requested bounds.
    Complete,
    /// Expansion produced a bounded subset and stopped before loading all candidates.
    Partial,
}

/// Result payload for working-set expansion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpansionResult {
    pub(crate) working_set_id: WorkingSetId,
    pub(crate) status: ExpansionResultStatus,
    pub(crate) usage: ExpansionBudgetUsage,
    pub(crate) explanation: WorkingSetExplanation,
    pub(crate) budget_error: Option<ExpansionBudgetExceeded>,
    #[serde(default)]
    pub(crate) supernode_error: Option<SupernodeExpansionBlocked>,
}

impl ExpansionResult {
    /// Build a result payload for an expansion attempt.
    pub fn new(
        working_set_id: WorkingSetId,
        status: ExpansionResultStatus,
        usage: ExpansionBudgetUsage,
        explanation: WorkingSetExplanation,
        budget_error: Option<ExpansionBudgetExceeded>,
    ) -> Self {
        Self {
            working_set_id,
            status,
            usage,
            explanation,
            budget_error,
            // Supernode error.
            supernode_error: None,
        }
    }

    /// Return a copy marked partial because a supernode policy blocked expansion.
    ///
    ///
    /// preserve the phase 1 result seam for without wiring runtime
    /// high-degree-node detection into the current expansion loop yet.
    pub fn with_supernode_error(mut self, error: SupernodeExpansionBlocked) -> Self {
        self.status = ExpansionResultStatus::Partial;
        self.supernode_error = Some(error);
        self
    }

    /// Return the working-set ID affected by the expansion attempt.
    pub fn working_set_id(&self) -> &WorkingSetId {
        &self.working_set_id
    }

    /// Return whether expansion completed or stopped with a partial result.
    pub fn status(&self) -> ExpansionResultStatus {
        self.status
    }

    /// Return consumed budget counters for the expansion attempt.
    pub fn usage(&self) -> &ExpansionBudgetUsage {
        &self.usage
    }

    /// Return structured explanation metadata for the expansion attempt.
    pub fn explanation(&self) -> &WorkingSetExplanation {
        &self.explanation
    }

    /// Return the budget error that caused a partial result, when one exists.
    pub fn budget_error(&self) -> Option<&ExpansionBudgetExceeded> {
        self.budget_error.as_ref()
    }

    /// Return the supernode error that caused a partial result, when one exists.
    pub fn supernode_error(&self) -> Option<&SupernodeExpansionBlocked> {
        self.supernode_error.as_ref()
    }
}
