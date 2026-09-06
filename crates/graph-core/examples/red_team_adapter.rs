//! Gold-free, versioned synthetic workload adapter using real graph-core APIs.
use graph_core::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};
type Error = Box<dyn std::error::Error>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema_version: String,
    mode: Mode,
    as_of: String,
    authority_policy: Authority,
    documents: Vec<Document>,
    queries: Vec<Query>,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    Defended,
    Unprotected,
    QuarantineAll,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Authority {
    version: String,
    trusted_source_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assertion {
    subject: String,
    predicate: String,
    value: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    id: String,
    subject: String,
    predicate: String,
    value: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    id: String,
    source_id: String,
    body: String,
    assertion: Assertion,
    valid_from: String,
    valid_until: Option<String>,
    #[serde(default)]
    risk_features: Option<Features>,
    #[serde(default, rename = "claimedAuthority")]
    _claimed_authority: Option<String>,
    #[serde(default, rename = "contextFacts")]
    _context_facts: Vec<Assertion>,
    #[serde(default, rename = "supersededBy")]
    _superseded_by: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Features {
    attribution: String,
    #[serde(default)]
    embedding: Option<Vec<f64>>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    infrastructure: Vec<String>,
    #[serde(default)]
    upstream_citations: Vec<String>,
    #[serde(default)]
    publication_seconds: Option<i64>,
    #[serde(default)]
    generation_fingerprint: Option<String>,
}
fn evaluate(input: Value) -> Result<Value, Error> {
    let request: Request = serde_json::from_value(input)?;
    if request.schema_version != "corrobore-red-team-request-v1"
        || request.documents.len() > 128
        || request.queries.is_empty()
        || request.queries.len() > 16
    {
        return Err("unsupported request version or workload size".into());
    }
    let at = TemporalTimestamp::new(&request.as_of)?;
    let stamp = BitemporalStamp::new(at.clone(), at.clone())?;
    let mut graph = Graph::new();
    let mut sources = BTreeSet::new();
    let mut document_ids = BTreeSet::new();
    let mut query_ids = BTreeSet::new();
    for query in &request.queries {
        if !query_ids.insert(&query.id) {
            return Err("duplicate query ID".into());
        }
        graph
            .epistemic_stores_mut()
            .claims
            .create_asserted_claim(ClaimInput::new(
                ClaimId::new(&query.id)?,
                ClaimStatement::new(format!(
                    "{} {} = {}",
                    query.subject, query.predicate, query.value
                ))?,
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(&query.subject, None)),
            ))?;
    }
    let mut active_features = Vec::new();
    for document in &request.documents {
        if !document_ids.insert(&document.id) {
            return Err("duplicate document ID".into());
        }
        let source = SourceId::new(&document.source_id)?;
        if sources.insert(document.source_id.clone()) {
            graph
                .epistemic_stores_mut()
                .sources
                .register_source(SourceInput::new(
                    source.clone(),
                    format!("urn:synthetic:{}", document.source_id),
                    EvidenceSourceType::Document,
                ))?;
        }
        let observation = ObservationId::new(format!("observation:{}", document.id))?;
        let stores = graph.epistemic_stores_mut();
        stores.observations.create_observation(
            ObservationInput::new(
                observation.clone(),
                source.clone(),
                &document.body,
                ObservationModality::Text,
            ),
            &stores.sources,
        )?;
        stores.claims.register_observation(observation.clone());
        let evidence = EvidenceId::new(&document.id)?;
        graph.create_evidence(
            EvidenceInput::new(evidence.clone(), &document.source_id, &document.body)
                .with_source_id(source)
                .with_observation_id(observation.clone()),
        )?;
        let mut validity =
            BitemporalStamp::new(TemporalTimestamp::new(&document.valid_from)?, at.clone())?;
        if let Some(end) = &document.valid_until {
            validity = validity.with_valid_to(TemporalTimestamp::new(end)?)?;
        }
        for query in &request.queries {
            if query.subject != document.assertion.subject
                || query.predicate != document.assertion.predicate
            {
                continue;
            }
            let kind = if query.value == document.assertion.value {
                ClaimLinkKind::Supports
            } else {
                ClaimLinkKind::Refutes
            };
            graph.epistemic_stores_mut().claims.attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Observation(observation.clone()),
                    ClaimId::new(&query.id)?,
                    kind,
                )
                .with_strength(Confidence::new(1.0)?)
                .with_bitemporal(validity.clone()),
            )?;
        }
        if document.valid_from <= request.as_of
            && document
                .valid_until
                .as_ref()
                .is_none_or(|end| end > &request.as_of)
        {
            let mut features = EvidenceRiskFeatures::new(evidence, "synthetic-adapter-v1");
            if let Some(f) = &document.risk_features {
                features.attribution = f.attribution.clone();
                features.embedding = f.embedding.clone();
                features.embedding_model = f.embedding_model.clone();
                features.infrastructure = f.infrastructure.clone();
                features.upstream_citations = f.upstream_citations.clone();
                features.publication_seconds = f.publication_seconds;
                features.generation_fingerprint = f.generation_fingerprint.clone();
            }
            active_features.push(features);
        }
    }
    let mut tiers = GraphTierRegistry::new();
    let mut immune = ImmuneResponder::new();
    if request.mode == Mode::Defended {
        for query in &request.queries {
            let relevant: Vec<_> = active_features
                .iter()
                .filter(|f| {
                    request.documents.iter().any(|d| {
                        d.id == f.evidence_id.as_str()
                            && d.assertion.subject == query.subject
                            && d.assertion.predicate == query.predicate
                    })
                })
                .cloned()
                .collect();
            // Cover every pair and triple without exceeding the engine's 64-record
            // assessment limit. Three 21-record blocks cover temporal bursts too.
            let chunks: Vec<_> = relevant.chunks(21).collect();
            for a in 0..chunks.len() {
                for b in a..chunks.len() {
                    for c in b..chunks.len() {
                        if chunks.len() >= 3 && !(a < b && b < c) {
                            continue;
                        }
                        if chunks.len() < 3 && (a != 0 || b != 0 || c != chunks.len() - 1) {
                            continue;
                        }
                        let blocks: BTreeSet<_> = [a, b, c].into_iter().collect();
                        let batch: Vec<_> = blocks
                            .into_iter()
                            .flat_map(|i| chunks[i].iter().cloned())
                            .collect();
                        graph.apply_evidence_risks(
                            &ClaimId::new(&query.id)?,
                            &batch,
                            stamp.clone(),
                            "synthetic-red-team-v1",
                            &mut tiers,
                            &mut immune,
                        )?;
                    }
                }
            }
        }
    } else if request.mode == Mode::QuarantineAll {
        for document in &request.documents {
            tiers.transition(
                TierRecordRef::Evidence(EvidenceId::new(&document.id)?),
                GraphTier::Quarantine,
                "deliberately-excessive-control-v1",
                TierTransitionReason::ValidatorFinding,
            )?;
        }
    }
    let mut bindings = Vec::new();
    for source in sources {
        // The excessive control refuses all source authority after quarantining
        // every document. It must produce unknown answers and lose utility.
        let weight = if request.mode != Mode::QuarantineAll
            && request
                .authority_policy
                .trusted_source_ids
                .contains(&source)
        {
            1.0
        } else {
            0.0
        };
        bindings.push(SourceAuthority::new(
            SourceId::new(source)?,
            "synthetic",
            "fact",
            Confidence::new(weight)?,
            &request.authority_policy.version,
        )?);
    }
    graph
        .epistemic_stores_mut()
        .verdicts
        .register_source_authority_policy(SourceAuthorityPolicy::new(
            &request.authority_policy.version,
            bindings,
        )?)?;
    let evidence = graph.evidence_store().clone();
    let mut verdicts = BTreeMap::new();
    for query in &request.queries {
        let stores = graph.epistemic_stores_mut();
        let inputs = ResolutionInputs::new(
            &stores.verifications,
            &evidence,
            &stores.observations,
            &stores.sources,
        )
        .with_source_authority(&request.authority_policy.version, "synthetic", "fact");
        let claim = ClaimId::new(&query.id)?;
        resolve_current_claim_verdict(
            &mut stores.claims,
            &mut stores.verdicts,
            &inputs,
            &claim,
            stamp.clone(),
        )?;
        verdicts.insert(
            query.id.clone(),
            stores
                .verdicts
                .current_verdict(&claim)
                .ok_or("missing resolved verdict")?
                .state()
                .as_str(),
        );
    }
    let mut audits = BTreeMap::new();
    for query in &request.queries {
        audits.insert(
            &query.id,
            graph.claim_audit_path(&ClaimId::new(&query.id)?)?,
        );
    }
    Ok(
        json!({"schemaVersion":"corrobore-red-team-response-v1", "engine":"graph-core", "policyVersion":CLUSTER_AGGREGATION_POLICY_VERSION, "verdicts":verdicts,"quarantinedCount":tiers.records_in_tier(GraphTier::Quarantine).len(),"tierTransitions":tiers.audit_trail(),"audits":audits}),
    )
}
fn main() -> Result<(), Error> {
    let mut input = String::new();
    io::stdin().take(4_000_001).read_to_string(&mut input)?;
    if input.len() > 4_000_000 {
        return Err("request exceeds 4 MB".into());
    }
    println!("{}", evaluate(serde_json::from_str(&input)?)?);
    Ok(())
}
#[path = "red_team/tests.rs"]
#[cfg(test)]
mod tests;
