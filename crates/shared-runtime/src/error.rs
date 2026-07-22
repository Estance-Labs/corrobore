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

#[derive(Debug, Error, PartialEq, Eq)]
/// Runtime error.
pub enum RuntimeError {
    #[error("workspace name must not be empty")]
    /// Invalid workspace name.
    InvalidWorkspaceName,
    #[error("workspace already exists: {0:?}")]
    /// Workspace already exists.
    WorkspaceAlreadyExists(WorkspaceId),
    #[error("workspace not found: {0:?}")]
    /// Workspace not found.
    WorkspaceNotFound(WorkspaceId),
    #[error("session already exists: {0:?}")]
    /// Session already exists.
    SessionAlreadyExists(SessionId),
    #[error("session not found: {0:?}")]
    /// Session not found.
    SessionNotFound(SessionId),
    #[error("missing actor metadata for runtime session")]
    /// Missing actor.
    MissingActor,
    #[error(
        "workspace/session mismatch for session {session_id:?}: requested {workspace_id:?}, session uses {session_workspace_id:?}"
    )]
    /// Workspace session mismatch.
    WorkspaceSessionMismatch {
        /// Workspace id.
        workspace_id: WorkspaceId,
        /// Session workspace id.
        session_workspace_id: WorkspaceId,
        /// Session id.
        session_id: SessionId,
    },
    #[error("missing required transaction metadata field: {0}")]
    /// Missing transaction metadata.
    MissingTransactionMetadata(&'static str),
    #[error(
        "transaction workspace mismatch for transaction {transaction_id:?}: transaction workspace {workspace_id:?}, session workspace {session_workspace_id:?} (session {session_id:?})"
    )]
    /// Transaction workspace mismatch.
    TransactionWorkspaceMismatch {
        /// Transaction id.
        transaction_id: TransactionId,
        /// Workspace id.
        workspace_id: WorkspaceId,
        /// Session workspace id.
        session_workspace_id: WorkspaceId,
        /// Session id.
        session_id: SessionId,
    },
    #[error(
        "transaction actor mismatch for transaction {transaction_id:?}: transaction actor {actor_id:?}, session actor {session_actor_id:?} (session {session_id:?})"
    )]
    /// Transaction actor mismatch.
    TransactionActorMismatch {
        /// Transaction id.
        transaction_id: TransactionId,
        /// Actor id.
        actor_id: ActorId,
        /// Session actor id.
        session_actor_id: ActorId,
        /// Session id.
        session_id: SessionId,
    },
    #[error("unsupported cypher request mode for gateway execution: {0:?}")]
    /// Unsupported cypher request mode.
    UnsupportedCypherRequestMode(CypherRequestMode),
    #[error("malformed cypher request field: {0}")]
    /// Malformed cypher request.
    MalformedCypherRequest(&'static str),
    #[error("request mode is disallowed by runtime policy: {mode:?} ({fix_hint})")]
    /// Disallowed request mode.
    DisallowedRequestMode {
        /// Mode.
        mode: CypherRequestMode,
        /// Fix hint.
        fix_hint: String,
    },
    #[error("unsupported cypher feature: {feature} ({fix_hint})")]
    /// Unsupported Cypher feature.
    UnsupportedCypherFeature {
        /// Feature.
        feature: String,
        /// Fix hint.
        fix_hint: String,
    },
    #[error("unsafe mutation attempt: {reason} ({fix_hint})")]
    /// Unsafe mutation attempt blocked by policy.
    UnsafeMutationAttempt {
        /// Reason.
        reason: String,
        /// Fix hint.
        fix_hint: String,
    },
    #[error("request limit exceeded for {field}: actual {actual}, limit {limit} ({fix_hint})")]
    /// Request limit exceeded.
    RequestLimitExceeded {
        /// Field.
        field: &'static str,
        /// Limit.
        limit: usize,
        /// Actual.
        actual: usize,
        /// Fix hint.
        fix_hint: String,
    },
    #[error("query budget exceeded: {details:?}")]
    /// Query budget exceeded.
    QueryBudgetExceeded {
        /// Details.
        details: RuntimeBudgetExceeded,
    },
    #[error("mutation budget exceeded: {details:?}")]
    /// Mutation budget exceeded.
    MutationBudgetExceeded {
        /// Details.
        details: RuntimeBudgetExceeded,
    },
    #[error("audit metadata creation failed for field: {0}")]
    /// Audit metadata creation failed.
    AuditMetadataCreationFailed(&'static str),
    #[error("invalid benchmark corpus field: {0}")]
    /// Invalid benchmark corpus field.
    InvalidBenchmarkCorpus(&'static str),
    #[error("duplicate benchmark fixture id: {0}")]
    /// Duplicate benchmark fixture identifier.
    DuplicateBenchmarkFixtureId(String),
    #[error(
        "benchmark run fixture mismatch: baseline {baseline_fixture_id}, graph {graph_fixture_id}"
    )]
    /// Benchmark fixture mismatch between baseline and graph runs.
    BenchmarkFixtureMismatch {
        /// Baseline fixture id.
        baseline_fixture_id: String,
        /// Graph fixture id.
        graph_fixture_id: String,
    },
    #[error("invalid benchmark run field: {0}")]
    /// Invalid benchmark run field.
    InvalidBenchmarkRun(&'static str),
    #[error("invalid multi-agent benchmark scenario field: {0}")]
    /// Invalid multi-agent benchmark scenario field.
    InvalidMultiAgentBenchmarkScenario(&'static str),
}
