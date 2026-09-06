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
//! Human judgments beside machine records, never replacements for them.
use crate::{ActorId, ClaimId, Graph, GraphError, TemporalTimestamp};
use serde::{Deserialize, Serialize};
/// The human action; reversal is a new record pointing to a prior judgment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalystDecisionAction {
    /// A human note without a replacement conclusion.
    Annotation {
        /// Human note.
        text: String,
    },
    /// An analyst conclusion beside the unchanged machine verdict.
    Override {
        /// Human conclusion beside the machine verdict.
        judgment: String,
        /// Reason for this conclusion.
        rationale: String,
    },
    /// Withdrawal of a previous annotation or override on this same claim.
    Reversal {
        /// Earlier annotation or override to withdraw.
        decision_id: String,
        /// Reason for this human action.
        rationale: String,
    },
}
/// Immutable attributed human judgment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalystDecision {
    id: String,
    claim_id: ClaimId,
    actor: ActorId,
    recorded_at: TemporalTimestamp,
    action: AnalystDecisionAction,
}
impl AnalystDecision {
    fn validate(&self) -> Result<(), GraphError> {
        required(&self.id)?;
        ClaimId::new(self.claim_id.as_str())?;
        ActorId::new(self.actor.as_str())?;
        TemporalTimestamp::new(self.recorded_at.as_str())?;
        match &self.action {
            AnalystDecisionAction::Annotation { text } => required(text)?,
            AnalystDecisionAction::Override {
                judgment,
                rationale,
            } => {
                required(judgment)?;
                required(rationale)?;
            }
            AnalystDecisionAction::Reversal {
                decision_id,
                rationale,
            } => {
                required(decision_id)?;
                required(rationale)?;
            }
        }
        Ok(())
    }
    /// Validate required attribution and action text before accepting a record.
    pub fn new(
        id: impl Into<String>,
        claim_id: ClaimId,
        actor: ActorId,
        recorded_at: TemporalTimestamp,
        action: AnalystDecisionAction,
    ) -> Result<Self, GraphError> {
        let record = Self {
            id: id.into(),
            claim_id,
            actor,
            recorded_at,
            action,
        };
        record.validate()?;
        Ok(record)
    }
}
/// Append-only human ledger, kept separate from machine stores.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalystDecisionStore {
    records: Vec<AnalystDecision>,
}
fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
fn required(value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(invalid("analyst decision fields must not be empty"));
    }
    Ok(())
}
impl AnalystDecisionStore {
    fn append(&mut self, record: AnalystDecision) -> Result<String, GraphError> {
        record.validate()?;
        if let Some(existing) = self.records.iter().find(|r| r.id == record.id) {
            return if existing == &record {
                Ok(record.id)
            } else {
                Err(invalid("immutable analyst decision identifier conflict"))
            };
        }
        if let AnalystDecisionAction::Reversal { decision_id, .. } = &record.action {
            let original = self
                .records
                .iter()
                .find(|r| &r.id == decision_id)
                .ok_or_else(|| invalid("reversal target does not exist"))?;
            if original.claim_id != record.claim_id
                || matches!(original.action, AnalystDecisionAction::Reversal { .. })
            {
                return Err(invalid(
                    "reversal must target an annotation or override on the same claim",
                ));
            }
            if self.records.iter().any(|r| matches!(&r.action, AnalystDecisionAction::Reversal { decision_id: target, .. } if target == decision_id)) {
                return Err(invalid("analyst decision has already been reversed"));
            }
        }
        let id = record.id.clone();
        self.records.push(record);
        Ok(id)
    }
    pub(crate) fn validate(&self, claims: &crate::ClaimStore) -> Result<(), GraphError> {
        let mut replay = Self::default();
        for record in &self.records {
            claims.claim_by_id(&record.claim_id)?;
            let previous = replay.records.len();
            replay.append(record.clone())?;
            if replay.records.len() == previous {
                return Err(invalid("duplicate analyst ledger record"));
            }
        }
        Ok(())
    }

    /// Whether the ledger has no decisions.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    /// All retained human decisions on this claim in append order.
    pub fn records_for_claim(&self, claim: &ClaimId) -> Vec<&AnalystDecision> {
        self.records
            .iter()
            .filter(|r| &r.claim_id == claim)
            .collect()
    }
}
impl Graph {
    /// Append atomically after attribution, target and reversal validation.
    /// Only the human ledger may change; retries must be idempotent.
    pub fn record_analyst_decision(
        &mut self,
        record: AnalystDecision,
    ) -> Result<String, GraphError> {
        let stores = self.epistemic_stores_mut();
        stores.claims.claim_by_id(&record.claim_id)?;
        stores.analyst_decisions.append(record)
    }
}
