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
//! Reversible identity grouping over immutable mention and evidence records.
//!
//! The ordered ledger is authoritative for applications and reversals. Original
//! mentions, observations, links and judgments are never rewritten. Active joins
//! define a quotient view; removing a leaf join restores its original endpoints.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}

/// Immutable analyst reversal of one applied reconciliation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeUndo {
    id: String,
    reconciliation_id: ReconciliationRecordId,
    actor: ActorId,
    undone_at: TemporalTimestamp,
    rationale: String,
}
impl MergeUndo {
    /// Construct an attributed, reasoned reversal.
    pub fn new(
        id: impl Into<String>,
        reconciliation_id: ReconciliationRecordId,
        actor: ActorId,
        undone_at: TemporalTimestamp,
        rationale: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let undo = Self {
            id: id.into(),
            reconciliation_id,
            actor,
            undone_at,
            rationale: rationale.into(),
        };
        undo.validate()?;
        Ok(undo)
    }
    fn validate(&self) -> Result<(), GraphError> {
        if self.id.trim().is_empty() || self.rationale.trim().is_empty() {
            return Err(invalid("undo requires a nonblank ID and rationale"));
        }
        ReconciliationRecordId::new(self.reconciliation_id.as_str())?;
        ActorId::new(self.actor.as_str())?;
        TemporalTimestamp::new(self.undone_at.as_str())?;
        Ok(())
    }
    pub(crate) fn to_property_map(&self) -> PropertyMap {
        PropertyMap::from([
            ("undo_id".into(), PropertyValue::String(self.id.clone())),
            (
                "undo_reconciliation_id".into(),
                PropertyValue::String(self.reconciliation_id.as_str().into()),
            ),
            (
                "undo_actor".into(),
                PropertyValue::String(self.actor.as_str().into()),
            ),
            (
                "undo_at".into(),
                PropertyValue::String(self.undone_at.as_str().into()),
            ),
            (
                "undo_rationale".into(),
                PropertyValue::String(self.rationale.clone()),
            ),
        ])
    }
    /// Judgment being reversed.
    pub fn reconciliation_id(&self) -> &ReconciliationRecordId {
        &self.reconciliation_id
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Context {
        record: ReconciliationRecordId,
        dependencies: Vec<ReconciliationRecordId>,
    },
    Apply {
        record: ReconciliationRecordId,
        dependencies: Vec<ReconciliationRecordId>,
    },
    Undo {
        undo: MergeUndo,
    },
}
/// Append-only merge applications, decision dependencies and reversals.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeStore {
    events: Vec<Event>,
}
impl MergeStore {
    /// Whether no merge history exists.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    /// Retained reversal records in ledger order.
    pub fn undos(&self) -> Vec<&MergeUndo> {
        self.events
            .iter()
            .filter_map(|event| match event {
                Event::Undo { undo } => Some(undo),
                _ => None,
            })
            .collect()
    }
    /// Whether this judgment currently contributes an identity join.
    pub fn is_active(&self, id: &ReconciliationRecordId) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, Event::Apply { record, .. } if record == id))
            && !self.is_undone(id)
    }
    fn is_undone(&self, id: &ReconciliationRecordId) -> bool {
        self.undos().iter().any(|u| u.reconciliation_id() == id)
    }
    fn representatives(
        &self,
        records: &ReconciliationStore,
    ) -> Result<HashMap<EntityMentionId, EntityMentionId>, GraphError> {
        let mut roots = HashMap::new();
        for event in &self.events {
            if let Event::Apply { record, .. } = event {
                if !self.is_active(record) {
                    continue;
                }
                let r = records
                    .record_by_id(record)
                    .ok_or_else(|| invalid("merge judgment missing"))?;
                let left = roots
                    .get(r.left())
                    .cloned()
                    .unwrap_or_else(|| r.left().clone());
                let right = roots
                    .get(r.right())
                    .cloned()
                    .unwrap_or_else(|| r.right().clone());
                for root in roots.values_mut() {
                    if root == &right {
                        *root = left.clone();
                    }
                }
                roots.insert(r.right().clone(), left.clone());
                roots.insert(right, left.clone());
                roots.insert(r.left().clone(), left);
            }
        }
        Ok(roots)
    }
    fn dependencies(
        &self,
        id: &ReconciliationRecordId,
        records: &ReconciliationStore,
    ) -> Result<Vec<ReconciliationRecordId>, GraphError> {
        let r = records
            .record_by_id(id)
            .ok_or_else(|| invalid("unknown reconciliation"))?;
        let roots = self.representatives(records)?;
        let root = |id: &EntityMentionId| roots.get(id).cloned().unwrap_or_else(|| id.clone());
        let pair = [root(r.left()), root(r.right())];
        let mut dependencies = Vec::new();
        for event in &self.events {
            if let Event::Apply { record, .. } = event
                && self.is_active(record)
                && record != id
            {
                let applied = records
                    .record_by_id(record)
                    .ok_or_else(|| invalid("merge judgment missing"))?;
                if pair.contains(&root(applied.left())) || pair.contains(&root(applied.right())) {
                    dependencies.push(record.clone());
                }
            }
        }
        Ok(dependencies)
    }
    pub(crate) fn record_context(
        &mut self,
        id: &ReconciliationRecordId,
        records: &ReconciliationStore,
    ) -> Result<(), GraphError> {
        if self
            .events
            .iter()
            .any(|e| matches!(e, Event::Context { record, .. } if record == id))
        {
            return Ok(());
        }
        let dependencies = self.dependencies(id, records)?;
        self.events.push(Event::Context {
            record: id.clone(),
            dependencies,
        });
        Ok(())
    }
    fn apply(
        &mut self,
        id: &ReconciliationRecordId,
        records: &ReconciliationStore,
    ) -> Result<(), GraphError> {
        let record = records
            .record_by_id(id)
            .ok_or_else(|| invalid("unknown reconciliation"))?;
        if record.outcome() != ReconciliationOutcome::Merge {
            return Err(invalid("only a Merge judgment may be applied"));
        }
        if self.is_undone(id) {
            return Err(invalid(
                "a reversed merge requires a new reconciliation record",
            ));
        }
        if self.is_active(id) {
            return Ok(());
        }
        let dependencies = self.dependencies(id, records)?;
        self.events.push(Event::Apply {
            record: id.clone(),
            dependencies,
        });
        Ok(())
    }
    /// First unreversed judgment depending on this merge, including Distinct or Abstain.
    pub fn dependent_record(&self, id: &ReconciliationRecordId) -> Option<&ReconciliationRecordId> {
        self.events.iter().find_map(|event| match event {
            Event::Context {
                record,
                dependencies,
            }
            | Event::Apply {
                record,
                dependencies,
            } if record != id && !self.is_undone(record) && dependencies.contains(id) => {
                Some(record)
            }
            _ => None,
        })
    }
    fn undo(&mut self, undo: MergeUndo) -> Result<(), GraphError> {
        undo.validate()?;
        if let Some(old) = self.undos().into_iter().find(|old| old.id == undo.id) {
            return if old == &undo {
                Ok(())
            } else {
                Err(invalid("immutable undo ID conflict"))
            };
        }
        if !self.is_active(&undo.reconciliation_id) {
            return Err(invalid("reconciliation is not an active merge"));
        }
        if let Some(dependent) = self.dependent_record(&undo.reconciliation_id) {
            return Err(GraphError::DependentReconciliation {
                merge_record: undo.reconciliation_id.clone(),
                dependent_record: dependent.clone(),
            });
        }
        self.events.push(Event::Undo { undo });
        Ok(())
    }
    pub(crate) fn validate_bindings(
        &self,
        records: &ReconciliationStore,
    ) -> Result<(), GraphError> {
        // Replay through the same transition functions. Exact equality rejects
        // forged dependencies, duplicate events and illegal dependent reversals.
        let mut replay = Self::default();
        for event in &self.events {
            let before = replay.events.len();
            match event {
                Event::Context { record, .. } => replay.record_context(record, records)?,
                Event::Apply { record, .. } => replay.apply(record, records)?,
                Event::Undo { undo } => replay.undo(undo.clone())?,
            }
            if replay.events.len() != before + 1 || replay.events.last() != Some(event) {
                return Err(invalid("invalid reconciliation merge ledger"));
            }
        }
        Ok(())
    }
}
impl Graph {
    /// Group mentions under an evidence-cited Merge judgment. Exact retries are no-ops.
    pub fn apply_reconciliation_merge(
        &mut self,
        id: &ReconciliationRecordId,
    ) -> Result<(), GraphError> {
        let stores = self.epistemic_stores_mut();
        stores.merges.validate_bindings(&stores.reconciliations)?;
        stores.merges.apply(id, &stores.reconciliations)
    }
    /// Reverse an independent merge atomically, retaining both records.
    pub fn undo_reconciliation_merge(&mut self, undo: MergeUndo) -> Result<(), GraphError> {
        let stores = self.epistemic_stores_mut();
        stores.merges.validate_bindings(&stores.reconciliations)?;
        stores.merges.undo(undo)
    }
    /// Current representative for every retained mention, replayed once per view.
    pub fn resolved_mentions(
        &self,
    ) -> Result<HashMap<EntityMentionId, EntityMentionId>, GraphError> {
        let stores = self.epistemic_stores();
        stores.merges.validate_bindings(&stores.reconciliations)?;
        let roots = stores.merges.representatives(&stores.reconciliations)?;
        Ok(stores
            .mentions
            .mentions()
            .iter()
            .map(|mention| {
                (
                    mention.id().clone(),
                    roots
                        .get(mention.id())
                        .cloned()
                        .unwrap_or_else(|| mention.id().clone()),
                )
            })
            .collect())
    }
    /// Current identity representative; original mentions and links remain immutable.
    pub fn resolved_mention(&self, id: &EntityMentionId) -> Result<EntityMentionId, GraphError> {
        self.resolved_mentions()?
            .remove(id)
            .ok_or_else(|| invalid("unknown entity mention"))
    }
}
