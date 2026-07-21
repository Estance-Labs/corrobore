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
//! Bitemporal fact model and as-of query semantics (Epic 0018).
//!
//!
//!
//! - Represent successive and contradictory fact states without overwrite:
//!   history is append-only, contradictions coexist, and nothing collapses
//!   into a canonical value at this layer.
//! - Distinguish the epic's four temporal dimensions per state: `valid_time`
//!   (when the fact was true in the world, as an interval), `transaction_time`
//!   (when the engine recorded the state), `observation_time` (when the source
//!   observed it), and `publication_time` (when the source published it).
//! - Answer as-of questions deterministically at a valid time and, optionally,
//!   at a transaction time reflecting exactly what the engine knew then.
//! - Forbid overwrite-based temporal updates at the contract level: recording
//!   at or before the engine's own latest transaction time for a fact is a
//!   typed error, never a silent rewrite.
//! - Keep FIMI/CTI domain temporal rules outside graph-core; these are the
//!   neutral primitives they build on.
//!
//! # Canonical comparable form
//!
//! Chronological comparison uses the lexicographic order of the timestamp
//! strings, which is only correct when every stamp uses the canonical
//! second-precision UTC form `YYYY-MM-DDThh:mm:ssZ`. Stamp construction
//! enforces that form with a typed error so chronology never depends on
//! caller formatting.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{GraphError, ids::FactId, temporal::TemporalTimestamp};

/// Validated four-dimension temporal stamp of one fact state.
///
///
/// carry the epic's temporal dimensions as one validated value so every fact
/// state answers when it was true, when the engine learned it, and when the
/// source observed and published it.
///
///
/// hold the valid interval, the transaction time, and the optional source
/// times; construction enforces the canonical comparable form and interval
/// order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitemporalStamp {
    /// Start of the interval during which the fact was true in the world.
    pub valid_from: TemporalTimestamp,

    /// End of the valid interval; `None` means still valid or unknown.
    pub valid_to: Option<TemporalTimestamp>,

    /// Time the engine recorded this state.
    pub transaction_time: TemporalTimestamp,

    /// Time the source observed the fact, when known.
    pub observation_time: Option<TemporalTimestamp>,

    /// Time the source published the fact, when known.
    pub publication_time: Option<TemporalTimestamp>,
}

impl BitemporalStamp {
    /// Build a stamp from its two mandatory dimensions.
    ///
    ///
    /// make the world time and the engine time mandatory: a state without
    /// them cannot participate in as-of queries.
    ///
    ///
    /// validate the canonical comparable form of both timestamps; the valid
    /// interval starts open-ended and source times start unknown.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidBitemporalStamp` when a timestamp is not in
    /// canonical second-precision UTC form.
    pub fn new(
        valid_from: TemporalTimestamp,
        transaction_time: TemporalTimestamp,
    ) -> Result<Self, GraphError> {
        require_canonical(&valid_from)?;
        require_canonical(&transaction_time)?;

        Ok(Self {
            valid_from,
            valid_to: None,
            transaction_time,
            observation_time: None,
            publication_time: None,
        })
    }

    /// Close the valid interval.
    ///
    ///
    /// represent facts that stopped being true without deleting or rewriting
    /// their state.
    ///
    ///
    /// validate the canonical form and that the interval end follows its
    /// start, then attach it.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidBitemporalStamp` for a non-canonical or
    /// inverted interval end.
    pub fn with_valid_to(mut self, valid_to: TemporalTimestamp) -> Result<Self, GraphError> {
        require_canonical(&valid_to)?;
        if valid_to.as_str() <= self.valid_from.as_str() {
            return Err(GraphError::InvalidBitemporalStamp(format!(
                "valid_to {} does not follow valid_from {}",
                valid_to.as_str(),
                self.valid_from.as_str()
            )));
        }

        self.valid_to = Some(valid_to);
        Ok(self)
    }

    /// Attach the source observation time.
    ///
    ///
    /// keep source chronology separate from world and engine chronology.
    ///
    ///
    /// attach the timestamp as-is; observation time participates in no
    /// interval comparison at this layer.
    ///
    /// # Errors
    ///
    /// none expected because the value only annotates the state.
    pub fn with_observation_time(mut self, observation_time: TemporalTimestamp) -> Self {
        self.observation_time = Some(observation_time);
        self
    }

    /// Attach the source publication time.
    ///
    ///
    /// keep source chronology separate from world and engine chronology.
    ///
    ///
    /// attach the timestamp as-is; publication time participates in no
    /// interval comparison at this layer.
    ///
    /// # Errors
    ///
    /// none expected because the value only annotates the state.
    pub fn with_publication_time(mut self, publication_time: TemporalTimestamp) -> Self {
        self.publication_time = Some(publication_time);
        self
    }
}

/// One recorded state of one bitemporal fact.
///
///
/// pair the asserted statement with its temporal stamp so contradictory and
/// successive statements stay distinct records.
///
///
/// carry the statement text and the validated stamp.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitemporalFactState {
    /// Asserted statement of this state.
    pub statement: String,

    /// Validated temporal stamp of this state.
    pub stamp: BitemporalStamp,
}

/// Append-only store of bitemporal facts.
///
///
/// own the epic's no-overwrite contract: states are only ever appended in
/// strictly increasing transaction order per fact, and queries read history
/// without mutating it.
///
///
/// keep one append-only state list per fact identifier and answer history and
/// as-of queries deterministically.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitemporalFactStore {
    facts: HashMap<FactId, Vec<BitemporalFactState>>,
}

impl BitemporalFactStore {
    /// Create an empty store.
    ///
    ///
    /// provide the stable constructor used before any assertion.
    ///
    ///
    /// initialize an empty fact map.
    ///
    /// # Errors
    ///
    /// none expected because an empty store has no external dependency.
    pub fn new() -> Self {
        Self::default()
    }

    /// Assert one new state of a fact.
    ///
    ///
    /// keep history append-only: successive and contradictory states coexist,
    /// and recording into the engine's own past is impossible.
    ///
    ///
    /// append the state when its transaction time strictly follows the fact's
    /// latest transaction time.
    ///
    /// # Errors
    ///
    /// return `GraphError::BitemporalOverwriteForbidden` when the transaction
    /// time does not strictly follow the latest recorded state of the fact.
    pub fn assert_fact_state(
        &mut self,
        fact_id: FactId,
        statement: impl Into<String>,
        stamp: BitemporalStamp,
    ) -> Result<(), GraphError> {
        let states = self.facts.entry(fact_id.clone()).or_default();

        if let Some(latest) = states.last()
            && stamp.transaction_time.as_str() <= latest.stamp.transaction_time.as_str()
        {
            return Err(GraphError::BitemporalOverwriteForbidden(fact_id));
        }

        states.push(BitemporalFactState {
            statement: statement.into(),
            stamp,
        });
        Ok(())
    }

    /// Return the full recorded history of a fact in transaction order.
    ///
    ///
    /// expose every state — successive and contradictory — for audit and
    /// downstream arbitration.
    ///
    ///
    /// return the append-only state list; unknown facts read as empty.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty history.
    pub fn fact_history(&self, fact_id: &FactId) -> &[BitemporalFactState] {
        self.facts.get(fact_id).map_or(&[], Vec::as_slice)
    }

    /// Query the states of a fact as of a valid time and optional transaction time.
    ///
    ///
    /// answer the epic's as-of questions deterministically: what was true in
    /// the world at a time, as known by the engine at a time.
    ///
    ///
    /// return, in transaction order, every state whose valid interval covers
    /// the valid time and whose transaction time is not later than the given
    /// transaction bound; contradictory states are all returned.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty result.
    pub fn states_as_of(
        &self,
        fact_id: &FactId,
        valid_at: &TemporalTimestamp,
        transaction_at: Option<&TemporalTimestamp>,
    ) -> Vec<&BitemporalFactState> {
        self.fact_history(fact_id)
            .iter()
            .filter(|state| {
                let known = transaction_at
                    .is_none_or(|bound| state.stamp.transaction_time.as_str() <= bound.as_str());
                let started = state.stamp.valid_from.as_str() <= valid_at.as_str();
                let not_ended = state
                    .stamp
                    .valid_to
                    .as_ref()
                    .is_none_or(|end| valid_at.as_str() < end.as_str());
                known && started && not_ended
            })
            .collect()
    }
}

fn require_canonical(timestamp: &TemporalTimestamp) -> Result<(), GraphError> {
    let value = timestamp.as_str();
    if value.len() != 20 || !value.ends_with('Z') {
        return Err(GraphError::InvalidBitemporalStamp(format!(
            "timestamp {value} is not in canonical second-precision UTC form"
        )));
    }
    Ok(())
}
