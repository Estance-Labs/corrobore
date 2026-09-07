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
//! The parser-backed evaluation harness.
//!
//! Module boundary: this module runs a compiler over examples, validates every
//! envelope through [`crate::validate`] exactly as a host would, and counts
//! what matters for a release gate: schema validity, canonical equivalence,
//! action-class agreement, invented evidence, writes under read-only tasks,
//! and unsupported requests recompiled into something else. Every metric is
//! reported per language and in aggregate; the French and English surfaces of
//! one record are also compared with each other.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Action, ActionKind, NlqCompiler, NlqRequest, RejectionCode, TrustBoundary,
    dataset::{Example, ExpectedOutcome},
    validate,
};

/// Counts and rates for one slice of examples.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Examples in the slice.
    pub examples: usize,
    /// Fraction of envelopes [`validate`] accepted.
    pub schema_valid_rate: f64,
    /// Fraction whose canonical action (or terminal kind) equals the expected one.
    pub canonical_accuracy: f64,
    /// Fraction whose action kind equals the expected one.
    pub action_class_accuracy: f64,
    /// Among examples expecting a terminal result, the fraction that got the
    /// right one.
    pub abstention_accuracy: f64,
    /// Envelopes refused for citing evidence the caller did not supply.
    pub invented_evidence: usize,
    /// Envelopes that proposed a write under a read-only task.
    pub read_only_writes: usize,
    /// Examples expecting `unsupported` that compiled into an action instead.
    pub unsupported_recompiled: usize,
}

#[derive(Default)]
struct Tally {
    examples: usize,
    schema_valid: usize,
    canonical_hits: usize,
    class_hits: usize,
    terminal_expected: usize,
    terminal_hits: usize,
    invented_evidence: usize,
    read_only_writes: usize,
    unsupported_recompiled: usize,
}

impl Tally {
    fn metrics(&self) -> Metrics {
        let rate = |hits: usize, total: usize| -> f64 {
            if total == 0 {
                1.0
            } else {
                hits as f64 / total as f64
            }
        };
        Metrics {
            examples: self.examples,
            schema_valid_rate: rate(self.schema_valid, self.examples),
            canonical_accuracy: rate(self.canonical_hits, self.examples),
            action_class_accuracy: rate(self.class_hits, self.examples),
            abstention_accuracy: rate(self.terminal_hits, self.terminal_expected),
            invented_evidence: self.invented_evidence,
            read_only_writes: self.read_only_writes,
            unsupported_recompiled: self.unsupported_recompiled,
        }
    }

    fn absorb(&mut self, other: &Tally) {
        self.examples += other.examples;
        self.schema_valid += other.schema_valid;
        self.canonical_hits += other.canonical_hits;
        self.class_hits += other.class_hits;
        self.terminal_expected += other.terminal_expected;
        self.terminal_hits += other.terminal_hits;
        self.invented_evidence += other.invented_evidence;
        self.read_only_writes += other.read_only_writes;
        self.unsupported_recompiled += other.unsupported_recompiled;
    }
}

/// An evaluation report, per language and aggregate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The compiler evaluated.
    pub compiler: String,
    /// Metrics keyed by language tag.
    pub per_language: BTreeMap<String, Metrics>,
    /// Metrics over every example.
    pub aggregate: Metrics,
    /// Fraction of semantic records whose language surfaces compiled to the
    /// same canonical action (or the same terminal kind).
    pub pair_agreement_rate: f64,
}

/// What one example compiled to, reduced to what the pair comparison needs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Rejected,
    Terminal(ActionKind),
    Action(String),
}

/// Whether an envelope proposes a write, judged from the raw action so a
/// refused envelope is still counted.
fn proposes_write(action: &Action) -> bool {
    match action {
        Action::CypherWriteProposal { .. } => true,
        Action::MemoryOperation { operation, .. } => {
            matches!(
                operation.as_str(),
                "remember" | "relate" | "update" | "forget" | "consolidate"
            )
        }
        _ => false,
    }
}

/// Evaluate `compiler` over `examples`.
#[must_use]
pub fn evaluate(compiler: &dyn NlqCompiler, examples: &[Example]) -> Report {
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, Vec<Outcome>> = BTreeMap::new();

    for example in examples {
        let request = NlqRequest::new(example.text.clone())
            .with_language(example.language.clone())
            .with_evidence_refs(example.evidence_refs.iter().cloned())
            .allow_writes(example.allow_writes);
        let boundary =
            TrustBoundary::new(example.evidence_refs.iter().cloned(), example.allow_writes);
        let envelope = compiler.compile(&request);
        let tally = tallies
            .entry(example.language.tag().to_owned())
            .or_default();
        tally.examples += 1;
        if !example.allow_writes && proposes_write(&envelope.action) {
            tally.read_only_writes += 1;
        }
        let expected_terminal = match &example.expected {
            ExpectedOutcome::Terminal(kind) => Some(*kind),
            ExpectedOutcome::Action { .. } => None,
        };
        if expected_terminal.is_some() {
            tally.terminal_expected += 1;
        }

        let json = match serde_json::to_value(&envelope) {
            Ok(json) => json,
            Err(_) => {
                outcomes
                    .entry(example.record_id.clone())
                    .or_default()
                    .push(Outcome::Rejected);
                continue;
            }
        };
        let outcome = match validate(&json, &boundary) {
            Ok(validated) => {
                tally.schema_valid += 1;
                let kind = validated.kind();
                let (class_hit, canonical_hit) = match &example.expected {
                    ExpectedOutcome::Action {
                        kind: expected_kind,
                        canonical,
                        ..
                    } => (
                        kind == *expected_kind,
                        kind == *expected_kind && validated.canonical() == canonical,
                    ),
                    ExpectedOutcome::Terminal(expected_kind) => {
                        let hit = kind == *expected_kind;
                        (hit, hit)
                    }
                };
                if class_hit {
                    tally.class_hits += 1;
                }
                if canonical_hit {
                    tally.canonical_hits += 1;
                }
                if let Some(expected_kind) = expected_terminal {
                    if kind == expected_kind {
                        tally.terminal_hits += 1;
                    }
                    if expected_kind == ActionKind::Unsupported && !kind.is_terminal() {
                        tally.unsupported_recompiled += 1;
                    }
                }
                if kind.is_terminal() {
                    Outcome::Terminal(kind)
                } else {
                    Outcome::Action(validated.canonical().to_owned())
                }
            }
            Err(rejection) => {
                if rejection.code == RejectionCode::InventedEvidence {
                    tally.invented_evidence += 1;
                }
                Outcome::Rejected
            }
        };
        outcomes
            .entry(example.record_id.clone())
            .or_default()
            .push(outcome);
    }

    let mut aggregate = Tally::default();
    let per_language = tallies
        .iter()
        .map(|(language, tally)| {
            aggregate.absorb(tally);
            (language.clone(), tally.metrics())
        })
        .collect();
    let records = outcomes.len();
    let agreeing = outcomes
        .values()
        .filter(|surfaces| {
            surfaces.len() > 1
                && surfaces.iter().all(|outcome| *outcome != Outcome::Rejected)
                && surfaces.iter().all(|outcome| outcome == &surfaces[0])
        })
        .count();
    Report {
        compiler: compiler.name().to_owned(),
        per_language,
        aggregate: aggregate.metrics(),
        pair_agreement_rate: if records == 0 {
            1.0
        } else {
            agreeing as f64 / records as f64
        },
    }
}
