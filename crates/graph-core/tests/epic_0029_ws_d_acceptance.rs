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
//! WS-D acceptance gate. Reuse the canonical fixtures so the workstream gate
//! cannot silently diverge from the individual feature contracts.
#[path = "cluster_aggregation.rs"]
mod aggregation;
#[path = "uncertainty_explanations.rs"]
mod explanation;
#[path = "actionability_resolution.rs"]
mod fabricated_evidence;
#[path = "hypothesis_set.rs"]
mod hypotheses;
#[path = "confidence_dimensions.rs"]
mod migration;
#[path = "actionability.rs"]
mod permission;
#[path = "epic_0029_ws_a_acceptance.rs"]
mod ws_a_compatibility;
#[path = "epic_0029_ws_b_acceptance.rs"]
mod ws_b_precedence;
