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

//! Engine persistence for requests pinned to one graph commit.
//!
//! The canonical engine reads and writes `main` through the paged store. A
//! read at an older commit, or a read or write at a branch head, runs on a
//! one-shot engine whose graph is the commit projection. Reads pinned to a
//! commit can never write; branch writes publish a branch commit.

use std::sync::{Arc, Mutex};

use corrobore_engine::EnginePersistence;
use graph_core::Graph;
use graph_storage::{
    CanonicalEngineStore, GraphCommitId, GraphStorageError, TransitionCommitContext,
};

/// Read-only engine persistence holding a commit projection.
#[derive(Debug)]
pub(crate) struct PinnedGraphPersistence {
    graph: Graph,
}

impl PinnedGraphPersistence {
    /// Wrap the projection a pinned read executes on.
    pub(crate) fn read_only(graph: Graph) -> Self {
        Self { graph }
    }
}

impl EnginePersistence for PinnedGraphPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self.graph.clone())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Err("reads pinned to a graph commit are read-only".to_owned())
    }
}

/// Outcome of a branch write, shared with the request handler so it can
/// report the new head or map a storage refusal to its API code.
#[derive(Debug, Default)]
pub(crate) struct BranchWriteOutcome {
    pub(crate) commit: Option<GraphCommitId>,
    pub(crate) error: Option<GraphStorageError>,
}

/// Engine persistence that publishes mutations as commits on one branch.
#[derive(Debug)]
pub(crate) struct BranchWritePersistence {
    graph: Graph,
    store: Arc<Mutex<CanonicalEngineStore>>,
    branch: String,
    expected_head: GraphCommitId,
    actor: Option<String>,
    outcome: Arc<Mutex<BranchWriteOutcome>>,
}

impl BranchWritePersistence {
    /// Wrap the branch-head projection a write executes on.
    pub(crate) fn new(
        graph: Graph,
        store: Arc<Mutex<CanonicalEngineStore>>,
        branch: String,
        expected_head: GraphCommitId,
        actor: Option<String>,
        outcome: Arc<Mutex<BranchWriteOutcome>>,
    ) -> Self {
        Self {
            graph,
            store,
            branch,
            expected_head,
            actor,
            outcome,
        }
    }
}

impl EnginePersistence for BranchWritePersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self.graph.clone())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Err("branch writes are published as graph transitions".to_owned())
    }

    fn persist_graph_transition(
        &mut self,
        previous: &Graph,
        current: &Graph,
    ) -> Result<(), String> {
        let result = self
            .store
            .lock()
            .map_err(|_| "canonical store lock poisoned".to_owned())?
            .commit_branch_transition(
                &self.branch,
                previous,
                current,
                TransitionCommitContext {
                    actor: self.actor.clone(),
                    ..TransitionCommitContext::default()
                },
                Some(&self.expected_head),
            );
        let mut outcome = self
            .outcome
            .lock()
            .map_err(|_| "branch write outcome lock poisoned".to_owned())?;
        match result {
            Ok(commit) => {
                if let Some(commit) = commit {
                    self.expected_head = commit.commit_id.clone();
                    outcome.commit = Some(commit.commit_id.clone());
                }
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                outcome.error = Some(error);
                Err(message)
            }
        }
    }
}
