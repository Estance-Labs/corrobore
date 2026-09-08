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
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read};
type Error = Box<dyn std::error::Error>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    schema_version: String,
    as_of: String,
    operation: String,
    claim: Claim,
    corpus: Vec<Document>,
    #[serde(default)]
    evidence: Vec<Document>,
}
#[derive(Deserialize)]
struct Claim {
    id: String,
    text: String,
}
// claimId is an evaluator-owned label; it is deliberately not deserialized so
// retrieval cannot short-circuit on it.
#[derive(Deserialize, Clone)]
struct Document {
    id: String,
    entity: String,
    stance: String,
    text: String,
}

const TOP_K: usize = 10;

fn ingest(graph: &mut Graph, documents: &[Document]) -> Result<(), Error> {
    for document in documents {
        let evidence = EvidenceId::new(&document.id)?;
        graph.create_evidence(EvidenceInput::new(
            evidence.clone(),
            &document.entity,
            &document.text,
        ))?;
        // The seed resolver matches node labels and properties, not evidence
        // bodies, so each document also needs a searchable node.
        graph.create_node(
            NodeInput::new(["Evidence"])
                .with_property("name", PropertyValue::String(document.text.clone()))
                .with_evidence_ref(evidence),
        )?;
    }
    Ok(())
}

fn counters(inputs: usize, outputs: usize, failures: usize) -> Value {
    json!({"inputs": inputs, "outputs": outputs, "failures": failures})
}

fn retrieve(request: &Request) -> Result<Value, Error> {
    let mut graph = Graph::new();
    ingest(&mut graph, &request.corpus)?;

    let query = SemanticSeedQueryRequest::new(
        request.claim.text.clone(),
        WorkspaceId::new("workspace--evidence-suite")?,
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::FullText,
        TOP_K,
        0.0,
    )?;
    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver.resolve(&query)?;

    let known: BTreeSet<&str> = request.corpus.iter().map(|item| item.id.as_str()).collect();
    let mut ids = Vec::new();
    for candidate in response.seed_candidates() {
        for reference in candidate.explanation().source_refs() {
            if known.contains(reference.as_str()) && !ids.contains(reference) {
                ids.push(reference.clone());
            }
        }
    }
    Ok(json!({
        "schemaVersion": "corrobore-evidence-suite-response-v1",
        "engine": "graph-core",
        "evidenceIds": ids,
        "instrumentation": {
            "retrieval": counters(1, ids.len(), 0)
        }
    }))
}

fn link_kind(stance: &str) -> Option<ClaimLinkKind> {
    match stance {
        "supports" => Some(ClaimLinkKind::Supports),
        "refutes" => Some(ClaimLinkKind::Refutes),
        _ => None,
    }
}

fn evaluate_claim(request: &Request) -> Result<Value, Error> {
    let at = TemporalTimestamp::new(&request.as_of)?;
    let stamp = BitemporalStamp::new(at.clone(), at.clone())?;
    let mut graph = Graph::new();
    let claim_id = ClaimId::new(&request.claim.id)?;
    let mut sources = BTreeSet::new();
    let mut observation_source = HashMap::new();

    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim_id.clone(),
            ClaimStatement::new(request.claim.text.clone())?,
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(&request.claim.id, None)),
        ))?;

    let mut extraction = Vec::new();
    for document in &request.evidence {
        let source = SourceId::new(&document.entity)?;
        if sources.insert(document.entity.clone()) {
            graph
                .epistemic_stores_mut()
                .sources
                .register_source(SourceInput::new(
                    source.clone(),
                    format!("urn:synthetic:{}", document.entity),
                    EvidenceSourceType::Document,
                ))?;
        }
        let observation = ObservationId::new(format!("observation:{}", document.id))?;
        let stores = graph.epistemic_stores_mut();
        stores.observations.create_observation(
            ObservationInput::new(
                observation.clone(),
                source.clone(),
                &document.text,
                ObservationModality::Text,
            ),
            &stores.sources,
        )?;
        stores.claims.register_observation(observation.clone());
        observation_source.insert(observation.as_str().to_owned(), document.entity.clone());

        let evidence = EvidenceId::new(&document.id)?;
        graph.create_evidence(
            EvidenceInput::new(evidence, &document.entity, &document.text)
                .with_source_id(source)
                .with_observation_id(observation.clone()),
        )?;
        extraction.push(document.id.clone());

        // A neutral document carries no stance, so it attaches no signal link.
        if let Some(kind) = link_kind(&document.stance) {
            graph.epistemic_stores_mut().claims.attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Observation(observation),
                    claim_id.clone(),
                    kind,
                )
                .with_strength(Confidence::new(1.0)?)
                .with_bitemporal(stamp.clone()),
            )?;
        }
    }

    let bindings = sources
        .iter()
        .map(|source| {
            SourceAuthority::new(
                SourceId::new(source)?,
                "synthetic",
                "fact",
                Confidence::new(1.0)?,
                "evidence-suite-v1",
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    graph
        .epistemic_stores_mut()
        .verdicts
        .register_source_authority_policy(SourceAuthorityPolicy::new(
            "evidence-suite-v1",
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
    .with_source_authority("evidence-suite-v1", "synthetic", "fact");
    let outcome = resolve_current_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim_id,
        stamp,
    )?;
    let state = outcome.state().as_str().to_owned();

    let as_of = VerdictAsOf::new(at.clone(), at);
    let mut subgraph = Vec::new();
    let mut entities = BTreeSet::new();
    for link in stores.claims.links_active_at(&claim_id, &as_of) {
        match link.source() {
            ClaimLinkSource::Observation(id) => {
                if let Some(entity) = observation_source.get(id.as_str()) {
                    entities.insert(entity.clone());
                }
                if let Some(name) = id.as_str().strip_prefix("observation:") {
                    subgraph.push(name.to_owned());
                }
            }
            ClaimLinkSource::Evidence(id) => subgraph.push(id.as_str().to_owned()),
            ClaimLinkSource::Claim(_) => {}
        }
    }

    let sufficiency = if state == "insufficient_evidence" || subgraph.is_empty() {
        "insufficient"
    } else {
        "sufficient"
    };

    // A document that attached no signal link produced no assertion, so it
    // counts as an input the extraction stage did not turn into an output.
    let documents = request.evidence.len();
    let stanced = request
        .evidence
        .iter()
        .filter(|document| link_kind(&document.stance).is_some())
        .count();
    let entities: Vec<String> = entities.into_iter().collect();

    Ok(json!({
        "schemaVersion": "corrobore-evidence-suite-response-v1",
        "engine": "graph-core",
        "stages": {
            "extraction": extraction,
            "entity_resolution": entities.clone(),
            "subgraph_construction": subgraph.clone(),
            "evidence_sufficiency": [sufficiency],
            "verifier": [state.clone()],
            "verdict": [state]
        },
        "instrumentation": {
            "extraction": counters(documents, stanced, documents - stanced),
            "entity_resolution": counters(documents, entities.len(), 0),
            "subgraph_construction": counters(1, subgraph.len(), 0),
            "evidence_sufficiency": counters(1, 1, 0),
            "verifier": counters(1, 1, 0),
            "verdict": counters(1, 1, 0)
        }
    }))
}

fn evaluate(input: Value) -> Result<Value, Error> {
    let request: Request = serde_json::from_value(input)?;
    if request.schema_version != "corrobore-evidence-suite-request-v1" || request.corpus.len() > 512
    {
        return Err("unsupported request version or workload size".into());
    }
    match request.operation.as_str() {
        "retrieve" => retrieve(&request),
        "evaluate" => evaluate_claim(&request),
        other => Err(format!("unsupported operation: {other}").into()),
    }
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

#[path = "evidence_suite/tests.rs"]
#[cfg(test)]
mod tests;
