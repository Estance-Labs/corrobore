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
use graph_core::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{self, Read};
type Error = Box<dyn std::error::Error>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema_version: String,
    as_of: String,
    authority_policy: Authority,
    input: Input,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Authority {
    version: String,
    trusted_source_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    claim: Assertion,
    evidence: Vec<Document>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assertion {
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
}

const CLAIM_ID: &str = "abstention-claim";

/// Only Supported and Refuted are answers. Every other resolved state, including
/// the reachability-gated InsufficientEvidence, is an abstention.
fn decision(state: &str) -> Value {
    match state {
        "supported" => json!({"action": "answer", "verdict": "supported"}),
        "refuted" => json!({"action": "answer", "verdict": "refuted"}),
        _ => json!({"action": "abstain"}),
    }
}

fn evaluate(input: Value) -> Result<Value, Error> {
    let request: Request = serde_json::from_value(input)?;
    if request.schema_version != "corrobore-abstention-request-v1"
        || request.input.evidence.len() > 128
    {
        return Err("unsupported request version or workload size".into());
    }

    let at = TemporalTimestamp::new(&request.as_of)?;
    let stamp = BitemporalStamp::new(at.clone(), at.clone())?;
    let mut graph = Graph::new();
    let mut sources = BTreeSet::new();
    let mut document_ids = BTreeSet::new();
    let claim_id = ClaimId::new(CLAIM_ID)?;

    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim_id.clone(),
            ClaimStatement::new(format!(
                "{} {} = {}",
                request.input.claim.subject,
                request.input.claim.predicate,
                request.input.claim.value
            ))?,
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                &request.input.claim.subject,
                None,
            )),
        ))?;

    for document in &request.input.evidence {
        if !document_ids.insert(&document.id) {
            return Err("duplicate evidence ID".into());
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
            EvidenceInput::new(evidence, &document.source_id, &document.body)
                .with_source_id(source)
                .with_observation_id(observation.clone()),
        )?;

        let mut validity =
            BitemporalStamp::new(TemporalTimestamp::new(&document.valid_from)?, at.clone())?;
        if let Some(end) = &document.valid_until {
            validity = validity.with_valid_to(TemporalTimestamp::new(end)?)?;
        }

        // Evidence about another fact carries no stance, so no signal link is
        // attached and the claim stays unresolved.
        if document.assertion.subject != request.input.claim.subject
            || document.assertion.predicate != request.input.claim.predicate
        {
            continue;
        }
        let kind = if document.assertion.value == request.input.claim.value {
            ClaimLinkKind::Supports
        } else {
            ClaimLinkKind::Refutes
        };
        graph.epistemic_stores_mut().claims.attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim_id.clone(),
                kind,
            )
            .with_strength(Confidence::new(1.0)?)
            .with_bitemporal(validity),
        )?;
    }

    // Untrusted sources are bound at zero weight rather than left out, so the
    // engine withholds their authority instead of never seeing them.
    let mut bindings = Vec::new();
    for source in &sources {
        let weight = if request.authority_policy.trusted_source_ids.contains(source) {
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

    let evidence_store = graph.evidence_store().clone();
    let stores = graph.epistemic_stores_mut();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence_store,
        &stores.observations,
        &stores.sources,
    )
    .with_source_authority(&request.authority_policy.version, "synthetic", "fact");
    resolve_current_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim_id,
        stamp,
    )?;
    let state = stores
        .verdicts
        .current_verdict(&claim_id)
        .ok_or("missing resolved verdict")?
        .state()
        .as_str();

    Ok(json!({
        "schemaVersion": "corrobore-abstention-response-v1",
        "engine": "graph-core",
        "policyVersion": CLUSTER_AGGREGATION_POLICY_VERSION,
        "verdictState": state,
        "decision": decision(state)
    }))
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

#[path = "abstention/tests.rs"]
#[cfg(test)]
mod tests;
