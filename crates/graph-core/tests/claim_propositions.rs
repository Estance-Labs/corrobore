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
//! Integration contract for the structured claim proposition (Epic 0029,
//! WS-A item 1, issue #154).
//!
//! A proposition is the atomic, machine-readable form of a claim: subject,
//! predicate, object or value, polarity, modality, valid-time scope, and
//! extraction version. It sits beside the free-text statement, never replaces
//! it, and stays optional so every claim serialized before this change keeps
//! deserializing unchanged.
use graph_core::{
    Claim, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimModality, ClaimPolarity,
    ClaimProposition, ClaimPropositionObject, ClaimStatement, ClaimStore, ClaimTarget,
    ClaimTargetValidationContext, ClaimValidTimeScope, GraphError, NodeId, PropertyValue,
    TemporalTimestamp,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn statement(text: &str) -> ClaimStatement {
    ClaimStatement::new(text).expect("test statement should be valid")
}

fn analytical_target(text: &str) -> ClaimTarget {
    ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(text, None))
}

fn entity_proposition() -> ClaimProposition {
    ClaimProposition::new(
        "actor--apt-k-47",
        "operates",
        ClaimPropositionObject::Entity(node_id("campaign--winter-lantern")),
    )
    .expect("test proposition should be valid")
}

//
// Verify that a proposition preserves every structured field the product
// requirements name for an atomic claim, with affirmed and asserted defaults
// so callers only state what departs from the plain reading of a statement.
#[test]
fn proposition_builder_preserves_all_fields_and_applies_defaults() {
    let minimal = entity_proposition();

    assert_eq!(minimal.subject(), "actor--apt-k-47");
    assert_eq!(minimal.predicate(), "operates");
    assert_eq!(
        minimal.object(),
        &ClaimPropositionObject::Entity(node_id("campaign--winter-lantern"))
    );
    assert_eq!(minimal.polarity(), ClaimPolarity::Affirmed);
    assert_eq!(minimal.modality(), ClaimModality::Asserted);
    assert!(minimal.valid_time().is_none());
    assert!(minimal.extraction_version().is_none());

    let scope = ClaimValidTimeScope::new(
        Some(timestamp("2026-01-01T00:00:00Z")),
        Some(timestamp("2026-06-30T00:00:00Z")),
    )
    .expect("ordered scope should be valid");
    let full = entity_proposition()
        .with_polarity(ClaimPolarity::Negated)
        .with_modality(ClaimModality::Reported)
        .with_valid_time(scope.clone())
        .with_extraction_version("extractor-v3.2")
        .expect("non-blank extraction version should be accepted");

    assert_eq!(full.polarity(), ClaimPolarity::Negated);
    assert_eq!(full.modality(), ClaimModality::Reported);
    assert_eq!(full.valid_time(), Some(&scope));
    assert_eq!(full.extraction_version(), Some("extractor-v3.2"));
    assert_eq!(
        scope.valid_from().map(TemporalTimestamp::as_str),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(
        scope.valid_until().map(TemporalTimestamp::as_str),
        Some("2026-06-30T00:00:00Z")
    );
}

//
// Verify that the polarity and modality vocabularies are closed typed enums
// rather than free strings, so downstream verdict policy can match on them.
#[test]
fn polarity_and_modality_expose_closed_vocabularies() {
    assert_eq!(
        ClaimPolarity::ALL,
        [ClaimPolarity::Affirmed, ClaimPolarity::Negated]
    );
    assert_eq!(
        ClaimModality::ALL,
        [
            ClaimModality::Asserted,
            ClaimModality::Reported,
            ClaimModality::Hypothesized,
            ClaimModality::Predicted,
        ]
    );
    assert_eq!(ClaimPolarity::Negated.as_str(), "negated");
    assert_eq!(ClaimModality::Hypothesized.as_str(), "hypothesized");
}

//
// Verify that a proposition rejects blank subject or predicate: an atomic
// claim with an unnamed subject or relation cannot be verified or linked.
#[test]
fn proposition_rejects_blank_subject_and_predicate() {
    let blank_subject = ClaimProposition::new(
        "   ",
        "operates",
        ClaimPropositionObject::Literal(PropertyValue::String("x".to_owned())),
    );
    assert!(matches!(
        blank_subject,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("subject")
    ));

    let blank_predicate = ClaimProposition::new(
        "actor--apt-k-47",
        "",
        ClaimPropositionObject::Literal(PropertyValue::String("x".to_owned())),
    );
    assert!(matches!(
        blank_predicate,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("predicate")
    ));
}

//
// Verify that a literal object must carry a typed value: an explicit null is
// not a value a claim can assert about its subject.
#[test]
fn proposition_rejects_null_literal_object() {
    let result = ClaimProposition::new(
        "indicator--1",
        "has_confidence",
        ClaimPropositionObject::Literal(PropertyValue::Null),
    );

    assert!(matches!(
        result,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("literal")
    ));

    let typed = ClaimProposition::new(
        "indicator--1",
        "has_confidence",
        ClaimPropositionObject::Literal(PropertyValue::Float(0.8)),
    );
    assert!(typed.is_ok());
}

//
// Verify that the extraction version, when present, is non-blank so repair
// lineage in a later workstream can always name the extractor that produced
// the proposition.
#[test]
fn proposition_rejects_blank_extraction_version() {
    let result = entity_proposition().with_extraction_version(" ");

    assert!(matches!(
        result,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("extraction version")
    ));
}

//
// Verify that a valid-time scope must be ordered and must carry at least one
// bound; an empty scope is expressed by omitting the scope, not by two `None`
// bounds.
#[test]
fn valid_time_scope_requires_a_bound_and_ordering() {
    let inverted = ClaimValidTimeScope::new(
        Some(timestamp("2026-06-30T00:00:00Z")),
        Some(timestamp("2026-01-01T00:00:00Z")),
    );
    assert!(matches!(
        inverted,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("valid_from")
    ));

    let empty = ClaimValidTimeScope::new(None, None);
    assert!(matches!(
        empty,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("bound")
    ));

    let open_ended = ClaimValidTimeScope::new(Some(timestamp("2026-01-01T00:00:00Z")), None)
        .expect("open-ended scope should be valid");
    assert!(open_ended.valid_until().is_none());
}

//
// Verify that an entity object resolves against the same validation context
// claim targets use, and that an unknown node is reported with a typed error
// naming the missing node rather than silently accepted.
#[test]
fn entity_object_resolves_against_validation_context() {
    let mut context = ClaimTargetValidationContext::new();
    context.register_node(node_id("campaign--winter-lantern"));

    entity_proposition()
        .validate_references(&context)
        .expect("known entity object should resolve");

    let unknown = ClaimProposition::new(
        "actor--apt-k-47",
        "operates",
        ClaimPropositionObject::Entity(node_id("campaign--missing")),
    )
    .expect("proposition should be valid before reference resolution");
    let error = unknown
        .validate_references(&context)
        .expect_err("unknown entity object should be rejected");
    assert!(matches!(
        error,
        GraphError::ClaimPropositionEntityNotFound(id) if id == node_id("campaign--missing")
    ));

    let literal = ClaimProposition::new(
        "indicator--1",
        "has_confidence",
        ClaimPropositionObject::Literal(PropertyValue::Float(0.8)),
    )
    .expect("literal proposition should be valid");
    literal
        .validate_references(&ClaimTargetValidationContext::new())
        .expect("literal objects need no graph reference");
}

//
// Verify that a claim stores and returns its proposition beside the text
// statement, and that a claim created without one reads back `None`.
#[test]
fn claim_store_persists_optional_proposition_beside_statement() {
    let mut store = ClaimStore::new();

    let with_proposition = ClaimInput::new(
        claim_id("claim--structured"),
        statement("APT-K-47 operates Winter Lantern"),
        analytical_target("attribution"),
    )
    .with_proposition(entity_proposition());
    let without_proposition = ClaimInput::new(
        claim_id("claim--text-only"),
        statement("Something was observed"),
        analytical_target("observation"),
    );

    store
        .create_asserted_claim(with_proposition)
        .expect("claim with proposition should be created");
    store
        .create_candidate_claim(without_proposition)
        .expect("claim without proposition should be created");

    let structured = store
        .claim_by_id(&claim_id("claim--structured"))
        .expect("structured claim should exist");
    assert_eq!(
        structured.statement().as_str(),
        "APT-K-47 operates Winter Lantern"
    );
    assert_eq!(structured.proposition(), Some(&entity_proposition()));

    let text_only = store
        .claim_by_id(&claim_id("claim--text-only"))
        .expect("text-only claim should exist");
    assert!(text_only.proposition().is_none());
}

//
// Verify the compatibility contract: a claim serialized before the proposition
// existed has no `proposition` key, and such a payload deserializes into a
// claim whose proposition is `None`. Claims without a proposition must also
// keep serializing without the key so stored payloads stay byte-stable.
#[test]
fn claim_without_proposition_serializes_and_deserializes_without_the_key() {
    let mut store = ClaimStore::new();
    store
        .create_asserted_claim(ClaimInput::new(
            claim_id("claim--legacy"),
            statement("legacy statement"),
            analytical_target("legacy"),
        ))
        .expect("legacy-shaped claim should be created");
    let claim = store
        .claim_by_id(&claim_id("claim--legacy"))
        .expect("legacy claim should exist");

    let json = serde_json::to_value(claim).expect("claim should serialize");
    assert!(
        json.get("proposition").is_none(),
        "a claim without a proposition must not emit the key: {json}"
    );

    let restored: Claim = serde_json::from_value(json).expect("legacy payload should deserialize");
    assert_eq!(&restored, claim);
    assert!(restored.proposition().is_none());

    // A payload captured before this change carries exactly the pre-existing
    // fields; it must deserialize with `proposition = None`.
    let pre_change_payload = serde_json::json!({
        "id": { "value": "claim--pre-change" },
        "version_id": { "value": "claim-version--claim--pre-change--1" },
        "version": 1,
        "status": "Asserted",
        "statement": { "text": "pre-change statement" },
        "target": {
            "AnalyticalAssertion": { "summary": "pre-change", "hypothesis_workspace_ref": null }
        },
        "confidence": null,
        "created_by": null,
        "source_refs": [],
        "evidence_refs": [],
        "workspace_id": null,
        "extraction_run_id": null,
        "temporal": {
            "created_at": null, "updated_at": null, "recorded_at": null,
            "superseded_at": null, "observed_at": null, "first_seen": null,
            "last_seen": null, "valid_from": null, "valid_until": null
        }
    });
    let pre_change: Claim =
        serde_json::from_value(pre_change_payload).expect("pre-change payload should deserialize");
    assert!(pre_change.proposition().is_none());
    assert_eq!(pre_change.statement().as_str(), "pre-change statement");
}

//
// Verify that a claim carrying a proposition round-trips through serde with
// every structured field intact, including the entity object, polarity,
// modality, valid-time scope, and extraction version.
#[test]
fn claim_with_proposition_round_trips_through_serde() {
    let scope = ClaimValidTimeScope::new(Some(timestamp("2026-01-01T00:00:00Z")), None)
        .expect("scope should be valid");
    let proposition = entity_proposition()
        .with_polarity(ClaimPolarity::Negated)
        .with_modality(ClaimModality::Predicted)
        .with_valid_time(scope)
        .with_extraction_version("extractor-v3.2")
        .expect("extraction version should be accepted");

    let mut store = ClaimStore::new();
    store
        .create_asserted_claim(
            ClaimInput::new(
                claim_id("claim--round-trip"),
                statement("round trip"),
                analytical_target("round-trip"),
            )
            .with_proposition(proposition.clone()),
        )
        .expect("claim should be created");
    let claim = store
        .claim_by_id(&claim_id("claim--round-trip"))
        .expect("claim should exist");

    let json = serde_json::to_string(claim).expect("claim should serialize");
    let restored: Claim = serde_json::from_str(&json).expect("claim should deserialize");

    assert_eq!(&restored, claim);
    assert_eq!(restored.proposition(), Some(&proposition));
}

//
// Verify the graph-facing projection: a proposition renders as additive,
// namespaced properties so a `Claim` node in the epistemic vocabulary can
// expose it through Cypher reads without changing any existing property.
#[test]
fn proposition_projects_to_namespaced_properties() {
    let scope = ClaimValidTimeScope::new(
        Some(timestamp("2026-01-01T00:00:00Z")),
        Some(timestamp("2026-06-30T00:00:00Z")),
    )
    .expect("scope should be valid");
    let entity = entity_proposition()
        .with_modality(ClaimModality::Reported)
        .with_valid_time(scope)
        .with_extraction_version("extractor-v3.2")
        .expect("extraction version should be accepted");

    let properties = entity.to_property_map();

    assert_eq!(
        properties.get("proposition_subject"),
        Some(&PropertyValue::String("actor--apt-k-47".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_predicate"),
        Some(&PropertyValue::String("operates".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_object_kind"),
        Some(&PropertyValue::String("entity".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_object"),
        Some(&PropertyValue::String(
            "campaign--winter-lantern".to_owned()
        ))
    );
    assert_eq!(
        properties.get("proposition_polarity"),
        Some(&PropertyValue::String("affirmed".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_modality"),
        Some(&PropertyValue::String("reported".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_valid_from"),
        Some(&PropertyValue::String("2026-01-01T00:00:00Z".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_valid_until"),
        Some(&PropertyValue::String("2026-06-30T00:00:00Z".to_owned()))
    );
    assert_eq!(
        properties.get("proposition_extraction_version"),
        Some(&PropertyValue::String("extractor-v3.2".to_owned()))
    );
    assert!(properties.keys().all(|key| key.starts_with("proposition_")));

    let literal = ClaimProposition::new(
        "indicator--1",
        "has_confidence",
        ClaimPropositionObject::Literal(PropertyValue::Float(0.8)),
    )
    .expect("literal proposition should be valid");
    let literal_properties = literal.to_property_map();

    assert_eq!(
        literal_properties.get("proposition_object_kind"),
        Some(&PropertyValue::String("literal".to_owned()))
    );
    assert_eq!(
        literal_properties.get("proposition_object"),
        Some(&PropertyValue::Float(0.8))
    );
    assert!(!literal_properties.contains_key("proposition_valid_from"));
    assert!(!literal_properties.contains_key("proposition_extraction_version"));
}
