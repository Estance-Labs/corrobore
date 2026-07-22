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

/// Detect a node degree from a loaded adjacency page before supernode evaluation.
///
///
/// provide the phase 1 seam for so the implementation can use available
/// adjacency counts before deciding whether a frontier node is a supernode.
///
///
/// phase 3 should derive a deterministic observed degree from pager adjacency
/// metadata without loading neighboring node or relationship payloads.
///
/// # Errors
///
/// phase 3 may return a typed graph error if adjacency metadata is inconsistent
/// or insufficient for the selected expansion direction.
pub fn observed_degree_from_adjacency(adjacency: &PagedAdjacency) -> Result<u64, GraphError> {
    Ok(adjacency.entries.len() as u64)
}

/// Validate whether a request has the guards required to expand a supernode.
///
///
/// centralize the call site that will compare observed degree against a
/// `SupernodePolicy` and translate missing relationship, label, time-window, or
/// explicit-limit guards into a typed block.
///
///
/// phase 3 should return `Ok(())` for non-supernodes and for high-degree nodes
/// that include every guard required by the policy. Missing guards should return
/// `GraphError::SupernodeExpansionBlocked` with reason metadata and a fix hint.
///
/// # Errors
///
/// return a typed graph error instead of panicking or silently skipping expansion.
pub fn check_supernode_expansion_guards(
    policy: &SupernodePolicy,
    _source_node_id: &NodeId,
    observed_degree: u64,
    request: &ExpansionRequest,
) -> Result<(), GraphError> {
    policy
        .validate_expansion_guards(
            observed_degree,
            request.has_relationship_filter(),
            request.has_label_filter(),
            request.has_time_window(),
            request.has_explicit_limit(),
        )
        .map_err(GraphError::SupernodeExpansionBlocked)
}

/// Build structured explanation metadata for a blocked supernode expansion.
///
///
/// keep the mapping from `SupernodeExpansionBlocked` to
/// `SupernodeBlockExplanation` explicit and testable without mixing it into the
/// traversal loop.
///
///
/// phase 3 should preserve node ID, observed degree, threshold, missing guards,
/// and the actionable fix hint so agents can explain why expansion stopped.
///
/// # Errors
///
/// return a typed graph error if the blocked payload cannot be represented as an
/// explanation record.
pub fn build_supernode_block_explanation(
    source_node_id: NodeId,
    error: &SupernodeExpansionBlocked,
) -> Result<SupernodeBlockExplanation, GraphError> {
    Ok(SupernodeBlockExplanation {
        node_id: source_node_id,
        observed_degree: error.observed_degree,
        degree_threshold: error.degree_threshold,
        reason: SupernodeBlockReason::RequiredGuardsMissing,
        missing_guards: supernode_missing_guards(error),
        fix_hint: ExpansionFixHint {
            scope: FixHintScope::SupernodeGuard,
            message: error.fix_hint.clone(),
        },
    })
}

/// Record skipped-expansion and supernode-block metadata for a blocked frontier node.
///
///
/// provide the phase 1 hook that will add both `SkippedExpansionReason::SupernodePolicy`
/// and `WorkingSetExplanation::record_supernode_block` output when high-degree
/// expansion is stopped.
///
///
/// phase 3 should append deterministic explanation records and avoid mutating the
/// working set itself when a supernode is blocked.
///
/// # Errors
///
/// return a typed graph error if explanation metadata cannot be built.
pub fn record_supernode_blocked_expansion(
    explanation: &mut WorkingSetExplanation,
    source_node_id: NodeId,
    error: &SupernodeExpansionBlocked,
) -> Result<(), GraphError> {
    let block = build_supernode_block_explanation(source_node_id.clone(), error)?;

    explanation.record_skipped_expansion(SkippedExpansionExplanation {
        source_node_id,
        candidate_node_id: None,
        relationship_id: None,
        relationship_type: None,
        reason: SkippedExpansionReason::SupernodePolicy,
        budget_counter: None,
        fix_hint: Some(ExpansionFixHint {
            scope: FixHintScope::SupernodeGuard,
            message: error.fix_hint.clone(),
        }),
    });
    explanation.record_supernode_block(block);

    Ok(())
}
