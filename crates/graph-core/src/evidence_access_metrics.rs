//! Evidence coverage measured against the exact links examined by a verifier.
use crate::ClaimLinkSource;
use crate::{EvidenceId, Graph, InvestigationAction, VerdictAsOf, VerificationInputs};
use serde::Serialize;
use std::collections::BTreeSet;

/// Invalid measurement inputs; no graph state has been changed.
#[derive(Debug, thiserror::Error)]
#[error("invalid evidence access measurement: {0}")]
pub struct EvidenceAccessError(pub String);

/// Actionable diagnostic, still subject to the execution policy and budget.
#[derive(Debug, Serialize)]
pub struct EvidenceAccessProposal {
    /// Existing investigation action vocabulary.
    pub action: InvestigationAction,
    /// Stable machine-readable diagnostic code.
    pub reason: &'static str,
    /// Exact evidence records motivating the proposal.
    pub evidence_ids: Vec<EvidenceId>,
}

/// Separate set-based evidence series; missing denominators are null.
#[derive(Debug, Serialize)]
pub struct EvidenceAccessReport {
    /// Stable export contract.
    pub schema_version: &'static str,
    /// Exact examined inputs supplied by the verifier instrumentation.
    pub verification_inputs: VerificationInputs,
    /// Bitemporal point used to select active explanation links.
    pub as_of: VerdictAsOf,
    /// Expected evidence present in the graph divided by expected evidence.
    pub presence_rate: Option<f64>,
    /// Expected evidence recovered through examined links divided by expected evidence.
    pub reachability_rate: Option<f64>,
    /// Retrieved evidence with no active claim explanation divided by retrieved evidence.
    pub residual_evidence_rate: Option<f64>,
    /// Number of unique annotated reference IDs.
    pub expected_count: usize,
    /// Number of unique retrieved IDs.
    pub retrieved_count: usize,
    /// Reference evidence found in the graph.
    pub present: Vec<EvidenceId>,
    /// Reference evidence recovered through the connected examined path.
    pub reachable: Vec<EvidenceId>,
    /// Reference IDs not present in the graph.
    pub absent: Vec<EvidenceId>,
    /// Present reference IDs not recovered through the verifier's examined path.
    pub present_but_unreachable: Vec<EvidenceId>,
    /// Retrieved evidence with no active claim link, including contextual and opposing links.
    pub residual_evidence: Vec<EvidenceId>,
    /// Proposals, never an implicit authorization to execute external work.
    pub proposals: Vec<EvidenceAccessProposal>,
}

/// Measure graph presence independently from a verifier's recorded input path.
///
/// Expected IDs are reference annotations and may be absent. Retrieved IDs must
/// exist. Only active examined links connected to the verified claim contribute
/// to reachability; explicitly examined observations are direct paths too.
/// Evidence must also belong to the retrieved set. Residual
/// evidence is checked against all active claim explanations at the same time.
/// Neither missing annotations nor empty retrieval silently produces a zero rate.
///
/// # Errors
/// Reject unknown claims, retrieved records, observations, or inactive/unknown
/// examined links before emitting a measurement. Never mutate the graph.
pub fn measure_evidence_access(
    graph: &Graph,
    trace: &VerificationInputs,
    at: &VerdictAsOf,
    expected: &[EvidenceId],
    retrieved: &[EvidenceId],
) -> Result<EvidenceAccessReport, EvidenceAccessError> {
    let stores = graph.epistemic_stores();
    stores
        .claims
        .claim_by_id(trace.claim_id())
        .map_err(|e| EvidenceAccessError(e.to_string()))?;
    let expected: BTreeSet<_> = expected.iter().cloned().collect();
    let retrieved: BTreeSet<_> = retrieved.iter().cloned().collect();
    for id in &retrieved {
        if graph.evidence_by_id(id).is_none() {
            return Err(EvidenceAccessError(format!(
                "unknown retrieved evidence: {}",
                id.as_str()
            )));
        }
    }
    let mut examined = Vec::new();
    for reference in trace.link_refs() {
        let matching: Vec<_> = stores
            .claims
            .claim_links()
            .iter()
            .filter(|link| link.reference_key() == *reference && link.is_active_at(at))
            .collect();
        if matching.is_empty() {
            return Err(EvidenceAccessError(format!(
                "unknown or inactive examined link: {reference}"
            )));
        }
        for link in &matching {
            let valid = match link.source() {
                ClaimLinkSource::Evidence(id) => graph.evidence_by_id(id).is_some(),
                ClaimLinkSource::Observation(id) => {
                    stores.observations.observation_by_id(id).is_some()
                }
                ClaimLinkSource::Claim(id) => stores.claims.claim_by_id(id).is_ok(),
            };
            if !valid {
                return Err(EvidenceAccessError(format!(
                    "dangling examined link: {reference}"
                )));
            }
        }
        examined.extend(matching);
    }
    for id in trace.observation_ids() {
        if stores.observations.observation_by_id(id).is_none() {
            return Err(EvidenceAccessError(format!(
                "unknown examined observation: {}",
                id.as_str()
            )));
        }
    }
    // Traverse only the recorded link subgraph, backwards from the verified
    // claim towards its evidence. A globally connected but unexamined edge
    // cannot rescue a retrieval failure. The visited set terminates cycles.
    let mut visited = BTreeSet::from([trace.claim_id().clone()]);
    loop {
        let before = visited.len();
        for link in &examined {
            if visited.contains(link.target_claim_id())
                && let ClaimLinkSource::Claim(id) = link.source()
            {
                stores
                    .claims
                    .claim_by_id(id)
                    .map_err(|e| EvidenceAccessError(e.to_string()))?;
                visited.insert(id.clone());
            }
        }
        if visited.len() == before {
            break;
        }
    }
    if examined
        .iter()
        .any(|link| !visited.contains(link.target_claim_id()))
    {
        return Err(EvidenceAccessError(
            "examined links disconnected from verified claim".into(),
        ));
    }
    let on_path = |id: &EvidenceId| {
        // Explicit observation inputs are direct recorded reads from the
        // verified claim, independent of whether a claim link was traversed.
        let directly_observed = graph
            .evidence_by_id(id)
            .and_then(|record| record.observation_id())
            .is_some_and(|observation| trace.observation_ids().contains(observation));
        directly_observed
            || examined.iter().any(|link| match link.source() {
                ClaimLinkSource::Evidence(source) => source == id,
                ClaimLinkSource::Observation(source) => {
                    graph
                        .evidence_by_id(id)
                        .and_then(|record| record.observation_id())
                        == Some(source)
                }
                ClaimLinkSource::Claim(_) => false,
            })
    };
    // Residual explanation is independent of the examined path. A direct
    // read may recover an evidence record while leaving it unexplained.
    let explained = |id: &EvidenceId| {
        stores.claims.claim_links().iter().any(|link| {
            link.is_active_at(at)
                && match link.source() {
                    ClaimLinkSource::Evidence(source) => source == id,
                    ClaimLinkSource::Observation(source) => {
                        graph
                            .evidence_by_id(id)
                            .and_then(|record| record.observation_id())
                            == Some(source)
                    }
                    ClaimLinkSource::Claim(_) => false,
                }
        })
    };
    let present: Vec<_> = expected
        .iter()
        .filter(|id| graph.evidence_by_id(id).is_some())
        .cloned()
        .collect();
    let absent = expected
        .iter()
        .filter(|id| graph.evidence_by_id(id).is_none())
        .cloned()
        .collect::<Vec<_>>();
    let reachable: Vec<_> = present
        .iter()
        .filter(|id| retrieved.contains(*id) && on_path(id))
        .cloned()
        .collect();
    let present_but_unreachable = present
        .iter()
        .filter(|id| !reachable.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let residual_evidence = retrieved
        .iter()
        .filter(|id| !explained(id))
        .cloned()
        .collect::<Vec<_>>();
    let mut proposals = Vec::new();
    for (ids, action, reason) in [
        (
            &present_but_unreachable,
            InvestigationAction::ExpandRelation,
            "present_but_unreachable",
        ),
        (
            &absent,
            InvestigationAction::SearchCorpus,
            "reference_evidence_absent",
        ),
        (
            &residual_evidence,
            InvestigationAction::SearchCorpus,
            "retrieved_evidence_unexplained",
        ),
    ] {
        if !ids.is_empty() {
            proposals.push(EvidenceAccessProposal {
                action,
                reason,
                evidence_ids: ids.clone(),
            });
        }
    }
    let ratio = |n: usize, d: usize| {
        if d == 0 {
            None
        } else {
            Some(n as f64 / d as f64)
        }
    };
    Ok(EvidenceAccessReport {
        schema_version: "corrobore-evidence-access-v1",
        verification_inputs: trace.clone(),
        as_of: at.clone(),
        presence_rate: ratio(present.len(), expected.len()),
        reachability_rate: ratio(reachable.len(), expected.len()),
        residual_evidence_rate: ratio(residual_evidence.len(), retrieved.len()),
        expected_count: expected.len(),
        retrieved_count: retrieved.len(),
        present,
        reachable,
        absent,
        present_but_unreachable,
        residual_evidence,
        proposals,
    })
}

impl EvidenceAccessProposal {
    /// Feed this diagnostic into the existing budget- and policy-aware planner.
    /// The caller supplies calibrated benefits/costs and a stable candidate ID;
    /// raw metric values are not silently treated as expected utility.
    ///
    /// # Errors
    /// Reject a blank candidate identifier.
    pub fn candidate(
        &self,
        candidate_id: impl Into<String>,
        score: crate::NextBestEvidenceScoreBreakdown,
        constraints: crate::NextBestEvidenceConstraints,
    ) -> Result<crate::NextBestEvidenceCandidateInput, crate::GraphError> {
        crate::NextBestEvidenceCandidateInput::new(candidate_id, self.action, score, constraints)
    }
}
