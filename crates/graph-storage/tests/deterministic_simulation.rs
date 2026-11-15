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

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Issue #328: deterministic simulation of the graph write path.
//!
//! Each universe is seeded. It drives random main writes, branch creation,
//! branch writes, merges, history cleanup, reopens, torn history tails and
//! crashes injected at every WAL stage through the real store, while a small
//! in-memory model records what every branch must contain. After each step
//! the oracles check:
//!
//! - model differential: every branch head reads exactly what the model holds;
//! - history continuity: first-parent generations strictly decrease to a
//!   root, every listed commit is retrievable, and the main head view equals
//!   the canonical catalog;
//! - atomicity after a crash: once reopened, main holds either the state
//!   before the interrupted write or the state after it, never a mixture;
//! - rerun determinism: replaying a seed gives the same operation outcomes
//!   and the same final model.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, GraphMergeRequest, HistoryCleanupRequest, HistoryRecordKind, MAIN_BRANCH,
    MergeOutcomeKind, MutationCrashStage, RecordFormat, StorageManifest, StorageRoot,
    StorageTimestamp, StorageVersion, TransitionCommitContext, create_storage_root,
};

/// Key → value of every live record on a branch.
type State = BTreeMap<String, i64>;

/// Deterministic xorshift64* generator; no dependency, stable across platforms.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(usize::try_from(self.below(items.len() as u64)).unwrap_or(0))
        }
    }
}

struct Universe {
    root: StorageRoot,
    store: Option<CanonicalEngineStore>,
    model: BTreeMap<String, State>,
    /// Main state when each live branch was created (its merge base).
    bases: BTreeMap<String, State>,
    rng: Rng,
    next_key: u64,
    next_value: i64,
    next_tx: u64,
    outcomes: Vec<String>,
}

/// Two tests may build the universe of the same seed within the clock's
/// resolution (about a microsecond on macOS); a counter keeps their storage
/// roots apart (#370).
static ROOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_root(seed: u64) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-dst-{seed}-{}-{unique}-{}",
        std::process::id(),
        ROOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--dst".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

fn key_of(node: &graph_core::Node) -> Option<String> {
    match node.properties().get("key") {
        Some(PropertyValue::String(key)) => Some(key.clone()),
        _ => None,
    }
}

fn value_of(node: &graph_core::Node) -> i64 {
    match node.properties().get("value") {
        Some(PropertyValue::Integer(value)) => *value,
        other => panic!("record without integer value: {other:?}"),
    }
}

/// A planned change on one branch: create, update or delete records by key.
enum Change {
    Create(String, i64),
    Update(String, i64),
    Delete(String),
}

fn apply_to_graph(graph: &mut Graph, change: &Change) {
    let find = |graph: &Graph, key: &str| {
        graph
            .current_node_records()
            .unwrap()
            .into_iter()
            .find(|node| {
                node.status() != graph_core::RecordStatus::Tombstoned
                    && key_of(node).as_deref() == Some(key)
            })
            .unwrap_or_else(|| panic!("record {key} is missing from the projection"))
    };
    match change {
        Change::Create(key, value) => {
            graph
                .create_node(
                    NodeInput::new(["Sim"])
                        .with_property("key", PropertyValue::String(key.clone()))
                        .with_property("value", PropertyValue::Integer(*value)),
                )
                .unwrap();
        }
        Change::Update(key, value) => {
            let node = find(graph, key);
            graph
                .update_node(
                    node.id(),
                    NodePatch::default().set_property("value", PropertyValue::Integer(*value)),
                )
                .unwrap();
        }
        Change::Delete(key) => {
            let node = find(graph, key);
            graph.tombstone_node(node.id()).unwrap();
        }
    }
}

fn apply_to_state(state: &mut State, change: &Change) {
    match change {
        Change::Create(key, value) | Change::Update(key, value) => {
            state.insert(key.clone(), *value);
        }
        Change::Delete(key) => {
            state.remove(key);
        }
    }
}

impl Universe {
    fn new(seed: u64) -> Self {
        let root = temp_root(seed);
        let store =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        Self {
            root,
            store: Some(store),
            model: BTreeMap::from([(MAIN_BRANCH.to_owned(), State::new())]),
            bases: BTreeMap::new(),
            rng: Rng::new(seed),
            next_key: 0,
            next_value: 0,
            next_tx: 0,
            outcomes: Vec::new(),
        }
    }

    fn store(&mut self) -> &mut CanonicalEngineStore {
        self.store.as_mut().unwrap()
    }

    fn reopen(&mut self) {
        self.store = None;
        self.store = Some(
            CanonicalEngineStore::open(self.root.clone(), CanonicalStoreOptions::default())
                .unwrap(),
        );
    }

    fn plan_change(&mut self, branch: &str) -> Change {
        let keys: Vec<String> = self.model[branch].keys().cloned().collect();
        self.next_value += 1;
        let value = self.next_value;
        match (self.rng.below(3), self.rng.pick(&keys).cloned()) {
            (1, Some(key)) => Change::Update(key, value),
            (2, Some(key)) => Change::Delete(key),
            _ => {
                self.next_key += 1;
                Change::Create(format!("k{}", self.next_key), value)
            }
        }
    }

    fn main_write(&mut self, crash: Option<MutationCrashStage>) {
        let change = self.plan_change(MAIN_BRANCH);
        self.next_tx += 1;
        let transaction = format!("tx--dst-{}", self.next_tx);
        let store = self.store();
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .unwrap();
        let mut current = previous.clone();
        apply_to_graph(&mut current, &change);
        let result = store.commit_transition_with_context(
            &previous,
            &current,
            DurableTransactionId::new(transaction).unwrap(),
            TransitionCommitContext::default(),
            crash,
        );
        match (crash, result) {
            (None, Ok(_)) => {
                apply_to_state(self.model.get_mut(MAIN_BRANCH).unwrap(), &change);
                self.outcomes.push("main:committed".to_owned());
            }
            (Some(stage), Err(_)) => {
                // The process stops: reopen, then main must hold exactly the
                // state before or the state after the interrupted write.
                self.reopen();
                let before = self.model[MAIN_BRANCH].clone();
                let mut after = before.clone();
                apply_to_state(&mut after, &change);
                let observed = self.read(MAIN_BRANCH);
                assert!(
                    observed == before || observed == after,
                    "crash at {stage:?} left a mixed state: {observed:?} is neither {before:?} nor {after:?}"
                );
                let survived = observed == after && before != after;
                if survived {
                    *self.model.get_mut(MAIN_BRANCH).unwrap() = after;
                }
                self.outcomes
                    .push(format!("main:crash:{stage:?}:{survived}"));
            }
            (crash, result) => panic!("unexpected write outcome {result:?} for crash {crash:?}"),
        }
    }

    fn branch_create(&mut self) {
        let name = format!("sim/{}", self.next_tx);
        self.next_tx += 1;
        self.store()
            .create_graph_branch(&name, None, None, None)
            .unwrap();
        let main = self.model[MAIN_BRANCH].clone();
        self.model.insert(name.clone(), main.clone());
        self.bases.insert(name, main);
        self.outcomes.push("branch:created".to_owned());
    }

    fn live_branches(&self) -> Vec<String> {
        self.model
            .keys()
            .filter(|name| *name != MAIN_BRANCH)
            .cloned()
            .collect()
    }

    fn branch_write(&mut self) {
        let Some(branch) = self.rng.pick(&self.live_branches()).cloned() else {
            return self.branch_create();
        };
        let change = self.plan_change(&branch);
        let store = self.store();
        let head = store.graph_head(&branch).unwrap();
        let previous = store
            .load_commit_projection(&head, &CanonicalProjectionRequest::all())
            .unwrap();
        let mut current = previous.clone();
        apply_to_graph(&mut current, &change);
        store
            .commit_branch_transition(
                &branch,
                &previous,
                &current,
                TransitionCommitContext::default(),
                Some(&head),
            )
            .unwrap();
        apply_to_state(self.model.get_mut(&branch).unwrap(), &change);
        self.outcomes.push("branch:committed".to_owned());
    }

    fn merge(&mut self) {
        let Some(branch) = self.rng.pick(&self.live_branches()).cloned() else {
            return;
        };
        let base = self.bases[&branch].clone();
        let source = self.model[&branch].clone();
        let target = self.model[MAIN_BRANCH].clone();
        // Model three-way merge by key; values are unique, so equal values
        // mean the same version.
        let mut merged = target.clone();
        let mut conflict = false;
        let keys: std::collections::BTreeSet<&String> = base
            .keys()
            .chain(source.keys())
            .chain(target.keys())
            .collect();
        for key in keys {
            let (b, s, t) = (base.get(key), source.get(key), target.get(key));
            let source_changed = s != b;
            let target_changed = t != b;
            if source_changed && target_changed && s != t {
                conflict = true;
            } else if source_changed {
                match s {
                    Some(value) => merged.insert(key.clone(), *value),
                    None => merged.remove(key),
                };
            }
        }
        let report = self
            .store()
            .merge_graph_branch(GraphMergeRequest {
                source: branch.clone(),
                delete_source: true,
                ..GraphMergeRequest::default()
            })
            .unwrap();
        if conflict {
            assert_eq!(report.outcome, MergeOutcomeKind::Conflicted, "{report:?}");
            self.outcomes.push("merge:conflicted".to_owned());
            // Keep the conflicting branch out of later merges.
            self.store()
                .delete_graph_branch(&branch, true, None)
                .unwrap();
        } else {
            assert_ne!(
                report.outcome,
                MergeOutcomeKind::Conflicted,
                "conflicts {:?}\nbase {base:?}\nsource {source:?}\ntarget {target:?}\noutcomes {:?}",
                report.conflicts,
                self.outcomes
            );
            *self.model.get_mut(MAIN_BRANCH).unwrap() = merged;
            self.outcomes.push(format!("merge:{:?}", report.outcome));
        }
        self.model.remove(&branch);
        self.bases.remove(&branch);
    }

    fn cleanup(&mut self) {
        let keep = usize::try_from(self.rng.below(3) + 1).unwrap_or(1);
        self.store()
            .cleanup_graph_history(&HistoryCleanupRequest {
                keep_commits: keep,
                keep_seconds: None,
                dry_run: false,
            })
            .unwrap();
        self.outcomes.push("cleanup".to_owned());
    }

    fn tear_history_tail(&mut self) {
        self.store = None;
        let log = self.root.path().join("history").join("graph_history.log");
        let mut file = OpenOptions::new().append(true).open(&log).unwrap();
        file.write_all(b"{\"entry\":{\"type\":\"commit\",\"commit_id\":\"gc")
            .unwrap();
        drop(file);
        self.reopen();
        self.outcomes.push("torn-tail".to_owned());
    }

    fn read(&mut self, branch: &str) -> State {
        let store = self.store();
        let head = store.graph_head(branch).unwrap();
        let mut state = State::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = store
                .list_commit_records(
                    &head,
                    HistoryRecordKind::Node,
                    Some("Sim"),
                    cursor.as_deref(),
                    500,
                    false,
                )
                .unwrap();
            for node in &page.nodes {
                let key = key_of(node).unwrap();
                assert!(
                    state.insert(key.clone(), value_of(node)).is_none(),
                    "duplicate live key {key}"
                );
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        state
    }

    fn check_oracles(&mut self, step: usize) {
        for branch in self.model.keys().cloned().collect::<Vec<_>>() {
            let observed = self.read(&branch);
            assert_eq!(
                observed, self.model[&branch],
                "step {step}: {branch} diverged from the model"
            );
        }
        let store = self.store();
        let page = store.list_graph_commits(MAIN_BRANCH, None, 10_000).unwrap();
        for pair in page.commits.windows(2) {
            assert!(
                pair[0].generation > pair[1].generation,
                "step {step}: generations must decrease"
            );
            assert_eq!(
                pair[0].parents.first(),
                Some(&pair[1].commit_id),
                "step {step}: broken first-parent chain"
            );
        }
        let last = page.commits.last().unwrap();
        assert!(
            last.parents.is_empty(),
            "step {step}: the chain must end at a root"
        );
        for commit in &page.commits {
            store.graph_commit(&commit.commit_id).unwrap();
        }
        let head = store.graph_head(MAIN_BRANCH).unwrap();
        let live_catalog: usize = store.catalog().latest_node_records.len();
        let view_total = store
            .list_commit_records(&head, HistoryRecordKind::Node, None, None, 1, true)
            .unwrap()
            .total;
        assert_eq!(
            view_total, live_catalog,
            "step {step}: main head view and catalog disagree"
        );
    }

    fn run(&mut self, steps: usize) {
        for step in 0..steps {
            match self.rng.below(100) {
                0..=34 => self.main_write(None),
                35..=44 => {
                    let stage = *self
                        .rng
                        .pick(&[
                            MutationCrashStage::AfterWalIntent,
                            MutationCrashStage::AfterPayloadRecords,
                            MutationCrashStage::BeforeAppliedMarker,
                            MutationCrashStage::BeforeCheckpointWrite,
                        ])
                        .unwrap();
                    self.main_write(Some(stage));
                }
                45..=54 => self.branch_create(),
                55..=74 => self.branch_write(),
                75..=84 => self.merge(),
                85..=89 => self.cleanup(),
                90..=94 => {
                    self.reopen();
                    self.outcomes.push("reopen".to_owned());
                }
                _ => self.tear_history_tail(),
            }
            self.check_oracles(step);
        }
    }
}

impl Drop for Universe {
    fn drop(&mut self) {
        self.store = None;
        let _ = fs::remove_dir_all(self.root.path());
    }
}

const SEEDS: u64 = 16;
const STEPS: usize = 40;

#[test]
fn seeded_universes_keep_every_oracle() {
    for seed in 1..=SEEDS {
        let outcome = std::panic::catch_unwind(|| Universe::new(seed).run(STEPS));
        if let Err(panic) = outcome {
            eprintln!(
                "universe seed {seed} broke an oracle; rerun Universe::new({seed}) to replay it"
            );
            std::panic::resume_unwind(panic);
        }
    }
}

#[test]
fn replaying_a_seed_is_deterministic() {
    for seed in [3_u64, 11] {
        let mut first = Universe::new(seed);
        first.run(STEPS);
        let mut second = Universe::new(seed);
        second.run(STEPS);
        assert_eq!(
            first.outcomes, second.outcomes,
            "seed {seed}: outcomes differ"
        );
        assert_eq!(
            first.model, second.model,
            "seed {seed}: final models differ"
        );
    }
}

#[test]
fn a_universe_exercises_every_operation() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 1..=SEEDS {
        let mut universe = Universe::new(seed);
        universe.run(STEPS);
        for outcome in &universe.outcomes {
            seen.insert(outcome.split(':').take(2).collect::<Vec<_>>().join(":"));
        }
    }
    for expected in [
        "main:committed",
        "main:crash",
        "branch:created",
        "branch:committed",
        "cleanup",
        "reopen",
        "torn-tail",
    ] {
        assert!(
            seen.contains(expected),
            "no universe exercised {expected}: {seen:?}"
        );
    }
    assert!(
        seen.iter().any(|outcome| outcome.starts_with("merge:")),
        "no universe merged: {seen:?}"
    );
}
