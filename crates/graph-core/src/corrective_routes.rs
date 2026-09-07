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
//! Corrective routes, falsifiers, and the publish gate for high-impact claims.
//!
//! Module boundary: this module records how a claim could be re-checked against
//! the world and what would change its state. It executes nothing, scores
//! nothing of its own, and decides no belief.
//!
//! Two rules give it teeth. A route on the channel that produced the claim is a
//! self-consistency recheck, not a correction, so no expected value can promote
//! it above an independent channel and a high-impact claim cannot rely on one.
//! And a falsifier must name a state that is not `Supported`: evidence that
//! would confirm a claim is corroboration, and calling it a falsifier would let
//! a claim look falsifiable while nothing could ever move it.
//!
//! Scoring and eligibility are delegated to the Next Best Evidence ranking, so
//! budget, policy and source-risk limits keep one implementation.
use crate::*;
use serde::{Deserialize, Serialize};

/// How a claim could be re-checked against the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectiveRouteKind {
    /// An authority that answers for the fact directly.
    AuthoritativeApi,
    /// The document the claim ultimately rests on.
    PrimaryDocument,
    /// An instrument that measures the world again.
    Sensor,
    /// A cryptographic check of an artifact already held.
    SignatureCheck,
    /// Retrieval through a channel unrelated to the original one.
    IndependentRetrieval,
    /// An analyst decision on the record.
    HumanReview,
}

/// Whether a route can be used now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteLiveness {
    /// Usable now.
    Live,
    /// Temporarily unusable; the route stays catalogued.
    Unavailable,
    /// Permanently withdrawn.
    Retired,
}

/// Whether a route can correct the claim or only repeat it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteIndependence {
    /// The route's channel did not produce the claim.
    Independent,
    /// The route's channel produced the claim; agreement proves nothing new.
    SelfConsistent,
}

/// How much a wrong published claim would cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimImpact {
    /// The default gate applies.
    Standard,
    /// Publication additionally requires a live independent correction path.
    High,
}

/// Immutable corrective-route registration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectiveRouteInput {
    id: String,
    kind: CorrectiveRouteKind,
    claim_type: String,
    channel: String,
    liveness: RouteLiveness,
    score_breakdown: NextBestEvidenceScoreBreakdown,
    constraints: NextBestEvidenceConstraints,
}

/// One catalogued route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CorrectiveRoute {
    input: CorrectiveRouteInput,
}

/// Append-only catalogue of routes, keyed by claim type.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredRoutes")]
pub struct CorrectiveRouteCatalogue {
    routes: Vec<CorrectiveRoute>,
}

/// What would change a claim's state, and how that evidence could be obtained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FalsifierInput {
    id: String,
    claim_id: ClaimId,
    expectation: String,
    would_change_to: VerdictState,
    route_ids: Vec<String>,
    stamp: BitemporalStamp,
}

/// One retained falsifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FalsifierRecord {
    input: FalsifierInput,
}

/// Append-only falsifier records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredFalsifiers")]
pub struct FalsifierStore {
    records: Vec<FalsifierRecord>,
}

/// One route with its independence, availability and delegated eligibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedCorrectiveRoute {
    route_id: String,
    kind: CorrectiveRouteKind,
    channel: String,
    independence: RouteIndependence,
    liveness: RouteLiveness,
    ineligibility_reasons: Vec<NextBestEvidenceIneligibilityReason>,
}

/// Routes for one claim, ordered by what is worth doing next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrectiveRouteRanking {
    routes: Vec<RankedCorrectiveRoute>,
}

/// A reason publication is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishBlocker {
    /// The WS-D actionability gate refused; its own blockers are retained.
    NotActionable,
    /// No route is catalogued for the claim type.
    CorrectiveRouteMissing,
    /// Routes exist but none is live and eligible.
    CorrectiveRouteNotLive,
    /// Only same-channel rechecks are available.
    SelfConsistentRoutesOnly,
    /// Nothing is recorded that would change the claim's state.
    FalsifierMissing,
}

/// Publication decision, with the composed actionability reasons kept visible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishDecision {
    impact: ClaimImpact,
    blockers: Vec<PublishBlocker>,
    actionability_blockers: Vec<ActionabilityBlocker>,
    selected_route: Option<String>,
    falsifiers: Vec<String>,
}

impl CorrectiveRouteKind {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 6] = [
        Self::AuthoritativeApi,
        Self::PrimaryDocument,
        Self::Sensor,
        Self::SignatureCheck,
        Self::IndependentRetrieval,
        Self::HumanReview,
    ];

    /// Canonical snake_case token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthoritativeApi => "authoritative_api",
            Self::PrimaryDocument => "primary_document",
            Self::Sensor => "sensor",
            Self::SignatureCheck => "signature_check",
            Self::IndependentRetrieval => "independent_retrieval",
            Self::HumanReview => "human_review",
        }
    }

    /// The existing investigation action a route proposes.
    ///
    /// Every route is one of the Epic 0020 actions, which is what keeps this a
    /// use of the existing ranking rather than a second mechanism beside it.
    pub fn investigation_action(self) -> InvestigationAction {
        match self {
            Self::AuthoritativeApi | Self::PrimaryDocument | Self::Sensor => {
                InvestigationAction::RequestSource
            }
            Self::SignatureCheck => InvestigationAction::VerifyClaim,
            Self::IndependentRetrieval => InvestigationAction::SearchCorpus,
            Self::HumanReview => InvestigationAction::AskAnalyst,
        }
    }
}

impl RouteLiveness {
    /// Whether the route can be used now.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl CorrectiveRouteInput {
    /// Register how one claim type could be re-checked.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for a blank identity, claim type or
    /// channel. A channel has to be a nameable path to the world: without it,
    /// independence cannot be decided.
    pub fn new(
        id: impl Into<String>,
        kind: CorrectiveRouteKind,
        claim_type: impl Into<String>,
        channel: impl Into<String>,
        liveness: RouteLiveness,
        score_breakdown: NextBestEvidenceScoreBreakdown,
        constraints: NextBestEvidenceConstraints,
    ) -> Result<Self, GraphError> {
        let input = Self {
            id: id.into(),
            kind,
            claim_type: claim_type.into(),
            channel: channel.into(),
            liveness,
            score_breakdown,
            constraints,
        };
        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<(), GraphError> {
        if [&self.id, &self.claim_type, &self.channel]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(GraphError::InvalidPropertyValue(
                "corrective route requires a nonblank identity, claim type and channel".into(),
            ));
        }
        Ok(())
    }
}

impl CorrectiveRoute {
    /// Stable route identity.
    pub fn id(&self) -> &str {
        &self.input.id
    }

    /// How the claim would be re-checked.
    pub fn kind(&self) -> CorrectiveRouteKind {
        self.input.kind
    }

    /// Claim type this route answers for.
    pub fn claim_type(&self) -> &str {
        &self.input.claim_type
    }

    /// Opaque channel identity used to decide independence.
    pub fn channel(&self) -> &str {
        &self.input.channel
    }

    /// Whether the route can be used now.
    pub fn liveness(&self) -> RouteLiveness {
        self.input.liveness
    }

    /// Delegated Next Best Evidence terms.
    pub fn score_breakdown(&self) -> NextBestEvidenceScoreBreakdown {
        self.input.score_breakdown
    }

    /// Delegated Next Best Evidence constraints.
    pub fn constraints(&self) -> NextBestEvidenceConstraints {
        self.input.constraints
    }
}

impl CorrectiveRouteCatalogue {
    /// Whether no route is catalogued.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Every catalogued route in registration order.
    pub fn routes(&self) -> &[CorrectiveRoute] {
        &self.routes
    }

    /// Routes registered for one claim type. A route never answers for a claim
    /// type it was not registered against.
    pub fn routes_for_claim_type(&self, claim_type: &str) -> Vec<&CorrectiveRoute> {
        self.routes
            .iter()
            .filter(|route| route.claim_type() == claim_type)
            .collect()
    }

    /// One route by identity.
    pub fn route_by_id(&self, id: &str) -> Option<&CorrectiveRoute> {
        self.routes.iter().find(|route| route.id() == id)
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        let mut seen = std::collections::BTreeSet::new();
        for route in &self.routes {
            route.input.validate()?;
            if !seen.insert(route.id()) {
                return Err(GraphError::InvalidPropertyValue(format!(
                    "corrective route {} is registered more than once",
                    route.id()
                )));
            }
        }
        Ok(())
    }
}

impl FalsifierInput {
    /// Record what would change a claim's state.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for a blank identity or
    /// expectation, for no cited route, or for a state of `Supported`: evidence
    /// that would confirm a claim is corroboration, and recording it here would
    /// make a claim look falsifiable while nothing could move it.
    pub fn new(
        id: impl Into<String>,
        claim_id: ClaimId,
        expectation: impl Into<String>,
        would_change_to: VerdictState,
        route_ids: Vec<String>,
        stamp: BitemporalStamp,
    ) -> Result<Self, GraphError> {
        let input = Self {
            id: id.into(),
            claim_id,
            expectation: expectation.into(),
            would_change_to,
            route_ids,
            stamp,
        };
        input.validate_structure()?;
        Ok(input)
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        if self.id.trim().is_empty()
            || self.expectation.trim().is_empty()
            || self.route_ids.is_empty()
            || self.route_ids.iter().any(|id| id.trim().is_empty())
        {
            return Err(GraphError::InvalidPropertyValue(
                "falsifier requires an identity, an expectation and at least one route".into(),
            ));
        }
        if self.would_change_to == VerdictState::Supported {
            return Err(GraphError::InvalidPropertyValue(
                "a falsifier must name a state other than supported".into(),
            ));
        }
        let mut validated = BitemporalStamp::new(
            self.stamp.valid_from.clone(),
            self.stamp.transaction_time.clone(),
        )?;
        if let Some(end) = &self.stamp.valid_to {
            validated = validated.with_valid_to(end.clone())?;
        }
        let _ = validated;
        Ok(())
    }
}

impl FalsifierRecord {
    /// Stable falsifier identity.
    pub fn id(&self) -> &str {
        &self.input.id
    }

    /// Claim this falsifier could move.
    pub fn claim_id(&self) -> &ClaimId {
        &self.input.claim_id
    }

    /// The evidence that would change the claim.
    pub fn expectation(&self) -> &str {
        &self.input.expectation
    }

    /// The state the claim would move to.
    pub fn would_change_to(&self) -> VerdictState {
        self.input.would_change_to
    }

    /// Routes that could produce that evidence.
    pub fn route_ids(&self) -> &[String] {
        &self.input.route_ids
    }

    /// When the falsifier became known.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.input.stamp
    }
}

impl FalsifierStore {
    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Every falsifier in record order.
    pub fn records(&self) -> &[FalsifierRecord] {
        &self.records
    }

    /// Falsifiers recorded for one claim.
    pub fn records_for_claim(&self, claim: &ClaimId) -> Vec<&FalsifierRecord> {
        self.records
            .iter()
            .filter(|record| record.claim_id() == claim)
            .collect()
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        let mut seen = std::collections::BTreeSet::new();
        for record in &self.records {
            record.input.validate_structure()?;
            if !seen.insert(record.id()) {
                return Err(GraphError::InvalidPropertyValue(format!(
                    "falsifier {} is recorded more than once",
                    record.id()
                )));
            }
        }
        Ok(())
    }

    /// Validate every claim and route a retained falsifier cites.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim and
    /// [`GraphError::InvalidPropertyValue`] for an unknown route.
    pub(crate) fn validate_bindings(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        self.validate_structure()?;
        for record in &self.records {
            stores.claims.claim_by_id(record.claim_id())?;
            for route in record.route_ids() {
                if stores.corrective_routes.route_by_id(route).is_none() {
                    return Err(GraphError::InvalidPropertyValue(format!(
                        "falsifier {} cites unknown corrective route {route}",
                        record.id()
                    )));
                }
            }
        }
        Ok(())
    }
}

impl RankedCorrectiveRoute {
    /// Route identity.
    pub fn route_id(&self) -> &str {
        &self.route_id
    }

    /// How the claim would be re-checked.
    pub fn kind(&self) -> CorrectiveRouteKind {
        self.kind
    }

    /// Channel behind the route.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Whether the route can correct the claim or only repeat it.
    pub fn independence(&self) -> RouteIndependence {
        self.independence
    }

    /// Whether the route can be used now.
    pub fn liveness(&self) -> RouteLiveness {
        self.liveness
    }

    /// Delegated hard reasons the route cannot be selected.
    pub fn ineligibility_reasons(&self) -> &[NextBestEvidenceIneligibilityReason] {
        &self.ineligibility_reasons
    }

    /// Whether the route is live and passes every delegated constraint.
    pub fn is_available(&self) -> bool {
        self.liveness.is_live() && self.ineligibility_reasons.is_empty()
    }
}

impl CorrectiveRouteRanking {
    /// Every route in deterministic order.
    pub fn routes(&self) -> &[RankedCorrectiveRoute] {
        &self.routes
    }

    /// The highest-ranked available route, independent whenever one exists.
    pub fn selected(&self) -> Option<&RankedCorrectiveRoute> {
        self.routes.iter().find(|route| route.is_available())
    }

    /// The highest-ranked available route on a channel that did not produce the
    /// claim. This is the one a high-impact publication may rely on.
    pub fn selected_independent(&self) -> Option<&RankedCorrectiveRoute> {
        self.routes.iter().find(|route| {
            route.is_available() && route.independence == RouteIndependence::Independent
        })
    }
}

impl PublishDecision {
    /// Whether every precondition holds.
    pub fn may_publish(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Impact class the gate applied.
    pub fn impact(&self) -> ClaimImpact {
        self.impact
    }

    /// Publication blockers in policy order.
    pub fn blockers(&self) -> &[PublishBlocker] {
        &self.blockers
    }

    /// Actionability reasons, retained from the composed WS-D decision rather
    /// than re-derived here.
    pub fn actionability_blockers(&self) -> &[ActionabilityBlocker] {
        &self.actionability_blockers
    }

    /// Route the publication would rely on for correction.
    pub fn selected_route(&self) -> Option<&str> {
        self.selected_route.as_deref()
    }

    /// Falsifiers recorded for the claim.
    pub fn falsifiers(&self) -> &[String] {
        &self.falsifiers
    }
}

/// Rank corrective routes for one claim.
///
/// Order: available routes first, then independent channels, then the
/// delegated Next Best Evidence order. The independence key is lexicographic
/// and not a weight, so no expected value can promote a same-channel recheck
/// above an independent channel.
///
/// # Errors
/// Propagates [`GraphError::InvalidNextBestEvidenceInput`] from the delegated
/// ranking for an empty or duplicated candidate set.
pub fn rank_corrective_routes(
    routes: &[&CorrectiveRoute],
    producing_channels: &[String],
) -> Result<CorrectiveRouteRanking, GraphError> {
    if routes.is_empty() {
        return Ok(CorrectiveRouteRanking { routes: Vec::new() });
    }
    let candidates = routes
        .iter()
        .map(|route| {
            NextBestEvidenceCandidateInput::new(
                route.id(),
                route.kind().investigation_action(),
                route.score_breakdown(),
                route.constraints(),
            )
        })
        .collect::<Result<Vec<_>, GraphError>>()?;
    let delegated = rank_next_best_evidence(candidates)?;
    let mut ranked = Vec::with_capacity(routes.len());
    for (position, candidate) in delegated.ranked_candidates().iter().enumerate() {
        let route = routes
            .iter()
            .find(|route| route.id() == candidate.candidate_id())
            .ok_or_else(|| {
                GraphError::InvalidNextBestEvidenceInput(
                    "ranked candidate does not resolve to a route".to_owned(),
                )
            })?;
        let independence = if producing_channels
            .iter()
            .any(|channel| channel == route.channel())
        {
            RouteIndependence::SelfConsistent
        } else {
            RouteIndependence::Independent
        };
        ranked.push((
            position,
            RankedCorrectiveRoute {
                route_id: route.id().to_owned(),
                kind: route.kind(),
                channel: route.channel().to_owned(),
                independence,
                liveness: route.liveness(),
                ineligibility_reasons: candidate.ineligibility_reasons().to_vec(),
            },
        ));
    }
    ranked.sort_by(|(left_position, left), (right_position, right)| {
        right
            .is_available()
            .cmp(&left.is_available())
            .then_with(|| left.independence.cmp(&right.independence))
            .then_with(|| left_position.cmp(right_position))
    });
    Ok(CorrectiveRouteRanking {
        routes: ranked.into_iter().map(|(_, route)| route).collect(),
    })
}

impl Ord for RouteIndependence {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl PartialOrd for RouteIndependence {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl RouteIndependence {
    fn rank(self) -> u8 {
        match self {
            Self::Independent => 0,
            Self::SelfConsistent => 1,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRoutes {
    routes: Vec<CorrectiveRoute>,
}

impl TryFrom<StoredRoutes> for CorrectiveRouteCatalogue {
    type Error = GraphError;
    fn try_from(stored: StoredRoutes) -> Result<Self, Self::Error> {
        let catalogue = Self {
            routes: stored.routes,
        };
        catalogue.validate_structure()?;
        Ok(catalogue)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFalsifiers {
    records: Vec<FalsifierRecord>,
}

impl TryFrom<StoredFalsifiers> for FalsifierStore {
    type Error = GraphError;
    fn try_from(stored: StoredFalsifiers) -> Result<Self, Self::Error> {
        let store = Self {
            records: stored.records,
        };
        store.validate_structure()?;
        Ok(store)
    }
}

impl Graph {
    /// Catalogue how one claim type could be re-checked. Idempotent by content;
    /// reusing an identity for a different route is a conflict.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for invalid input or a reused
    /// identity.
    pub fn register_corrective_route(
        &mut self,
        input: CorrectiveRouteInput,
    ) -> Result<String, GraphError> {
        input.validate()?;
        let catalogue = &mut self.epistemic_stores_mut().corrective_routes;
        if let Some(existing) = catalogue.route_by_id(&input.id) {
            if existing.input == input {
                return Ok(input.id);
            }
            return Err(GraphError::InvalidPropertyValue(format!(
                "corrective route {} is already registered with different content",
                input.id
            )));
        }
        let id = input.id.clone();
        catalogue.routes.push(CorrectiveRoute { input });
        Ok(id)
    }

    /// Record what would change a claim's state. Idempotent by content.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim,
    /// [`GraphError::InvalidPropertyValue`] for an unknown route or a reused
    /// identity.
    pub fn record_falsifier(&mut self, input: FalsifierInput) -> Result<String, GraphError> {
        input.validate_structure()?;
        let stores = self.epistemic_stores();
        stores.claims.claim_by_id(&input.claim_id)?;
        for route in &input.route_ids {
            if stores.corrective_routes.route_by_id(route).is_none() {
                return Err(GraphError::InvalidPropertyValue(format!(
                    "falsifier {} cites unknown corrective route {route}",
                    input.id
                )));
            }
        }
        let store = &mut self.epistemic_stores_mut().falsifiers;
        if let Some(existing) = store.records.iter().find(|record| record.id() == input.id) {
            if existing.input == input {
                return Ok(input.id);
            }
            return Err(GraphError::InvalidPropertyValue(format!(
                "falsifier {} is already recorded with different content",
                input.id
            )));
        }
        let id = input.id.clone();
        store.records.push(FalsifierRecord { input });
        Ok(id)
    }

    /// Channels that produced a claim, derived from the extraction lineage of
    /// its evidence. A route on one of these can only repeat the claim.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim.
    pub fn producing_channels(&self, claim: &ClaimId) -> Result<Vec<String>, GraphError> {
        let stores = self.epistemic_stores();
        let claim = stores.claims.claim_by_id(claim)?;
        let mut channels = std::collections::BTreeSet::new();
        let mut evidence_ids: Vec<EvidenceId> = claim.evidence_refs().to_vec();
        for link in stores.claims.claim_links() {
            if link.target_claim_id() == claim.id()
                && let ClaimLinkSource::Evidence(id) = link.source()
            {
                evidence_ids.push(id.clone());
            }
        }
        for record in evidence_ids.iter().filter_map(|id| self.evidence_by_id(id)) {
            if let (Some(extractor), Some(version)) =
                (record.extractor_id(), record.model_version())
            {
                channels.insert(format!("model:{extractor}@{version}"));
            }
            if let Some(run) = record.extraction_run_id() {
                channels.insert(format!("run:{}", run.as_str()));
            }
        }
        Ok(channels.into_iter().collect())
    }

    /// Rank the catalogued routes for one claim and claim type.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim; ranking errors
    /// otherwise.
    pub fn corrective_route_ranking(
        &self,
        claim: &ClaimId,
        claim_type: &str,
    ) -> Result<CorrectiveRouteRanking, GraphError> {
        let channels = self.producing_channels(claim)?;
        let routes = self
            .epistemic_stores()
            .corrective_routes
            .routes_for_claim_type(claim_type);
        rank_corrective_routes(&routes, &channels)
    }

    /// Decide whether a claim may be published.
    ///
    /// The WS-D actionability decision is composed, not repeated: this gate
    /// adds no dimension-level reason of its own and retains the actionability
    /// blockers beside its own. A high-impact claim additionally needs a live
    /// route on a channel that did not produce it, and a recorded falsifier, so
    /// a published high-impact claim is never treated as permanently closed.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim; ranking errors
    /// otherwise.
    pub fn evaluate_publish_gate(
        &self,
        claim: &ClaimId,
        impact: ClaimImpact,
        claim_type: &str,
        actionability: &ActionabilityAssessment,
    ) -> Result<PublishDecision, GraphError> {
        let ranking = self.corrective_route_ranking(claim, claim_type)?;
        let falsifiers: Vec<String> = self
            .epistemic_stores()
            .falsifiers
            .records_for_claim(claim)
            .iter()
            .map(|record| record.id().to_owned())
            .collect();
        let mut blockers = Vec::new();
        if !actionability.is_actionable() {
            blockers.push(PublishBlocker::NotActionable);
        }
        if impact == ClaimImpact::High {
            if ranking.routes().is_empty() {
                blockers.push(PublishBlocker::CorrectiveRouteMissing);
            } else if ranking.selected().is_none() {
                blockers.push(PublishBlocker::CorrectiveRouteNotLive);
            } else if ranking.selected_independent().is_none() {
                blockers.push(PublishBlocker::SelfConsistentRoutesOnly);
            }
            if falsifiers.is_empty() {
                blockers.push(PublishBlocker::FalsifierMissing);
            }
        }
        Ok(PublishDecision {
            impact,
            blockers,
            actionability_blockers: actionability.blockers().to_vec(),
            selected_route: ranking
                .selected_independent()
                .map(|route| route.route_id().to_owned()),
            falsifiers,
        })
    }
}
