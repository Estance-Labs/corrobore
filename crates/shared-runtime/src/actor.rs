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
/// Actor kind.
pub enum ActorKind {
    /// User.
    User,
    /// Agent.
    Agent,
    /// Orchestrator agent.
    OrchestratorAgent,
    /// Worker agent.
    WorkerAgent,
    /// Tool.
    Tool,
    /// System.
    System,
    /// Test fixture.
    TestFixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Actor ref.
pub struct ActorRef {
    /// Actor id.
    pub actor_id: ActorId,
    /// Kind.
    pub kind: ActorKind,
}

impl ActorRef {
    /// Creates a new instance.
    pub fn new(actor_id: ActorId, kind: ActorKind) -> Self {
        Self { actor_id, kind }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime timestamp.
pub struct RuntimeTimestamp {
    millis_since_epoch: u64,
}

impl RuntimeTimestamp {
    /// Creates an instance from millis.
    pub fn from_millis(millis_since_epoch: u64) -> Self {
        Self { millis_since_epoch }
    }

    /// Returns the value as millis.
    pub fn as_millis(&self) -> u64 {
        self.millis_since_epoch
    }
}
