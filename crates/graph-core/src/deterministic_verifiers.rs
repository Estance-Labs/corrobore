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
//! Deterministic core verifiers (Epic 0029, WS-B items 2 and 3).
//!
//! This module is deliberately domain-neutral. It checks public identifier
//! syntax, governed payload hashes, temporal and arithmetic declarations,
//! graph references, and schemas supplied through the `domain-common`
//! boundary. No verifier assigns domain meaning or adjudicates a verdict.

use std::{collections::BTreeSet, net::IpAddr};

use chrono::DateTime;
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::{
    ClaimLinkKind, ClaimLinkSource, ClaimPropositionObject, ClaimTarget, EvidenceLocator,
    GraphError, PropertyValue, SchemaConstraintEvaluation, SchemaConstraintTarget,
    VerificationOutcome, VerificationRequest, VerificationResult, Verifier, VerifierCostClass,
};

/// Stable identifier of the public identifier-syntax verifier.
pub const IDENTIFIER_SYNTAX_VERIFIER_ID: &str = "verifier.identifier-syntax";
/// First replayable version of the public identifier-syntax rules.
pub const IDENTIFIER_SYNTAX_VERIFIER_VERSION: &str = "1.0.0";
/// Stable identifier of the governed-payload SHA-256 verifier.
pub const CONTENT_HASH_VERIFIER_ID: &str = "verifier.content-hash";
/// First replayable version of the governed-payload SHA-256 rules.
pub const CONTENT_HASH_VERIFIER_VERSION: &str = "1.0.0";
/// Stable identifier of the temporal and bitemporal ordering verifier.
pub const TEMPORAL_ORDERING_VERIFIER_ID: &str = "verifier.temporal-ordering";
/// First replayable version of the temporal ordering rules.
pub const TEMPORAL_ORDERING_VERIFIER_VERSION: &str = "1.0.0";
/// Stable identifier of the numeric bounds, unit, and aggregate verifier.
pub const ARITHMETIC_CONSISTENCY_VERIFIER_ID: &str = "verifier.arithmetic-consistency";
/// First replayable version of the arithmetic consistency rules.
pub const ARITHMETIC_CONSISTENCY_VERIFIER_VERSION: &str = "1.0.0";
/// Stable identifier of the governed-record graph consistency verifier.
pub const GRAPH_CONSISTENCY_VERIFIER_ID: &str = "verifier.graph-consistency";
/// First replayable version of the graph consistency rules.
pub const GRAPH_CONSISTENCY_VERIFIER_VERSION: &str = "1.0.0";
/// Stable identifier of the domain-supplied schema constraint verifier.
pub const SCHEMA_CONSTRAINT_VERIFIER_ID: &str = "verifier.schema-constraint";
/// First replayable version of the schema constraint execution rules.
pub const SCHEMA_CONSTRAINT_VERIFIER_VERSION: &str = "1.0.0";

/// Checks every available temporal ordering carried by a verification request.
///
/// Collects proposition scopes, observation/acquisition pairs, superseding
/// claim creation times, and link bitemporal dimensions;
/// any incoherent pair will become a named deterministic failure.
#[derive(Clone, Copy, Debug, Default)]
pub struct TemporalOrderingVerifier;

impl TemporalOrderingVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for TemporalOrderingVerifier {
    fn id(&self) -> &str {
        TEMPORAL_ORDERING_VERIFIER_ID
    }

    fn version(&self) -> &str {
        TEMPORAL_ORDERING_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Medium
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let mut checked = BTreeSet::new();
        let mut failures = Vec::new();
        let mut consumed = Vec::new();

        if let Some(scope) = request
            .claim()
            .proposition()
            .and_then(|proposition| proposition.valid_time())
        {
            checked.insert("valid-time");
            if let (Some(from), Some(until)) = (scope.valid_from(), scope.valid_until()) {
                check_not_after(
                    "proposition valid-time valid_from",
                    from.as_str(),
                    "valid_until",
                    until.as_str(),
                    &mut failures,
                );
            }
        }

        for observation in request.observations() {
            if let (Some(observed_at), Some(acquired_at)) = (
                observation.observed_at(),
                request
                    .source_of(observation)
                    .and_then(|source| source.acquired_at()),
            ) {
                checked.insert("acquisition");
                consumed.push(format!("observation:{}", observation.id().as_str()));
                check_not_after(
                    &format!(
                        "source acquisition for observation {}",
                        observation.id().as_str()
                    ),
                    acquired_at.as_str(),
                    "observed_at",
                    observed_at.as_str(),
                    &mut failures,
                );
            }
        }

        for link in request.links() {
            if let Some(stamp) = link.bitemporal() {
                checked.insert("bitemporal");
                consumed.push(format!("link:{}", link.reference_key()));
                if let Some(valid_to) = stamp.valid_to.as_ref() {
                    check_strictly_before(
                        "bitemporal valid_from",
                        stamp.valid_from.as_str(),
                        "valid_to",
                        valid_to.as_str(),
                        &mut failures,
                    );
                }
                if let (Some(observation_time), Some(publication_time)) = (
                    stamp.observation_time.as_ref(),
                    stamp.publication_time.as_ref(),
                ) {
                    check_not_after(
                        "bitemporal observation_time",
                        observation_time.as_str(),
                        "publication_time",
                        publication_time.as_str(),
                        &mut failures,
                    );
                }
                if let Some(observation_time) = stamp.observation_time.as_ref() {
                    check_not_after(
                        "bitemporal observation_time",
                        observation_time.as_str(),
                        "transaction_time",
                        stamp.transaction_time.as_str(),
                        &mut failures,
                    );
                }
                if let Some(publication_time) = stamp.publication_time.as_ref() {
                    check_not_after(
                        "bitemporal publication_time",
                        publication_time.as_str(),
                        "transaction_time",
                        stamp.transaction_time.as_str(),
                        &mut failures,
                    );
                }
            }

            if link.kind() == ClaimLinkKind::Supersedes
                && let ClaimLinkSource::Claim(superseding_id) = link.source()
                && let Some(superseding) = request.claim_by_id(superseding_id)
                && let (
                    Some((superseding_field, superseding_time)),
                    Some((older_field, older_time)),
                ) = (
                    claim_ordering_time(superseding),
                    claim_ordering_time(request.claim()),
                )
            {
                checked.insert("supersession");
                consumed.push(format!("link:{}", link.reference_key()));
                check_not_after(
                    &format!(
                        "superseded claim {} {older_field}",
                        request.claim().id().as_str()
                    ),
                    older_time,
                    &format!(
                        "superseding claim {} {superseding_field}",
                        superseding_id.as_str()
                    ),
                    superseding_time,
                    &mut failures,
                );
            }
        }

        Ok(consistency_outcome(
            checked,
            failures,
            consumed,
            "no proposition scope, observation/acquisition pair, supersession clock, or bitemporal stamp was available",
            TEMPORAL_ORDERING_LIMIT,
        ))
    }
}

/// Checks typed numeric bounds, units, and aggregate-part equality.
///
/// Inspects only numeric proposition literals carrying an
/// explicit arithmetic declaration, keeping ordinary prose and unannotated
/// numbers inconclusive.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArithmeticConsistencyVerifier;

impl ArithmeticConsistencyVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for ArithmeticConsistencyVerifier {
    fn id(&self) -> &str {
        ARITHMETIC_CONSISTENCY_VERIFIER_ID
    }

    fn version(&self) -> &str {
        ARITHMETIC_CONSISTENCY_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Low
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let Some(proposition) = request.claim().proposition() else {
            return Ok(inconclusive_outcome(
                "no arithmetic declaration was available on the claim proposition",
                ARITHMETIC_CONSISTENCY_LIMIT,
            ));
        };
        let Some(arithmetic) = proposition.arithmetic_constraint() else {
            return Ok(inconclusive_outcome(
                "no arithmetic declaration was available on the claim proposition",
                ARITHMETIC_CONSISTENCY_LIMIT,
            ));
        };
        if arithmetic.is_empty() {
            return Ok(inconclusive_outcome(
                "the arithmetic declaration carried no bound, unit, or aggregate part to check",
                ARITHMETIC_CONSISTENCY_LIMIT,
            ));
        }

        let mut checked = BTreeSet::new();
        let mut failures = Vec::new();
        let value = match proposition.object() {
            ClaimPropositionObject::Literal(PropertyValue::Integer(value)) => Some(*value as f64),
            ClaimPropositionObject::Literal(PropertyValue::Float(value)) => Some(*value),
            _ => None,
        };
        let Some(value) = value else {
            return Ok(VerificationOutcome::new(VerificationResult::Fail)
                .with_rationale("arithmetic metadata requires a numeric proposition literal")
                .with_limit(ARITHMETIC_CONSISTENCY_LIMIT)
                .with_evidence_consumed(format!(
                    "claim:{}:proposition",
                    request.claim().id().as_str()
                )));
        };

        if !value.is_finite() {
            failures.push(format!("proposition value {value} is not finite"));
        }
        if arithmetic.minimum().is_some() || arithmetic.maximum().is_some() {
            checked.insert("bounds");
        }
        if let (Some(minimum), Some(maximum)) = (arithmetic.minimum(), arithmetic.maximum())
            && minimum > maximum
        {
            failures.push(format!(
                "declared minimum {minimum} exceeds declared maximum {maximum}"
            ));
        }
        if let Some(minimum) = arithmetic.minimum() {
            if !minimum.is_finite() {
                failures.push(format!("declared minimum {minimum} is not finite"));
            } else if value < minimum {
                failures.push(format!(
                    "proposition value {value} is below declared minimum {minimum}"
                ));
            }
        }
        if let Some(maximum) = arithmetic.maximum() {
            if !maximum.is_finite() {
                failures.push(format!("declared maximum {maximum} is not finite"));
            } else if value > maximum {
                failures.push(format!(
                    "proposition value {value} exceeds declared maximum {maximum}"
                ));
            }
        }

        let aggregate_unit = arithmetic.unit();
        if aggregate_unit.is_some() || arithmetic.parts().iter().any(|part| part.unit().is_some()) {
            checked.insert("unit");
        }
        if aggregate_unit.is_some_and(|unit| unit.trim().is_empty()) {
            failures.push("aggregate unit is blank".to_owned());
        }
        for (index, part) in arithmetic.parts().iter().enumerate() {
            if !part.value().is_finite() {
                failures.push(format!("aggregate part {} is not finite", index + 1));
            }
            match (aggregate_unit, part.unit()) {
                (Some(expected), Some(actual)) if actual.trim().is_empty() => {
                    failures.push(format!(
                        "aggregate part {} unit is blank; expected '{expected}'",
                        index + 1
                    ))
                }
                (Some(expected), Some(actual)) if actual != expected => failures.push(format!(
                    "aggregate part {} unit '{actual}' differs from aggregate unit '{expected}'",
                    index + 1
                )),
                (Some(expected), None) => failures.push(format!(
                    "aggregate part {} has no unit; expected '{expected}'",
                    index + 1
                )),
                _ => {}
            }
        }
        if aggregate_unit.is_none() {
            let mut part_units = BTreeSet::new();
            for (index, part) in arithmetic.parts().iter().enumerate() {
                if let Some(unit) = part.unit() {
                    if unit.trim().is_empty() {
                        failures.push(format!("aggregate part {} unit is blank", index + 1));
                    } else {
                        part_units.insert(unit);
                    }
                }
            }
            if part_units.len() > 1 {
                failures.push(format!(
                    "aggregate parts declare inconsistent units: {}",
                    part_units.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }

        if !arithmetic.parts().is_empty() {
            checked.insert("aggregate");
            let sum = arithmetic
                .parts()
                .iter()
                .map(|part| part.value())
                .sum::<f64>();
            if !approximately_equal(sum, value) {
                failures.push(format!(
                    "sum of parts {sum} does not equal proposition value {value}"
                ));
            }
        }

        Ok(consistency_outcome(
            checked,
            failures,
            vec![format!(
                "claim:{}:proposition",
                request.claim().id().as_str()
            )],
            "the arithmetic declaration carried no bound, unit, or aggregate part to check",
            ARITHMETIC_CONSISTENCY_LIMIT,
        ))
    }
}

/// Checks graph targets, link sources, and superseded observation links.
///
/// Resolves every target and source against the immutable
/// stores on the request and name the exact missing or superseded record.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphConsistencyVerifier;

impl GraphConsistencyVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for GraphConsistencyVerifier {
    fn id(&self) -> &str {
        GRAPH_CONSISTENCY_VERIFIER_ID
    }

    fn version(&self) -> &str {
        GRAPH_CONSISTENCY_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Medium
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let mut checked = BTreeSet::new();
        let mut failures = Vec::new();
        let mut consumed = Vec::new();

        if let Some(graph) = request.graph() {
            match request.claim().target() {
                ClaimTarget::Node(node_id) => {
                    checked.insert("claim target");
                    consumed.push(format!("node:{}", node_id.as_str()));
                    if graph.get_node(node_id)?.is_none() {
                        failures.push(format!(
                            "claim target node {} is dangling",
                            node_id.as_str()
                        ));
                    }
                }
                ClaimTarget::Relationship(relationship_id) => {
                    checked.insert("claim target");
                    consumed.push(format!("relationship:{}", relationship_id.as_str()));
                    if graph.get_relationship(relationship_id)?.is_none() {
                        failures.push(format!(
                            "claim target relationship {} is dangling",
                            relationship_id.as_str()
                        ));
                    }
                }
                _ => {}
            }
            if let Some(ClaimPropositionObject::Entity(node_id)) = request
                .claim()
                .proposition()
                .map(|proposition| proposition.object())
            {
                checked.insert("proposition entity");
                consumed.push(format!("node:{}", node_id.as_str()));
                if graph.get_node(node_id)?.is_none() {
                    failures.push(format!(
                        "proposition entity node {} is dangling",
                        node_id.as_str()
                    ));
                }
            }
        }

        for link in request.links() {
            checked.insert("evidence link");
            consumed.push(format!("link:{}", link.reference_key()));
            match link.source() {
                ClaimLinkSource::Evidence(evidence_id) => {
                    if request.evidence_by_id(evidence_id).is_none() {
                        failures.push(format!(
                            "evidence link is dangling: missing record {}",
                            evidence_id.as_str()
                        ));
                    }
                }
                ClaimLinkSource::Observation(observation_id) => {
                    if request.observation_by_id(observation_id).is_none() {
                        failures.push(format!(
                            "observation link is dangling: missing record {}",
                            observation_id.as_str()
                        ));
                    } else if let Some(replacement) =
                        request.observation_superseded_by(observation_id)
                    {
                        failures.push(format!(
                            "observation link is dangling after supersession: {} was replaced by {}",
                            observation_id.as_str(),
                            replacement.as_str()
                        ));
                    }
                }
                ClaimLinkSource::Claim(claim_id) => {
                    if request.claim_by_id(claim_id).is_none() {
                        failures.push(format!(
                            "claim link is dangling: missing record {}",
                            claim_id.as_str()
                        ));
                    }
                }
            }
        }

        Ok(consistency_outcome(
            checked,
            failures,
            consumed,
            "no node or relationship target, proposition entity, or evidence link was available to check",
            GRAPH_CONSISTENCY_LIMIT,
        ))
    }
}

/// Runs the schema supplied by an installed domain pack against the exact
/// immutable graph record named by the claim.
///
/// Preserves absence as `Inconclusive`, maps an applicable
/// provider result to pass/fail, and never interpret domain vocabulary in core.
#[derive(Clone, Copy, Debug, Default)]
pub struct SchemaConstraintVerifier;

impl SchemaConstraintVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for SchemaConstraintVerifier {
    fn id(&self) -> &str {
        SCHEMA_CONSTRAINT_VERIFIER_ID
    }

    fn version(&self) -> &str {
        SCHEMA_CONSTRAINT_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Medium
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let Some(provider) = request.schema_constraints() else {
            return Ok(inconclusive_outcome(
                "no schema provider is installed",
                SCHEMA_CONSTRAINT_LIMIT,
            ));
        };
        let Some(graph) = request.graph() else {
            return Ok(inconclusive_outcome(
                "no graph snapshot was supplied for schema verification",
                SCHEMA_CONSTRAINT_LIMIT,
            ));
        };

        let (evaluation, reference) = match request.claim().target() {
            ClaimTarget::Node(node_id) => {
                let Some(node) = graph.get_node(node_id)? else {
                    return Ok(inconclusive_outcome(
                        &format!("schema target node {} does not resolve", node_id.as_str()),
                        SCHEMA_CONSTRAINT_LIMIT,
                    ));
                };
                (
                    provider.evaluate(SchemaConstraintTarget::Node(&node)),
                    format!("node:{}", node_id.as_str()),
                )
            }
            ClaimTarget::Relationship(relationship_id) => {
                let Some(relationship) = graph.get_relationship(relationship_id)? else {
                    return Ok(inconclusive_outcome(
                        &format!(
                            "schema target relationship {} does not resolve",
                            relationship_id.as_str()
                        ),
                        SCHEMA_CONSTRAINT_LIMIT,
                    ));
                };
                (
                    provider.evaluate(SchemaConstraintTarget::Relationship(&relationship)),
                    format!("relationship:{}", relationship_id.as_str()),
                )
            }
            _ => {
                return Ok(inconclusive_outcome(
                    "the claim target does not name a node or relationship for schema verification",
                    SCHEMA_CONSTRAINT_LIMIT,
                ));
            }
        };

        let outcome = match evaluation {
            SchemaConstraintEvaluation::NotApplicable => inconclusive_outcome(
                "no installed schema applies to the target record",
                SCHEMA_CONSTRAINT_LIMIT,
            ),
            SchemaConstraintEvaluation::Pass { rule } => {
                VerificationOutcome::new(VerificationResult::Pass)
                    .with_rationale(format!("target record satisfies schema rule {rule}"))
                    .with_limit(SCHEMA_CONSTRAINT_LIMIT)
                    .with_evidence_consumed(reference)
            }
            SchemaConstraintEvaluation::Fail { rule, violations } => {
                VerificationOutcome::new(VerificationResult::Fail)
                    .with_rationale(format!(
                        "schema rule {rule} failed: {}",
                        if violations.is_empty() {
                            "provider returned no violation detail".to_owned()
                        } else {
                            violations.join("; ")
                        }
                    ))
                    .with_limit(SCHEMA_CONSTRAINT_LIMIT)
                    .with_evidence_consumed(reference)
            }
        };
        Ok(outcome)
    }
}

const TEMPORAL_ORDERING_LIMIT: &str = "ordering only; timestamps do not establish that an event occurred or that a source reported it accurately";
const ARITHMETIC_CONSISTENCY_LIMIT: &str = "declared arithmetic only; the verifier does not infer units, bounds, parts, or semantic meaning";
const GRAPH_CONSISTENCY_LIMIT: &str = "reference integrity only; a resolved graph record or current link does not establish the claim's truth";
const SCHEMA_CONSTRAINT_LIMIT: &str = "installed schema only; absence or non-applicability is inconclusive and domain semantics remain pack-owned";

fn inconclusive_outcome(rationale: &str, limit: &str) -> VerificationOutcome {
    VerificationOutcome::new(VerificationResult::Inconclusive)
        .with_rationale(rationale)
        .with_limit(limit)
}

fn consistency_outcome(
    checked: BTreeSet<&str>,
    failures: Vec<String>,
    consumed: Vec<String>,
    empty_rationale: &str,
    limit: &str,
) -> VerificationOutcome {
    let (result, rationale) = if checked.is_empty() {
        (VerificationResult::Inconclusive, empty_rationale.to_owned())
    } else if failures.is_empty() {
        (
            VerificationResult::Pass,
            format!(
                "validated {} consistency check(s): {}",
                checked.len(),
                checked.into_iter().collect::<Vec<_>>().join(", ")
            ),
        )
    } else {
        (VerificationResult::Fail, failures.join("; "))
    };

    let mut outcome = VerificationOutcome::new(result)
        .with_rationale(rationale)
        .with_limit(limit);
    let mut unique = BTreeSet::new();
    for reference in consumed {
        if unique.insert(reference.clone()) {
            outcome = outcome.with_evidence_consumed(reference);
        }
    }
    outcome
}

fn parse_timestamp(
    label: &str,
    value: &str,
    failures: &mut Vec<String>,
) -> Option<DateTime<chrono::FixedOffset>> {
    match DateTime::parse_from_rfc3339(value) {
        Ok(timestamp) => Some(timestamp),
        Err(_) => {
            failures.push(format!("{label} is not a valid RFC3339 timestamp: {value}"));
            None
        }
    }
}

fn check_not_after(
    earlier_label: &str,
    earlier: &str,
    later_label: &str,
    later: &str,
    failures: &mut Vec<String>,
) {
    let parsed_earlier = parse_timestamp(earlier_label, earlier, failures);
    let parsed_later = parse_timestamp(later_label, later, failures);
    if let (Some(parsed_earlier), Some(parsed_later)) = (parsed_earlier, parsed_later)
        && parsed_earlier > parsed_later
    {
        failures.push(format!(
            "{earlier_label} {earlier} is after {later_label} {later}"
        ));
    }
}

fn check_strictly_before(
    earlier_label: &str,
    earlier: &str,
    later_label: &str,
    later: &str,
    failures: &mut Vec<String>,
) {
    let before = failures.len();
    check_not_after(earlier_label, earlier, later_label, later, failures);
    if failures.len() == before
        && DateTime::parse_from_rfc3339(earlier).ok() == DateTime::parse_from_rfc3339(later).ok()
    {
        failures.push(format!(
            "{earlier_label} {earlier} must strictly precede {later_label} {later}"
        ));
    }
}

fn claim_ordering_time(claim: &crate::Claim) -> Option<(&'static str, &str)> {
    let temporal = claim.temporal();
    temporal
        .created_at
        .as_deref()
        .map(|value| ("created_at", value))
        .or_else(|| {
            temporal
                .recorded_at
                .as_deref()
                .map(|value| ("recorded_at", value))
        })
}

fn approximately_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= f64::EPSILON * scale * 8.0
}

/// Validates identifier-shaped proposition literals and selected observation
/// payloads against public formats only.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdentifierSyntaxVerifier;

impl IdentifierSyntaxVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for IdentifierSyntaxVerifier {
    fn id(&self) -> &str {
        IDENTIFIER_SYNTAX_VERIFIER_ID
    }

    fn version(&self) -> &str {
        IDENTIFIER_SYNTAX_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Low
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let candidates = identifier_candidates(request);
        let mut failures = Vec::new();
        let mut formats = BTreeSet::new();

        for candidate in &candidates {
            formats.insert(candidate.kind.name());
            if let Err(reason) = candidate.kind.validate(candidate.value.as_str()) {
                failures.push(format!("{}: {reason}", candidate.location));
            }
        }

        let (result, rationale) = if candidates.is_empty() {
            (
                VerificationResult::Inconclusive,
                "no identifier-shaped proposition object or observation selector was available"
                    .to_owned(),
            )
        } else if failures.is_empty() {
            (
                VerificationResult::Pass,
                format!(
                    "validated {} identifier-shaped value(s) as {}",
                    candidates.len(),
                    formats.into_iter().collect::<Vec<_>>().join(", ")
                ),
            )
        } else {
            (VerificationResult::Fail, failures.join("; "))
        };

        let mut outcome = VerificationOutcome::new(result)
            .with_rationale(rationale)
            .with_limit(IDENTIFIER_SYNTAX_LIMIT);
        let mut recorded_references = Vec::new();
        for reference in candidates
            .iter()
            .map(|candidate| candidate.evidence_ref.as_str())
        {
            if !recorded_references.contains(&reference) {
                outcome = outcome.with_evidence_consumed(reference);
                recorded_references.push(reference);
            }
        }
        Ok(outcome)
    }
}

/// Recomputes SHA-256 over observation and evidence payload bytes and compares
/// it with each digest recorded on the governed input.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContentHashVerifier;

impl ContentHashVerifier {
    /// Creates the stateless verifier.
    pub const fn new() -> Self {
        Self
    }
}

impl Verifier for ContentHashVerifier {
    fn id(&self) -> &str {
        CONTENT_HASH_VERIFIER_ID
    }

    fn version(&self) -> &str {
        CONTENT_HASH_VERIFIER_VERSION
    }

    fn deterministic(&self) -> bool {
        true
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Low
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let mut comparisons = Vec::new();
        for observation in request.observations() {
            if let Some(recorded) = observation.payload_sha256() {
                comparisons.push(HashComparison::new(
                    format!("observation:{}", observation.id().as_str()),
                    recorded,
                    observation.payload(),
                ));
            }
        }
        for evidence in request.evidence_records() {
            if let Some(recorded) = evidence.content_sha256() {
                comparisons.push(HashComparison::new(
                    format!("evidence:{}", evidence.id().as_str()),
                    recorded,
                    evidence.payload(),
                ));
            }
        }

        let drift: Vec<String> = comparisons
            .iter()
            .filter(|comparison| !comparison.matches())
            .map(HashComparison::drift_message)
            .collect();
        let (result, rationale) = if comparisons.is_empty() {
            (
                VerificationResult::Inconclusive,
                "no recorded digest was available on the governed payloads".to_owned(),
            )
        } else if drift.is_empty() {
            (
                VerificationResult::Pass,
                format!(
                    "recorded SHA-256 matched the exact UTF-8 payload bytes for {} record(s)",
                    comparisons.len()
                ),
            )
        } else {
            (
                VerificationResult::Fail,
                format!("content hash drift: {}", drift.join("; ")),
            )
        };

        let mut outcome = VerificationOutcome::new(result)
            .with_rationale(rationale)
            .with_limit(CONTENT_HASH_LIMIT);
        for comparison in &comparisons {
            outcome = outcome.with_evidence_consumed(comparison.reference.as_str());
        }
        Ok(outcome)
    }
}

const IDENTIFIER_SYNTAX_LIMIT: &str = "syntax only; existence, ownership, currency, and meaning in any external registry are not checked";
const CONTENT_HASH_LIMIT: &str = "a matching digest establishes payload integrity against the recorded value, not source authenticity or truth";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IdentifierKind {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
    Digest,
    Uuid,
    Rfc3339,
    Domain,
    Ipv4,
    Ipv6,
    Ip,
    Url,
    StixId,
    CveId,
}

impl IdentifierKind {
    fn from_hint(hint: &str) -> Option<Self> {
        let tokens: Vec<String> = hint
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        let last = tokens.last()?.as_str();
        let last_pair = (tokens.len() >= 2)
            .then(|| format!("{}{}", tokens[tokens.len() - 2], tokens[tokens.len() - 1]));
        let terminal = last_pair.as_deref().unwrap_or(last);
        match (last, terminal) {
            ("sha256", _) | (_, "sha256") => Some(Self::Sha256),
            ("sha224", _) | (_, "sha224") => Some(Self::Sha224),
            ("sha384", _) | (_, "sha384") => Some(Self::Sha384),
            ("sha512", _) | (_, "sha512") => Some(Self::Sha512),
            ("sha1", _) | (_, "sha1") => Some(Self::Sha1),
            ("md5", _) => Some(Self::Md5),
            ("rfc3339", _) | (_, "rfc3339") | ("timestamp", _) | ("datetime", _) => {
                Some(Self::Rfc3339)
            }
            ("domain", _) | ("hostname", _) | (_, "domainname") => Some(Self::Domain),
            ("ipv4", _) | (_, "ipv4") => Some(Self::Ipv4),
            ("ipv6", _) | (_, "ipv6") => Some(Self::Ipv6),
            ("ip", _) | (_, "ipaddress") => Some(Self::Ip),
            ("stix", _) | (_, "stixid") => Some(Self::StixId),
            ("cve", _) | (_, "cveid") => Some(Self::CveId),
            ("digest" | "checksum" | "hash", _) => Some(Self::Digest),
            ("uuid", _) => Some(Self::Uuid),
            ("url" | "uri", _) => Some(Self::Url),
            _ => None,
        }
    }

    fn infer(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.starts_with("CVE-") {
            return Some(Self::CveId);
        }
        if value.contains("--")
            && value
                .rsplit_once("--")
                .is_some_and(|(_, suffix)| looks_uuid_shaped(suffix))
        {
            return Some(Self::StixId);
        }
        if value.contains("://") {
            return Some(Self::Url);
        }
        if value.parse::<IpAddr>().is_ok() {
            return Some(Self::Ip);
        }
        if looks_uuid_shaped(value) {
            return Some(Self::Uuid);
        }
        if looks_rfc3339_shaped(value) {
            return Some(Self::Rfc3339);
        }
        if is_hex(value) {
            return match value.len() {
                32 => Some(Self::Md5),
                40 => Some(Self::Sha1),
                56 => Some(Self::Sha224),
                64 => Some(Self::Sha256),
                96 => Some(Self::Sha384),
                128 => Some(Self::Sha512),
                _ => None,
            };
        }
        None
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha224 => "sha224",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
            Self::Digest => "digest",
            Self::Uuid => "uuid",
            Self::Rfc3339 => "rfc3339",
            Self::Domain => "domain",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
            Self::Ip => "ip",
            Self::Url => "url",
            Self::StixId => "stix_id",
            Self::CveId => "cve_id",
        }
    }

    fn validate(self, raw: &str) -> Result<(), String> {
        let value = raw.trim();
        let valid = match self {
            Self::Md5 => valid_hex_digest(value, 32),
            Self::Sha1 => valid_hex_digest(value, 40),
            Self::Sha224 => valid_hex_digest(value, 56),
            Self::Sha256 => valid_hex_digest(value, 64),
            Self::Sha384 => valid_hex_digest(value, 96),
            Self::Sha512 => valid_hex_digest(value, 128),
            Self::Digest => [32, 40, 56, 64, 96, 128].contains(&value.len()) && is_hex(value),
            Self::Uuid => valid_uuid(value),
            Self::Rfc3339 => DateTime::parse_from_rfc3339(value).is_ok(),
            Self::Domain => valid_domain(value),
            Self::Ipv4 => value.parse::<std::net::Ipv4Addr>().is_ok(),
            Self::Ipv6 => value.parse::<std::net::Ipv6Addr>().is_ok(),
            Self::Ip => value.parse::<IpAddr>().is_ok(),
            Self::Url => valid_url(value),
            Self::StixId => {
                return validate_stix_id(value)
                    .map_err(|reason| format!("invalid stix_id: {reason}"));
            }
            Self::CveId => valid_cve_id(value),
        };

        valid.then_some(()).ok_or_else(|| {
            format!(
                "invalid {} value '{}' according to its public syntax",
                self.name(),
                value
            )
        })
    }
}

#[derive(Debug)]
struct IdentifierCandidate {
    kind: IdentifierKind,
    value: String,
    location: String,
    evidence_ref: String,
}

fn identifier_candidates(request: &VerificationRequest<'_>) -> Vec<IdentifierCandidate> {
    let mut candidates = Vec::new();
    if let Some(proposition) = request.claim().proposition() {
        collect_property_candidates(
            proposition.object(),
            IdentifierKind::from_hint(proposition.predicate()),
            "claim proposition object",
            &format!("claim:{}:proposition-object", request.claim().id().as_str()),
            &mut candidates,
        );
    }

    for observation in request.observations() {
        let Some(selector) = observation.selector() else {
            continue;
        };
        let EvidenceLocator::RecordPath { path } = selector else {
            continue;
        };
        let value = selected_payload(observation.payload(), path);
        let Some(kind) = IdentifierKind::from_hint(path).or_else(|| IdentifierKind::infer(&value))
        else {
            continue;
        };
        candidates.push(IdentifierCandidate {
            kind,
            value,
            location: format!(
                "observation {} selector {}",
                observation.id().as_str(),
                path
            ),
            evidence_ref: format!(
                "observation:{}:selector:{}",
                observation.id().as_str(),
                selector.render()
            ),
        });
    }
    candidates
}

fn collect_property_candidates(
    object: &ClaimPropositionObject,
    hinted_kind: Option<IdentifierKind>,
    location: &str,
    evidence_ref: &str,
    candidates: &mut Vec<IdentifierCandidate>,
) {
    let ClaimPropositionObject::Literal(value) = object else {
        return;
    };
    match value {
        PropertyValue::String(value) => {
            push_candidate(value, hinted_kind, location, evidence_ref, candidates)
        }
        PropertyValue::StringList(values) => {
            for (index, value) in values.iter().enumerate() {
                push_candidate(
                    value,
                    hinted_kind,
                    &format!("{location}[{index}]"),
                    evidence_ref,
                    candidates,
                );
            }
        }
        PropertyValue::Json(value) => {
            collect_json_candidates(value, hinted_kind, location, evidence_ref, candidates);
        }
        PropertyValue::Null
        | PropertyValue::Bool(_)
        | PropertyValue::Integer(_)
        | PropertyValue::Float(_)
        | PropertyValue::IntegerList(_)
        | PropertyValue::FloatList(_)
        | PropertyValue::BoolList(_) => {}
    }
}

fn collect_json_candidates(
    value: &serde_json::Value,
    hinted_kind: Option<IdentifierKind>,
    location: &str,
    evidence_ref: &str,
    candidates: &mut Vec<IdentifierCandidate>,
) {
    match value {
        serde_json::Value::String(value) => {
            push_candidate(value, hinted_kind, location, evidence_ref, candidates)
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_json_candidates(
                    value,
                    hinted_kind,
                    &format!("{location}[{index}]"),
                    evidence_ref,
                    candidates,
                );
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                collect_json_candidates(
                    value,
                    IdentifierKind::from_hint(key).or(hinted_kind),
                    &format!("{location}.{key}"),
                    evidence_ref,
                    candidates,
                );
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn push_candidate(
    value: &str,
    hinted_kind: Option<IdentifierKind>,
    location: &str,
    evidence_ref: &str,
    candidates: &mut Vec<IdentifierCandidate>,
) {
    if let Some(kind) = hinted_kind.or_else(|| IdentifierKind::infer(value)) {
        candidates.push(IdentifierCandidate {
            kind,
            value: value.to_owned(),
            location: location.to_owned(),
            evidence_ref: evidence_ref.to_owned(),
        });
    }
}

fn selected_payload(payload: &str, path: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|document| document.pointer(path).cloned())
        .and_then(|selected| match selected {
            serde_json::Value::String(value) => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| payload.to_owned())
}

fn valid_hex_digest(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length && is_hex(value)
}

fn is_hex(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn looks_uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| value.as_bytes().get(index) == Some(&b'-'))
}

fn valid_uuid(value: &str) -> bool {
    looks_uuid_shaped(value) && Uuid::parse_str(value).is_ok()
}

fn looks_rfc3339_shaped(value: &str) -> bool {
    value.len() >= 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value
            .as_bytes()
            .get(10)
            .is_some_and(|byte| *byte == b'T' || *byte == b't')
}

fn valid_domain(value: &str) -> bool {
    if !value.is_ascii() || value.is_empty() || value.len() > 254 {
        return false;
    }
    let without_root_dot = value.strip_suffix('.').unwrap_or(value);
    if without_root_dot.is_empty() || without_root_dot.len() > 253 {
        return false;
    }
    without_root_dot.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    })
}

fn valid_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        !url.scheme().is_empty()
            && match url.scheme() {
                "http" | "https" | "ftp" => url.host_str().is_some(),
                _ => true,
            }
    })
}

fn valid_cve_id(value: &str) -> bool {
    let mut parts = value.split('-');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("CVE"), Some(year), Some(sequence), None)
            if year.len() == 4
                && year.bytes().all(|byte| byte.is_ascii_digit())
                && sequence.len() >= 4
                && sequence.bytes().all(|byte| byte.is_ascii_digit())
    )
}

fn validate_stix_id(value: &str) -> Result<(), String> {
    let Some((object_type, identifier)) = value.split_once("--") else {
        return Err(format!(
            "malformed STIX identifier '{value}' (expected <type>--<uuid>)"
        ));
    };
    let well_formed_type = !object_type.is_empty()
        && object_type.len() <= 250
        && object_type
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && object_type.as_bytes().first() != Some(&b'-')
        && object_type.as_bytes().last() != Some(&b'-');
    if !well_formed_type || !valid_uuid(identifier) || identifier.contains("--") {
        return Err(format!(
            "malformed STIX identifier '{value}' (expected <type>--<uuid>)"
        ));
    }
    if !STIX_OBJECT_TYPES.contains(&object_type) {
        return Err(format!(
            "STIX identifier '{value}' has a valid shape but unknown STIX object type '{object_type}'"
        ));
    }
    Ok(())
}

const STIX_OBJECT_TYPES: &[&str] = &[
    "artifact",
    "attack-pattern",
    "autonomous-system",
    "bundle",
    "campaign",
    "course-of-action",
    "directory",
    "domain-name",
    "email-addr",
    "email-message",
    "extension-definition",
    "file",
    "grouping",
    "identity",
    "incident",
    "indicator",
    "infrastructure",
    "intrusion-set",
    "ipv4-addr",
    "ipv6-addr",
    "language-content",
    "location",
    "mac-addr",
    "malware",
    "malware-analysis",
    "marking-definition",
    "mutex",
    "network-traffic",
    "note",
    "observed-data",
    "opinion",
    "process",
    "relationship",
    "report",
    "sighting",
    "software",
    "threat-actor",
    "tool",
    "url",
    "user-account",
    "vulnerability",
    "windows-registry-key",
    "x509-certificate",
];

#[derive(Debug)]
struct HashComparison {
    reference: String,
    recorded: String,
    computed: String,
}

impl HashComparison {
    fn new(reference: String, recorded: &str, payload: &str) -> Self {
        Self {
            reference,
            recorded: recorded.to_owned(),
            computed: format!("{:x}", Sha256::digest(payload.as_bytes())),
        }
    }

    fn matches(&self) -> bool {
        self.recorded == self.computed
    }

    fn drift_message(&self) -> String {
        format!(
            "{} recorded sha256 {}, computed sha256 {}",
            self.reference, self.recorded, self.computed
        )
    }
}
