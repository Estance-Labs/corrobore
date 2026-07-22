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
use std::{collections::HashMap, path::PathBuf};

use corrobore_http_server::{
    explorer_timeline::{
        ExplorerBoundarySelection, ExplorerTimelineError, ExplorerTimelineStore,
        ExplorerTimeshotInput,
    },
    session_runtime::{SessionRuntime, SessionServiceStatus, StartSessionInput},
};
use graph_core::{ActorId, SnapshotCreateRequest, SnapshotId, SnapshotManager, TransactionId};
use shared_runtime::ActorKind;

fn unique_store_dir(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "corrobore-explorer-timeline-{suffix}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn start_session(runtime: &mut SessionRuntime, workspace_id: &str, actor_id: &str) -> String {
    runtime
        .start_session(StartSessionInput {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            actor_kind: ActorKind::Agent,
            metadata: HashMap::new(),
        })
        .expect("session should start")
        .session_id
}

fn snapshot(
    id: &str,
    transaction_id: &str,
    created_at: &str,
    actor_id: &str,
) -> graph_core::Snapshot {
    let mut manager = SnapshotManager::new();
    manager
        .create_snapshot(
            SnapshotCreateRequest::new(
                SnapshotId::new(id).expect("snapshot id should be valid"),
                TransactionId::new(transaction_id).expect("transaction id should be valid"),
                ActorId::new(actor_id).expect("actor id should be valid"),
                "explorer checkpoint",
                id,
            )
            .expect("snapshot request should be valid"),
            created_at,
        )
        .expect("snapshot should be created")
}

#[test]
fn current_session_listing_is_stable_and_excludes_stopped_sessions_by_default() {
    let store_dir = unique_store_dir("sessions");
    let mut runtime = SessionRuntime::new(store_dir.display().to_string(), 0);
    let first = start_session(&mut runtime, "workspace--z", "actor--z");
    let second = start_session(&mut runtime, "workspace--a", "actor--a");
    let stopped = start_session(&mut runtime, "workspace--stopped", "actor--stopped");
    runtime
        .stop_session(&stopped)
        .expect("third session should stop");

    let current = runtime.list_sessions(false);
    let current_ids = current
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<Vec<_>>();
    let mut expected = vec![first.as_str(), second.as_str()];
    expected.sort_unstable();
    assert_eq!(current_ids, expected);
    assert!(current.iter().all(|session| {
        session.status != SessionServiceStatus::Stopped
            && session.started_at_ms <= session.updated_at_ms
            && !session.workspace_id.is_empty()
            && !session.actor_id.is_empty()
    }));

    let with_stopped = runtime.list_sessions(true);
    assert_eq!(with_stopped.len(), 3);
    assert!(
        with_stopped
            .windows(2)
            .all(|pair| { pair[0].session_id.as_str() < pair[1].session_id.as_str() })
    );
}

#[test]
fn snapshot_and_timeshot_lineage_is_acyclic_stable_and_persistent() {
    let store_dir = unique_store_dir("persistent-lineage");
    let mut runtime = SessionRuntime::new(store_dir.display().to_string(), 0);
    let session_id = start_session(&mut runtime, "workspace--timeline", "actor--timeline");
    let session = runtime
        .session_health(&session_id)
        .expect("session should exist");
    let baseline = snapshot(
        "snapshot--baseline",
        "transaction--40",
        "2026-07-17T00:00:00Z",
        "actor--timeline",
    );
    let followup = snapshot(
        "snapshot--followup",
        "transaction--42",
        "2026-07-17T00:02:00Z",
        "actor--timeline",
    );

    let mut timeline = ExplorerTimelineStore::new(&store_dir);
    timeline
        .record_snapshot(&session, &baseline, None)
        .expect("baseline should be recorded");
    timeline
        .record_timeshot(
            &session,
            ExplorerTimeshotInput::new(
                "timeshot--analysis-41",
                "snapshot--baseline",
                Some("transaction--41"),
                "2026-07-17T00:01:00Z",
                "analyst review",
            )
            .expect("timeshot should be valid"),
        )
        .expect("timeshot should be recorded");
    timeline
        .record_snapshot(&session, &followup, Some("timeshot--analysis-41"))
        .expect("followup should be recorded");

    let tree = timeline
        .timeline_for_session(&session)
        .expect("timeline should resolve");
    assert_eq!(tree.roots.len(), 1);
    assert_eq!(tree.roots[0].boundary.boundary_id, "snapshot--baseline");
    assert_eq!(tree.roots[0].children.len(), 1);
    assert_eq!(
        tree.roots[0].children[0].boundary.boundary_id,
        "timeshot--analysis-41"
    );
    assert_eq!(tree.roots[0].children[0].children.len(), 1);
    assert_eq!(
        tree.roots[0].children[0].children[0].boundary.boundary_id,
        "snapshot--followup"
    );

    let reloaded = ExplorerTimelineStore::new(&store_dir);
    assert_eq!(
        tree,
        reloaded
            .timeline_for_session(&session)
            .expect("reloaded timeline should resolve")
    );
}

#[test]
fn boundary_resolution_preserves_exact_identity_and_rejects_cross_session_access() {
    let store_dir = unique_store_dir("boundary-resolution");
    let mut runtime = SessionRuntime::new(store_dir.display().to_string(), 0);
    let owner_id = start_session(&mut runtime, "workspace--owner", "actor--owner");
    let other_id = start_session(&mut runtime, "workspace--other", "actor--other");
    let owner = runtime
        .session_health(&owner_id)
        .expect("owner session should exist");
    let other = runtime
        .session_health(&other_id)
        .expect("other session should exist");
    let snapshot = snapshot(
        "snapshot--owned",
        "transaction--owned",
        "2026-07-17T01:00:00Z",
        "actor--owner",
    );
    let mut timeline = ExplorerTimelineStore::new(&store_dir);
    timeline
        .record_snapshot(&owner, &snapshot, None)
        .expect("snapshot should be recorded");

    let resolved = timeline
        .resolve_boundary(
            &owner,
            &ExplorerBoundarySelection::snapshot("snapshot--owned"),
        )
        .expect("owner should resolve snapshot");
    assert_eq!(resolved.kind(), "snapshot");
    assert_eq!(resolved.boundary_id(), Some("snapshot--owned"));
    assert_eq!(resolved.transaction_id(), Some("transaction--owned"));
    assert_eq!(resolved.at(), Some("2026-07-17T01:00:00Z"));

    let cross_session = timeline
        .resolve_boundary(
            &other,
            &ExplorerBoundarySelection::snapshot("snapshot--owned"),
        )
        .expect_err("another session must not resolve the boundary");
    assert!(matches!(
        cross_session,
        ExplorerTimelineError::BoundarySessionMismatch {
            ref boundary_id,
            ref requested_session_id,
        } if boundary_id == "snapshot--owned" && requested_session_id == &other_id
    ));
}

#[test]
fn invalid_parent_and_temporal_order_are_rejected_before_persistence() {
    let store_dir = unique_store_dir("invalid-lineage");
    let mut runtime = SessionRuntime::new(store_dir.display().to_string(), 0);
    let session_id = start_session(&mut runtime, "workspace--lineage", "actor--lineage");
    let session = runtime
        .session_health(&session_id)
        .expect("session should exist");
    let mut timeline = ExplorerTimelineStore::new(&store_dir);

    let missing_parent = timeline
        .record_timeshot(
            &session,
            ExplorerTimeshotInput::new(
                "timeshot--orphan",
                "snapshot--missing",
                None::<String>,
                "2026-07-17T02:00:00Z",
                "orphan",
            )
            .expect("input shape should be valid"),
        )
        .expect_err("missing parent should be rejected");
    assert!(matches!(
        missing_parent,
        ExplorerTimelineError::ParentBoundaryNotFound { .. }
    ));

    let baseline = snapshot(
        "snapshot--late",
        "transaction--late",
        "2026-07-17T03:00:00Z",
        "actor--lineage",
    );
    timeline
        .record_snapshot(&session, &baseline, None)
        .expect("baseline should be recorded");
    let backwards = timeline
        .record_timeshot(
            &session,
            ExplorerTimeshotInput::new(
                "timeshot--early",
                "snapshot--late",
                None::<String>,
                "2026-07-17T02:59:00Z",
                "backwards",
            )
            .expect("input shape should be valid"),
        )
        .expect_err("child before parent should be rejected");
    assert!(matches!(
        backwards,
        ExplorerTimelineError::InvalidTemporalOrder { .. }
    ));
}
