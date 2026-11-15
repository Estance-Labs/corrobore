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

//! Engine persistence for reads pinned to one graph commit.
//!
//! The canonical engine reads `main` through the paged store. A read at an
//! older commit, or at a branch head, runs on a one-shot engine whose graph is
//! the commit projection; it can never write through this adapter.

use corrobore_engine::EnginePersistence;
use graph_core::Graph;

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
