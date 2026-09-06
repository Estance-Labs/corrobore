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

/// Epistemic lifecycle state for first-class claim records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    /// Candidate extracted claim that is not yet explicitly asserted.
    Candidate,
    /// Explicitly asserted claim by an actor or processing layer.
    Asserted,
    /// Claim currently has explicit supporting context.
    Supported,
    /// Claim is actively disputed but not resolved.
    Disputed,
    /// Claim is explicitly contradicted by another claim or evidence.
    Contradicted,
    /// Claim has been replaced by a newer or more precise claim.
    Superseded,
    /// Claim has been retracted but must remain available in history.
    Retracted,
    /// Claim has been rejected with explicit negative adjudication.
    Rejected,
    /// Claim has been validated by deterministic policy or analyst process.
    Validated,
    /// Claim remains unresolved despite available context.
    Unresolved,
}

impl ClaimStatus {
    /// Validate a lifecycle transition and return a typed error when rejected.
    pub fn ensure_valid_transition(from: Self, to: Self) -> Result<(), GraphError> {
        if from == to || is_transition_allowed(from, to) {
            return Ok(());
        }

        Err(GraphError::InvalidClaimStatusTransition { from, to })
    }
}

fn is_transition_allowed(from: ClaimStatus, to: ClaimStatus) -> bool {
    use ClaimStatus::*;

    match from {
        Candidate => matches!(to, Asserted | Rejected | Retracted | Unresolved),
        Asserted => matches!(
            to,
            Supported
                | Disputed
                | Contradicted
                | Superseded
                | Retracted
                | Rejected
                | Validated
                | Unresolved
        ),
        Supported => matches!(
            to,
            Disputed | Contradicted | Superseded | Retracted | Rejected | Validated | Unresolved
        ),
        Disputed => matches!(
            to,
            Supported | Contradicted | Superseded | Retracted | Rejected | Validated | Unresolved
        ),
        Contradicted => matches!(
            to,
            Disputed | Superseded | Retracted | Rejected | Unresolved
        ),
        Unresolved => matches!(
            to,
            Supported | Disputed | Contradicted | Superseded | Retracted | Rejected | Validated
        ),
        // A deterministic verification failure is an authority-boundary
        // event: a previously actionable claim must be able to leave the
        // trusted state as contradicted or disputed. Other ordinary backwards
        // transitions from Validated remain forbidden.
        Validated => matches!(to, Disputed | Contradicted | Superseded | Retracted),
        Superseded | Retracted | Rejected => false,
    }
}
