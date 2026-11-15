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

//! Graph commit history: addressable commits, branches and commit views.
//!
//! The canonical WAL makes a transaction durable but gives it no identity a
//! reader can address later. This module adds that identity on top of the
//! append-only payload logs, which already retain every record version:
//!
//! - every applied canonical transaction becomes a [`GraphCommit`] whose
//!   `changes` are the record versions it published, each with the storage
//!   reference that still holds its bytes;
//! - a commit's first parent is the previous head of its branch, so the view
//!   of any commit is the fold of first-parent deltas from the genesis commit;
//! - branches are named heads recorded in the same log, and branch writes keep
//!   their payloads in a dedicated branch payload log that the canonical
//!   catalog never indexes, so catalog rebuild cannot leak branch data into
//!   `main`.
//!
//! The history log (`history/graph_history.log`) is JSON Lines. Every line
//! carries a SHA-256 of its entry and is appended with `sync_data`. A torn
//! final line is truncated on open; any other invalid line is corruption.
//! Appending a line is the visibility point of a commit or branch change,
//! which mirrors the manifest publication that makes an Omnigraph write
//! visible.

use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    GraphStorageError, GraphStorageResult, JsonLinesRecordCodec, StorageRef, StorageRoot,
    StorageSegment, validate_encoded_record_checksum,
};

/// Name of the branch backed by the canonical catalog.
pub const MAIN_BRANCH: &str = "main";

const HISTORY_DIR: &str = "history";
const HISTORY_LOG: &str = "graph_history.log";
const BRANCH_PAYLOAD_LOG: &str = "branch_payloads.log";
const COMMIT_ID_PREFIX: &str = "gc";
const COMMIT_ID_HEX_LEN: usize = 40;
const MAX_BRANCH_NAME_LEN: usize = 128;
// Views are memoized along first-parent chains so that reading an old commit
// replays at most this many deltas from the nearest memoized ancestor.
const VIEW_MEMO_INTERVAL: u64 = 256;
const VIEW_MEMO_CAPACITY: usize = 8;

/// Stable identifier of one graph commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphCommitId(String);

impl GraphCommitId {
    /// Validate and wrap a commit id (`gc` followed by 40 lowercase hex digits).
    pub fn new(value: impl Into<String>) -> GraphStorageResult<Self> {
        let value = value.into();
        let valid = value.strip_prefix(COMMIT_ID_PREFIX).is_some_and(|hex| {
            hex.len() == COMMIT_ID_HEX_LEN
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
        if valid {
            Ok(Self(value))
        } else {
            Err(GraphStorageError::UnknownGraphCommit { commit_id: value })
        }
    }

    /// Borrow the commit id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GraphCommitId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Kind of record a commit change publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRecordKind {
    /// Graph node.
    Node,
    /// Graph relationship.
    Relationship,
}

/// Where the bytes of a committed record version live.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "store", rename_all = "snake_case")]
pub enum CommitPayloadRef {
    /// Canonical node or relationship payload log, indexed by the catalog.
    Canonical {
        /// Retained storage reference of the record version.
        storage_ref: StorageRef,
    },
    /// Branch payload log, never indexed by the canonical catalog.
    Branch {
        /// Byte offset in `history/branch_payloads.log`.
        offset: u64,
        /// Payload length in bytes.
        length: u64,
        /// Lowercase hex SHA-256 of the payload bytes.
        sha256: String,
    },
}

/// One record version published by a commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRecordChange {
    /// Node or relationship.
    pub kind: HistoryRecordKind,
    /// Stable logical record id (`node--N`, `relationship--N`).
    pub record_id: String,
    /// Version id published by this commit.
    pub version_id: String,
    /// Whether the published version is a tombstone.
    pub deleted: bool,
    /// Node labels, or the relationship type as a single entry.
    pub labels: Vec<String>,
    /// Relationship source node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Relationship target node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Where the version bytes live.
    pub payload: CommitPayloadRef,
}

/// One addressable graph commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCommit {
    /// Content-derived commit id.
    pub commit_id: GraphCommitId,
    /// First parent is the previous head of `branch`; a merge adds the source head.
    pub parents: Vec<GraphCommitId>,
    /// Branch whose head this commit became.
    pub branch: String,
    /// `1 + max(parent generation)`; genesis is generation zero.
    pub generation: u64,
    /// Canonical durable transaction id when the commit went through the WAL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    /// Canonical mutation sequence when the commit went through the WAL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_sequence_number: Option<u64>,
    /// Actor that requested the write, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Free-text commit message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
    /// Record versions published relative to the first parent.
    pub changes: Vec<CommitRecordChange>,
}

impl GraphCommit {
    /// Number of changed nodes.
    pub fn node_change_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.kind == HistoryRecordKind::Node)
            .count()
    }

    /// Number of changed relationships.
    pub fn relationship_change_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.kind == HistoryRecordKind::Relationship)
            .count()
    }

    /// Number of published tombstones.
    pub fn deletion_count(&self) -> usize {
        self.changes.iter().filter(|change| change.deleted).count()
    }
}

/// A named branch head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphBranchInfo {
    /// Branch name.
    pub name: String,
    /// Current head commit.
    pub head: GraphCommitId,
    /// Commit the branch was created from.
    pub created_from: GraphCommitId,
    /// RFC 3339 UTC creation time.
    pub created_at: String,
    /// Actor that created the branch, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

/// Commit to append, before its id and generation are derived.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphCommitDraft {
    /// Target branch; its current head becomes the first parent.
    pub branch: String,
    /// Additional parents (the source head of a merge).
    pub extra_parents: Vec<GraphCommitId>,
    /// Canonical transaction id, for WAL-backed commits.
    pub transaction_id: Option<String>,
    /// Canonical mutation sequence, for WAL-backed commits.
    pub mutation_sequence_number: Option<u64>,
    /// Requesting actor.
    pub actor: Option<String>,
    /// Commit message.
    pub message: Option<String>,
    /// RFC 3339 UTC creation time; `None` uses the current time.
    pub created_at: Option<String>,
    /// Record versions published relative to the first parent.
    pub changes: Vec<CommitRecordChange>,
}

/// The state of one record in a commit view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitViewEntry {
    /// Version visible at the commit.
    pub version_id: String,
    /// Whether the visible version is a tombstone.
    pub deleted: bool,
    /// Node labels, or the relationship type.
    pub labels: Vec<String>,
    /// Relationship source.
    pub source: Option<String>,
    /// Relationship target.
    pub target: Option<String>,
    /// Where the visible version bytes live.
    pub payload: CommitPayloadRef,
    /// Commit that published the visible version.
    pub published_by: GraphCommitId,
}

/// Immutable set of record versions visible at one commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitView {
    /// Visible nodes keyed by logical id, in id order for keyset paging.
    pub nodes: BTreeMap<String, CommitViewEntry>,
    /// Visible relationships keyed by logical id.
    pub relationships: BTreeMap<String, CommitViewEntry>,
}

impl CommitView {
    fn apply(&mut self, commit: &GraphCommit) {
        for change in &commit.changes {
            let entry = CommitViewEntry {
                version_id: change.version_id.clone(),
                deleted: change.deleted,
                labels: change.labels.clone(),
                source: change.source.clone(),
                target: change.target.clone(),
                payload: change.payload.clone(),
                published_by: commit.commit_id.clone(),
            };
            match change.kind {
                HistoryRecordKind::Node => {
                    self.nodes.insert(change.record_id.clone(), entry);
                }
                HistoryRecordKind::Relationship => {
                    self.relationships.insert(change.record_id.clone(), entry);
                }
            }
        }
    }

    /// Entries of one kind.
    pub fn entries(&self, kind: HistoryRecordKind) -> &BTreeMap<String, CommitViewEntry> {
        match kind {
            HistoryRecordKind::Node => &self.nodes,
            HistoryRecordKind::Relationship => &self.relationships,
        }
    }

    /// Number of live (non-tombstoned) records of one kind.
    pub fn live_count(&self, kind: HistoryRecordKind) -> usize {
        self.entries(kind)
            .values()
            .filter(|entry| !entry.deleted)
            .count()
    }
}

/// Highest id sequences ever published on any branch.
///
/// Graph ids are sequence-allocated (`node--N`). Branches allocate from the
/// same space as `main`, so every writer must start above this floor or two
/// branches could mint the same id for different records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryIdFloor {
    /// Highest node sequence.
    pub node: u64,
    /// Highest node-version sequence.
    pub node_version: u64,
    /// Highest relationship sequence.
    pub relationship: u64,
    /// Highest relationship-version sequence.
    pub relationship_version: u64,
}

impl HistoryIdFloor {
    fn observe(&mut self, change: &CommitRecordChange) {
        let record = sequence_suffix(&change.record_id);
        let version = sequence_suffix(&change.version_id);
        match change.kind {
            HistoryRecordKind::Node => {
                self.node = self.node.max(record);
                self.node_version = self.node_version.max(version);
            }
            HistoryRecordKind::Relationship => {
                self.relationship = self.relationship.max(record);
                self.relationship_version = self.relationship_version.max(version);
            }
        }
    }
}

/// One page of a first-parent commit listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommitPage {
    /// Commits, newest first.
    pub commits: Vec<Arc<GraphCommit>>,
    /// Cursor for the next (older) page.
    pub next_cursor: Option<GraphCommitId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HistoryEntry {
    Commit(GraphCommit),
    BranchCreated {
        name: String,
        from: GraphCommitId,
        created_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<String>,
    },
    BranchDeleted {
        name: String,
        deleted_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deleted_by: Option<String>,
    },
}

#[derive(Serialize, Deserialize)]
struct HistoryLine {
    entry: HistoryEntry,
    sha256: String,
}

/// Durable graph commit history of one storage root.
#[derive(Clone, Debug)]
pub struct GraphHistory {
    directory: PathBuf,
    commits: HashMap<GraphCommitId, Arc<GraphCommit>>,
    order: Vec<GraphCommitId>,
    branches: BTreeMap<String, GraphBranchInfo>,
    genesis: Option<GraphCommitId>,
    last_main_sequence: Option<u64>,
    id_floor: HistoryIdFloor,
    repaired_torn_tail: bool,
    memo: HashMap<GraphCommitId, Arc<CommitView>>,
    memo_order: VecDeque<GraphCommitId>,
}

impl GraphHistory {
    /// Open the history of a storage root, repairing a torn final line.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphHistoryCorrupted`] when a complete line
    /// fails its checksum, does not decode, or references unknown commits.
    pub fn open(root: &StorageRoot) -> GraphStorageResult<Self> {
        let directory = root.path().join(HISTORY_DIR);
        let mut history = Self {
            directory,
            commits: HashMap::new(),
            order: Vec::new(),
            branches: BTreeMap::new(),
            genesis: None,
            last_main_sequence: None,
            id_floor: HistoryIdFloor::default(),
            repaired_torn_tail: false,
            memo: HashMap::new(),
            memo_order: VecDeque::new(),
        };
        let path = history.log_path();
        if !path.is_file() {
            return Ok(history);
        }
        let bytes =
            fs::read(&path).map_err(|error| io_error("open_graph_history", &path, error))?;
        let mut valid_len = 0_usize;
        let mut line_number = 0_usize;
        let mut cursor = 0_usize;
        while cursor < bytes.len() {
            line_number += 1;
            let Some(relative_end) = bytes[cursor..].iter().position(|byte| *byte == b'\n') else {
                // A final line without its newline was torn mid-append.
                break;
            };
            let end = cursor + relative_end;
            let line = &bytes[cursor..end];
            let is_last = end + 1 >= bytes.len();
            match decode_line(line) {
                Ok(entry) => {
                    history.apply_entry(entry).map_err(|reason| {
                        GraphStorageError::GraphHistoryCorrupted {
                            line: line_number,
                            reason,
                        }
                    })?;
                    valid_len = end + 1;
                }
                Err(_) if is_last => break,
                Err(reason) => {
                    return Err(GraphStorageError::GraphHistoryCorrupted {
                        line: line_number,
                        reason,
                    });
                }
            }
            cursor = end + 1;
        }
        if valid_len < bytes.len() {
            let file = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|error| io_error("repair_graph_history_tail", &path, error))?;
            file.set_len(valid_len as u64)
                .and_then(|()| file.sync_data())
                .map_err(|error| io_error("repair_graph_history_tail", &path, error))?;
            history.repaired_torn_tail = true;
        }
        Ok(history)
    }

    /// Whether a genesis commit exists.
    pub fn is_initialized(&self) -> bool {
        self.genesis.is_some()
    }

    /// Whether opening truncated a torn final line.
    pub fn repaired_torn_tail(&self) -> bool {
        self.repaired_torn_tail
    }

    /// Genesis commit id.
    pub fn genesis(&self) -> Option<&GraphCommitId> {
        self.genesis.as_ref()
    }

    /// Highest canonical mutation sequence recorded by a `main` commit.
    pub fn last_main_sequence(&self) -> Option<u64> {
        self.last_main_sequence
    }

    /// Highest id sequences published on any branch.
    pub fn id_floor(&self) -> HistoryIdFloor {
        self.id_floor
    }

    /// Number of retained commits.
    pub fn commit_count(&self) -> usize {
        self.order.len()
    }

    /// Create the genesis commit on `main`.
    ///
    /// `changes` describe the records already present when history starts, and
    /// `mutation_sequence_number` the last canonical transaction they include.
    pub fn initialize_genesis(
        &mut self,
        changes: Vec<CommitRecordChange>,
        mutation_sequence_number: Option<u64>,
        created_at: Option<String>,
    ) -> GraphStorageResult<Arc<GraphCommit>> {
        if self.genesis.is_some() {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: MAIN_BRANCH.to_owned(),
                reason: "history already has a genesis commit".to_owned(),
            });
        }
        let created_at = created_at.unwrap_or_else(now_rfc3339);
        let mut commit = GraphCommit {
            commit_id: GraphCommitId(String::new()),
            parents: Vec::new(),
            branch: MAIN_BRANCH.to_owned(),
            generation: 0,
            transaction_id: None,
            mutation_sequence_number,
            actor: None,
            message: Some("genesis".to_owned()),
            created_at,
            changes,
        };
        commit.commit_id = derive_commit_id(&commit)?;
        self.append(HistoryEntry::Commit(commit.clone()))?;
        self.commit(&commit.commit_id)
    }

    /// Append a commit to `draft.branch`, whose head becomes the first parent.
    pub fn record_commit(
        &mut self,
        draft: GraphCommitDraft,
    ) -> GraphStorageResult<Arc<GraphCommit>> {
        let head = self.head(&draft.branch)?.clone();
        let mut parents = vec![head];
        for parent in draft.extra_parents {
            if !self.commits.contains_key(&parent) {
                return Err(GraphStorageError::UnknownGraphCommit {
                    commit_id: parent.0,
                });
            }
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        }
        let generation = parents
            .iter()
            .filter_map(|parent| self.commits.get(parent))
            .map(|parent| parent.generation)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut commit = GraphCommit {
            commit_id: GraphCommitId(String::new()),
            parents,
            branch: draft.branch,
            generation,
            transaction_id: draft.transaction_id,
            mutation_sequence_number: draft.mutation_sequence_number,
            actor: draft.actor,
            message: draft.message,
            created_at: draft.created_at.unwrap_or_else(now_rfc3339),
            changes: draft.changes,
        };
        commit.commit_id = derive_commit_id(&commit)?;
        self.append(HistoryEntry::Commit(commit.clone()))?;
        self.commit(&commit.commit_id)
    }

    /// Create `name` pointing at `from`.
    pub fn create_branch(
        &mut self,
        name: &str,
        from: &GraphCommitId,
        created_by: Option<String>,
    ) -> GraphStorageResult<GraphBranchInfo> {
        validate_branch_name(name)?;
        if self.branches.contains_key(name) {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: name.to_owned(),
                reason: "a branch with this name already exists".to_owned(),
            });
        }
        if !self.commits.contains_key(from) {
            return Err(GraphStorageError::UnknownGraphCommit {
                commit_id: from.0.clone(),
            });
        }
        self.append(HistoryEntry::BranchCreated {
            name: name.to_owned(),
            from: from.clone(),
            created_at: now_rfc3339(),
            created_by,
        })?;
        self.branch(name).cloned()
    }

    /// Delete a branch other than `main`. Its commits stay retained until cleanup.
    pub fn delete_branch(
        &mut self,
        name: &str,
        deleted_by: Option<String>,
    ) -> GraphStorageResult<()> {
        if name == MAIN_BRANCH {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: name.to_owned(),
                reason: "main cannot be deleted".to_owned(),
            });
        }
        if !self.branches.contains_key(name) {
            return Err(GraphStorageError::UnknownGraphBranch {
                branch: name.to_owned(),
            });
        }
        self.append(HistoryEntry::BranchDeleted {
            name: name.to_owned(),
            deleted_at: now_rfc3339(),
            deleted_by,
        })
    }

    /// All branches in name order.
    pub fn branches(&self) -> impl Iterator<Item = &GraphBranchInfo> {
        self.branches.values()
    }

    /// One branch.
    pub fn branch(&self, name: &str) -> GraphStorageResult<&GraphBranchInfo> {
        self.branches
            .get(name)
            .ok_or_else(|| GraphStorageError::UnknownGraphBranch {
                branch: name.to_owned(),
            })
    }

    /// Head commit of a branch.
    pub fn head(&self, branch: &str) -> GraphStorageResult<&GraphCommitId> {
        self.branch(branch).map(|info| &info.head)
    }

    /// One retained commit.
    pub fn commit(&self, commit_id: &GraphCommitId) -> GraphStorageResult<Arc<GraphCommit>> {
        self.commits
            .get(commit_id)
            .cloned()
            .ok_or_else(|| GraphStorageError::UnknownGraphCommit {
                commit_id: commit_id.0.clone(),
            })
    }

    /// Whether a commit is retained.
    pub fn contains(&self, commit_id: &GraphCommitId) -> bool {
        self.commits.contains_key(commit_id)
    }

    /// Commits in append order (oldest first).
    pub fn commits_in_append_order(&self) -> impl Iterator<Item = Arc<GraphCommit>> + '_ {
        self.order
            .iter()
            .filter_map(|commit_id| self.commits.get(commit_id).cloned())
    }

    /// Walk the first-parent chain from `start`, newest first.
    ///
    /// `cursor` resumes after a commit returned by a previous page.
    pub fn first_parent_page(
        &self,
        start: &GraphCommitId,
        cursor: Option<&GraphCommitId>,
        limit: usize,
    ) -> GraphStorageResult<GraphCommitPage> {
        let limit = limit.max(1);
        let mut current = Some(self.commit(start)?);
        if let Some(cursor) = cursor {
            loop {
                let Some(commit) = current.take() else {
                    return Err(GraphStorageError::UnknownGraphCommit {
                        commit_id: cursor.0.clone(),
                    });
                };
                current = self.first_parent(&commit);
                if &commit.commit_id == cursor {
                    break;
                }
            }
        }
        let mut commits = Vec::new();
        while let Some(commit) = current {
            if commits.len() == limit {
                let next_cursor = commits
                    .last()
                    .map(|last: &Arc<GraphCommit>| last.commit_id.clone());
                return Ok(GraphCommitPage {
                    commits,
                    next_cursor,
                });
            }
            current = self.first_parent(&commit);
            commits.push(commit);
        }
        Ok(GraphCommitPage {
            commits,
            next_cursor: None,
        })
    }

    fn first_parent(&self, commit: &GraphCommit) -> Option<Arc<GraphCommit>> {
        commit
            .parents
            .first()
            .and_then(|parent| self.commits.get(parent).cloned())
    }

    /// Whether `ancestor` is reachable from `descendant` through any parent (inclusive).
    pub fn is_ancestor(
        &self,
        ancestor: &GraphCommitId,
        descendant: &GraphCommitId,
    ) -> GraphStorageResult<bool> {
        let target = self.commit(ancestor)?;
        let start = self.commit(descendant)?;
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(commit) = stack.pop() {
            if commit.commit_id == target.commit_id {
                return Ok(true);
            }
            if commit.generation <= target.generation || !seen.insert(commit.commit_id.clone()) {
                continue;
            }
            for parent in &commit.parents {
                if let Some(parent) = self.commits.get(parent) {
                    stack.push(Arc::clone(parent));
                }
            }
        }
        Ok(false)
    }

    /// Best common ancestor of two commits: the shared ancestor with the
    /// highest generation, found by walking `b` in descending generation.
    pub fn merge_base(
        &self,
        a: &GraphCommitId,
        b: &GraphCommitId,
    ) -> GraphStorageResult<Option<GraphCommitId>> {
        let a_ancestors = self.ancestors(a)?;
        let start = self.commit(b)?;
        let mut heap = BinaryHeap::new();
        let mut seen = HashSet::new();
        heap.push((start.generation, start.commit_id.clone()));
        while let Some((_, commit_id)) = heap.pop() {
            if !seen.insert(commit_id.clone()) {
                continue;
            }
            if a_ancestors.contains(&commit_id) {
                return Ok(Some(commit_id));
            }
            let commit = self.commit(&commit_id)?;
            for parent in &commit.parents {
                if let Some(parent) = self.commits.get(parent) {
                    heap.push((parent.generation, parent.commit_id.clone()));
                }
            }
        }
        Ok(None)
    }

    fn ancestors(&self, start: &GraphCommitId) -> GraphStorageResult<HashSet<GraphCommitId>> {
        let mut seen = HashSet::new();
        let mut stack = vec![self.commit(start)?];
        while let Some(commit) = stack.pop() {
            if !seen.insert(commit.commit_id.clone()) {
                continue;
            }
            for parent in &commit.parents {
                if let Some(parent) = self.commits.get(parent) {
                    stack.push(Arc::clone(parent));
                }
            }
        }
        Ok(seen)
    }

    /// Materialize the record versions visible at a commit.
    pub fn view(&mut self, commit_id: &GraphCommitId) -> GraphStorageResult<Arc<CommitView>> {
        if let Some(view) = self.memo.get(commit_id) {
            return Ok(Arc::clone(view));
        }
        let mut chain = Vec::new();
        let mut base = None;
        let mut current = Some(self.commit(commit_id)?);
        while let Some(commit) = current {
            if let Some(view) = self.memo.get(&commit.commit_id) {
                base = Some(Arc::clone(view));
                break;
            }
            current = self.first_parent(&commit);
            chain.push(commit);
        }
        let mut view = base.map(|view| (*view).clone()).unwrap_or_default();
        let last = chain.len();
        for (index, commit) in chain.into_iter().rev().enumerate() {
            view.apply(&commit);
            let is_target = index + 1 == last;
            if !is_target && commit.generation % VIEW_MEMO_INTERVAL == 0 {
                self.remember(commit.commit_id.clone(), Arc::new(view.clone()));
            }
        }
        let view = Arc::new(view);
        self.remember(commit_id.clone(), Arc::clone(&view));
        Ok(view)
    }

    fn remember(&mut self, commit_id: GraphCommitId, view: Arc<CommitView>) {
        if self.memo.insert(commit_id.clone(), view).is_none() {
            self.memo_order.push_back(commit_id);
        }
        while self.memo_order.len() > VIEW_MEMO_CAPACITY {
            if let Some(evicted) = self.memo_order.pop_front() {
                self.memo.remove(&evicted);
            }
        }
    }

    /// Append branch payloads with one `sync_data`, returning their references.
    pub fn append_branch_payloads(
        &mut self,
        payloads: &[Vec<u8>],
    ) -> GraphStorageResult<Vec<CommitPayloadRef>> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        let path = self.directory.join(BRANCH_PAYLOAD_LOG);
        let created = self.ensure_directory()? || !path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| io_error("append_branch_payloads", &path, error))?;
        let mut offset = file
            .seek(SeekFrom::End(0))
            .map_err(|error| io_error("append_branch_payloads", &path, error))?;
        let mut buffer = Vec::new();
        let mut references = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let length = payload.len() as u64;
            references.push(CommitPayloadRef::Branch {
                offset,
                length,
                sha256: hex_sha256(payload),
            });
            buffer.extend_from_slice(payload);
            offset = offset.saturating_add(length);
        }
        file.write_all(&buffer)
            .and_then(|()| file.sync_data())
            .map_err(|error| io_error("append_branch_payloads", &path, error))?;
        if created {
            sync_directory(&self.directory)?;
        }
        Ok(references)
    }

    /// Read and verify the bytes of a committed record version.
    pub fn read_payload(
        root: &StorageRoot,
        kind: HistoryRecordKind,
        payload: &CommitPayloadRef,
    ) -> GraphStorageResult<Vec<u8>> {
        match payload {
            CommitPayloadRef::Canonical { storage_ref } => {
                let path = match (&storage_ref.segment, kind) {
                    (StorageSegment::NodeRecords, HistoryRecordKind::Node) => {
                        root.path().join("nodes").join("node_records.log")
                    }
                    (StorageSegment::RelationshipRecords, HistoryRecordKind::Relationship) => root
                        .path()
                        .join("relationships")
                        .join("relationship_records.log"),
                    (segment, _) => {
                        return Err(GraphStorageError::OperationFailed {
                            operation: "read_graph_history_payload",
                            message: format!("{segment:?} does not hold {kind:?} payloads"),
                        });
                    }
                };
                let bytes = read_range(&path, storage_ref.offset, storage_ref.length)?;
                if let Some(checksum) = &storage_ref.checksum {
                    validate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes, checksum)?;
                }
                Ok(bytes)
            }
            CommitPayloadRef::Branch {
                offset,
                length,
                sha256,
            } => {
                let path = root.path().join(HISTORY_DIR).join(BRANCH_PAYLOAD_LOG);
                let bytes = read_range(&path, *offset, *length)?;
                let actual = hex_sha256(&bytes);
                if &actual != sha256 {
                    return Err(GraphStorageError::ChecksumMismatch {
                        expected: crate::RecordChecksum {
                            algorithm: "sha256".to_owned(),
                            value: sha256.clone(),
                        },
                        actual: crate::RecordChecksum {
                            algorithm: "sha256".to_owned(),
                            value: actual,
                        },
                    });
                }
                Ok(bytes)
            }
        }
    }

    fn log_path(&self) -> PathBuf {
        self.directory.join(HISTORY_LOG)
    }

    fn ensure_directory(&self) -> GraphStorageResult<bool> {
        if self.directory.is_dir() {
            return Ok(false);
        }
        fs::create_dir_all(&self.directory)
            .map_err(|error| io_error("create_graph_history_dir", &self.directory, error))?;
        if let Some(parent) = self.directory.parent() {
            sync_directory(parent)?;
        }
        Ok(true)
    }

    fn append(&mut self, entry: HistoryEntry) -> GraphStorageResult<()> {
        // Validate against in-memory state first so a refused entry is never durable.
        let mut probe_commits = None;
        if let HistoryEntry::Commit(commit) = &entry {
            probe_commits = Some(commit.commit_id.clone());
            if self.commits.contains_key(&commit.commit_id) {
                return Err(GraphStorageError::GraphBranchConflict {
                    branch: commit.branch.clone(),
                    reason: format!("commit {} already exists", commit.commit_id),
                });
            }
        }
        let line = encode_line(&entry)?;
        let created = self.ensure_directory()?;
        let path = self.log_path();
        let new_file = created || !path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| io_error("append_graph_history", &path, error))?;
        file.write_all(&line)
            .and_then(|()| file.sync_data())
            .map_err(|error| io_error("append_graph_history", &path, error))?;
        if new_file {
            sync_directory(&self.directory)?;
        }
        self.apply_entry(entry)
            .map_err(|reason| GraphStorageError::GraphHistoryCorrupted {
                line: self.order.len() + usize::from(probe_commits.is_some()),
                reason,
            })
    }

    fn apply_entry(&mut self, entry: HistoryEntry) -> Result<(), String> {
        match entry {
            HistoryEntry::Commit(commit) => {
                if self.commits.contains_key(&commit.commit_id) {
                    return Err(format!("duplicate commit {}", commit.commit_id));
                }
                for parent in &commit.parents {
                    if !self.commits.contains_key(parent) {
                        return Err(format!(
                            "commit {} has unknown parent {parent}",
                            commit.commit_id
                        ));
                    }
                }
                if commit.parents.is_empty() {
                    if self.genesis.is_some() {
                        return Err(format!("second genesis commit {}", commit.commit_id));
                    }
                    self.genesis = Some(commit.commit_id.clone());
                    self.branches.insert(
                        MAIN_BRANCH.to_owned(),
                        GraphBranchInfo {
                            name: MAIN_BRANCH.to_owned(),
                            head: commit.commit_id.clone(),
                            created_from: commit.commit_id.clone(),
                            created_at: commit.created_at.clone(),
                            created_by: None,
                        },
                    );
                } else {
                    let Some(branch) = self.branches.get_mut(&commit.branch) else {
                        return Err(format!(
                            "commit {} targets unknown branch {}",
                            commit.commit_id, commit.branch
                        ));
                    };
                    if commit.parents.first() != Some(&branch.head) {
                        return Err(format!(
                            "commit {} does not extend the head of {}",
                            commit.commit_id, commit.branch
                        ));
                    }
                    branch.head = commit.commit_id.clone();
                }
                if commit.branch == MAIN_BRANCH
                    && let Some(sequence) = commit.mutation_sequence_number
                {
                    self.last_main_sequence = Some(
                        self.last_main_sequence
                            .map_or(sequence, |last| last.max(sequence)),
                    );
                }
                for change in &commit.changes {
                    self.id_floor.observe(change);
                }
                self.order.push(commit.commit_id.clone());
                self.commits
                    .insert(commit.commit_id.clone(), Arc::new(commit));
            }
            HistoryEntry::BranchCreated {
                name,
                from,
                created_at,
                created_by,
            } => {
                if self.branches.contains_key(&name) {
                    return Err(format!("branch {name} created twice"));
                }
                if !self.commits.contains_key(&from) {
                    return Err(format!("branch {name} created from unknown commit {from}"));
                }
                self.branches.insert(
                    name.clone(),
                    GraphBranchInfo {
                        name,
                        head: from.clone(),
                        created_from: from,
                        created_at,
                        created_by,
                    },
                );
            }
            HistoryEntry::BranchDeleted { name, .. } => {
                if name == MAIN_BRANCH || self.branches.remove(&name).is_none() {
                    return Err(format!("branch {name} cannot be deleted"));
                }
            }
        }
        Ok(())
    }
}

/// Validate a branch name: 1-128 characters from `[A-Za-z0-9._/-]`, not `main`,
/// no leading or trailing `/`, no `..`.
pub fn validate_branch_name(name: &str) -> GraphStorageResult<()> {
    let reason = if name.is_empty() {
        Some("name is empty")
    } else if name.len() > MAX_BRANCH_NAME_LEN {
        Some("name is longer than 128 characters")
    } else if name == MAIN_BRANCH {
        Some("main is reserved")
    } else if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
    {
        Some("only letters, digits, '.', '_', '/' and '-' are allowed")
    } else if name.starts_with('/') || name.ends_with('/') || name.contains("..") {
        Some("leading or trailing '/' and '..' are not allowed")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(GraphStorageError::InvalidGraphBranchName {
            branch: name.to_owned(),
            reason: reason.to_owned(),
        }),
        None => Ok(()),
    }
}

fn encode_line(entry: &HistoryEntry) -> GraphStorageResult<Vec<u8>> {
    let entry_bytes = serde_json::to_vec(entry).map_err(encode_error)?;
    let line = HistoryLine {
        entry: entry.clone(),
        sha256: hex_sha256(&entry_bytes),
    };
    let mut bytes = serde_json::to_vec(&line).map_err(encode_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn decode_line(line: &[u8]) -> Result<HistoryEntry, String> {
    let decoded: HistoryLine = serde_json::from_slice(line).map_err(|error| error.to_string())?;
    let entry_bytes = serde_json::to_vec(&decoded.entry).map_err(|error| error.to_string())?;
    let actual = hex_sha256(&entry_bytes);
    if actual != decoded.sha256 {
        return Err(format!(
            "checksum mismatch: expected {}, computed {actual}",
            decoded.sha256
        ));
    }
    Ok(decoded.entry)
}

fn derive_commit_id(commit: &GraphCommit) -> GraphStorageResult<GraphCommitId> {
    #[derive(Serialize)]
    struct CommitIdentity<'a> {
        parents: &'a [GraphCommitId],
        branch: &'a str,
        transaction_id: &'a Option<String>,
        mutation_sequence_number: &'a Option<u64>,
        created_at: &'a str,
        changes: &'a [CommitRecordChange],
    }
    let bytes = serde_json::to_vec(&CommitIdentity {
        parents: &commit.parents,
        branch: &commit.branch,
        transaction_id: &commit.transaction_id,
        mutation_sequence_number: &commit.mutation_sequence_number,
        created_at: &commit.created_at,
        changes: &commit.changes,
    })
    .map_err(encode_error)?;
    let digest = hex_sha256(&bytes);
    Ok(GraphCommitId(format!(
        "{COMMIT_ID_PREFIX}{}",
        &digest[..COMMIT_ID_HEX_LEN]
    )))
}

fn read_range(path: &Path, offset: u64, length: u64) -> GraphStorageResult<Vec<u8>> {
    let mut file =
        File::open(path).map_err(|error| io_error("read_graph_history_payload", path, error))?;
    let file_len = file
        .metadata()
        .map_err(|error| io_error("read_graph_history_payload", path, error))?
        .len();
    if offset.checked_add(length).is_none_or(|end| end > file_len) {
        return Err(GraphStorageError::OperationFailed {
            operation: "read_graph_history_payload",
            message: format!(
                "payload range {offset}+{length} exceeds {} ({file_len} bytes)",
                path.display()
            ),
        });
    }
    let length = usize::try_from(length).map_err(|error| GraphStorageError::OperationFailed {
        operation: "read_graph_history_payload",
        message: error.to_string(),
    })?;
    let mut bytes = vec![0; length];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .map_err(|error| io_error("read_graph_history_payload", path, error))?;
    Ok(bytes)
}

fn sync_directory(path: &Path) -> GraphStorageResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("sync_graph_history_dir", path, error))
}

/// Lowercase hex SHA-256.
pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Current UTC time in RFC 3339 with millisecond precision.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Trailing `--N` sequence of a graph id, or zero.
pub(crate) fn sequence_suffix(value: &str) -> u64 {
    value
        .rsplit_once("--")
        .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
        .unwrap_or(0)
}

fn encode_error(error: serde_json::Error) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "encode_graph_history",
        message: error.to_string(),
    }
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> GraphStorageError {
    GraphStorageError::IoOperationFailed {
        operation,
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    }
}
