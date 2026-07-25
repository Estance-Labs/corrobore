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
//! Internal adjacency indexes for deterministic graph traversal.
//!
//! Maintains in-memory outgoing and incoming adjacency maps behind public
//! [`Graph`](crate::Graph) methods. The internal representation is private so
//! that a future hot, warm, or paged adjacency backend can replace it without
//! changing caller-facing semantics.

use std::collections::HashMap;

use crate::{GraphError, NodeId, RelationshipId};

/// Bidirectional adjacency index keyed by stable node identifiers.
///
/// Stores outgoing edges (keyed by source [`NodeId`]) and incoming edges
/// (keyed by target [`NodeId`]). Each entry preserves relationship IDs in
/// deterministic creation order, including multiple edges between the same
/// node pair. Public APIs hydrate these IDs through [`Graph`](crate::Graph),
/// so callers never depend on this internal storage shape.
#[derive(Clone, Debug, Default)]
pub(crate) struct AdjacencyIndexes {
    outgoing_by_source: HashMap<NodeId, Vec<RelationshipId>>,
    incoming_by_target: HashMap<NodeId, Vec<RelationshipId>>,
}

impl AdjacencyIndexes {
    /// Records a relationship in both the outgoing and incoming adjacency indexes.
    ///
    /// Appends `relationship_id` to the outgoing index for `source` and the
    /// incoming index for `target`, preserving deterministic creation order and
    /// supporting multiple edges between the same node pair.
    ///
    /// # Errors
    ///
    /// Returns no errors in the current in-memory implementation. The
    /// [`Result`] return type is retained so that future paged or persistent
    /// backends can surface adjacency-update failures without changing the
    /// [`Graph`](crate::Graph) write path.
    pub(crate) fn record_created_relationship(
        &mut self,
        relationship_id: &RelationshipId,
        source: &NodeId,
        target: &NodeId,
    ) -> Result<(), GraphError> {
        self.outgoing_by_source
            .entry(source.clone())
            .or_default()
            .push(relationship_id.clone());
        self.incoming_by_target
            .entry(target.clone())
            .or_default()
            .push(relationship_id.clone());

        Ok(())
    }

    /// Moves an existing relationship between adjacency buckets when a new
    /// canonical version changes one or both endpoints.
    pub(crate) fn record_replaced_relationship(
        &mut self,
        relationship_id: &RelationshipId,
        previous_source: &NodeId,
        previous_target: &NodeId,
        source: &NodeId,
        target: &NodeId,
    ) -> Result<(), GraphError> {
        if previous_source == source && previous_target == target {
            return Ok(());
        }
        if let Some(outgoing) = self.outgoing_by_source.get_mut(previous_source) {
            outgoing.retain(|candidate| candidate != relationship_id);
        }
        if let Some(incoming) = self.incoming_by_target.get_mut(previous_target) {
            incoming.retain(|candidate| candidate != relationship_id);
        }
        self.outgoing_by_source
            .entry(source.clone())
            .or_default()
            .push(relationship_id.clone());
        self.incoming_by_target
            .entry(target.clone())
            .or_default()
            .push(relationship_id.clone());
        Ok(())
    }

    /// Returns relationship IDs that leave `node_id` (outgoing edges).
    ///
    /// IDs are returned in deterministic creation order. An unknown or
    /// edge-free node yields an empty list.
    ///
    /// # Errors
    ///
    /// Returns no errors in the current in-memory implementation. The
    /// [`Result`] return type is retained for compatibility with future paged
    /// adjacency backends.
    pub(crate) fn outgoing_ids(&self, node_id: &NodeId) -> Result<Vec<RelationshipId>, GraphError> {
        Ok(self
            .outgoing_by_source
            .get(node_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Returns relationship IDs that enter `node_id` (incoming edges).
    ///
    /// IDs are returned in deterministic creation order. An unknown or
    /// edge-free node yields an empty list.
    ///
    /// # Errors
    ///
    /// Returns no errors in the current in-memory implementation. The
    /// [`Result`] return type is retained for compatibility with future paged
    /// adjacency backends.
    pub(crate) fn incoming_ids(&self, node_id: &NodeId) -> Result<Vec<RelationshipId>, GraphError> {
        Ok(self
            .incoming_by_target
            .get(node_id)
            .cloned()
            .unwrap_or_default())
    }

    /// Returns relationship IDs directed from `source` to `target`.
    ///
    /// Returns every edge from `source` to `target` in outgoing creation
    /// order, supporting multiple edges between the same pair. Unknown or
    /// unconnected pairs yield an empty list.
    ///
    /// # Errors
    ///
    /// Returns no errors in the current in-memory implementation. The
    /// [`Result`] return type is retained for compatibility with future paged
    /// adjacency backends.
    pub(crate) fn between_ids(
        &self,
        source: &NodeId,
        target: &NodeId,
    ) -> Result<Vec<RelationshipId>, GraphError> {
        let Some(outgoing) = self.outgoing_by_source.get(source) else {
            return Ok(Vec::new());
        };
        let Some(incoming) = self.incoming_by_target.get(target) else {
            return Ok(Vec::new());
        };

        Ok(outgoing
            .iter()
            .filter(|relationship_id| incoming.contains(relationship_id))
            .cloned()
            .collect())
    }
}
