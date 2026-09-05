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
//! Integration contract for the verifier framework (Epic 0029, WS-B item 1,
//! issue #163).
//!
//! ADR-0017: the `Verifier` trait and `VerifierRegistry` live in the core, a
//! verifier reports rather than adjudicates, and the registry owns provenance:
//! a verifier never sets the record identifier, the stamp, or the
//! `deterministic` flag. Versions live beside each other so a logic change
//! never rewrites an earlier record.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimStore, ClaimTarget, EvidenceRecordStore,
    EvidenceSourceType, GraphError, ObservationId, ObservationInput, ObservationModality,
    ObservationStore, SourceId, SourceInput, SourceStore, TemporalTimestamp, VerificationContext,
    VerificationOutcome, VerificationRecordStore, VerificationRequest, VerificationResult,
    Verifier, VerifierCostClass, VerifierRegistry, VerifierSpec,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("claim id")
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp")
}

fn stamp(transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-08-01T00:00:00Z"), ts(transaction)).expect("stamp")
}

/// A verifier that reports a fixed result and records what it saw, so the
/// tests can assert what the request carried.
struct Probe {
    id: &'static str,
    version: &'static str,
    deterministic: bool,
    result: VerificationResult,
}

impl Verifier for Probe {
    fn id(&self) -> &str {
        self.id
    }

    fn version(&self) -> &str {
        self.version
    }

    fn deterministic(&self) -> bool {
        self.deterministic
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Low
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let mut outcome = VerificationOutcome::new(self.result).with_rationale(format!(
            "{} links, {} observations",
            request.links().len(),
            request.observations().len()
        ));
        for observation in request.observations() {
            outcome = outcome
                .with_evidence_consumed(format!("observation:{}", observation.id().as_str()));
        }
        Ok(outcome.with_limit("fixture verifier; checks nothing"))
    }
}

fn probe(id: &'static str, version: &'static str, result: VerificationResult) -> Probe {
    Probe {
        id,
        version,
        deterministic: true,
        result,
    }
}

struct Fixture {
    claims: ClaimStore,
    observations: ObservationStore,
    sources: SourceStore,
    evidence: EvidenceRecordStore,
    records: VerificationRecordStore,
}

impl Fixture {
    fn new() -> Self {
        let mut sources = SourceStore::new();
        sources
            .register_source(SourceInput::new(
                SourceId::new("source--report").expect("id"),
                "https://vendor.example/report.pdf",
                EvidenceSourceType::Document,
            ))
            .expect("source");
        let mut observations = ObservationStore::new();
        observations
            .create_observation(
                ObservationInput::new(
                    ObservationId::new("observation--span").expect("id"),
                    SourceId::new("source--report").expect("id"),
                    "Actor A operates Campaign B.",
                    ObservationModality::Text,
                ),
                &sources,
            )
            .expect("observation");

        let mut claims = ClaimStore::new();
        claims
            .create_asserted_claim(ClaimInput::new(
                claim_id("claim--attribution"),
                ClaimStatement::new("Actor A operates Campaign B").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("attribution", None)),
            ))
            .expect("claim");
        claims.register_observation(ObservationId::new("observation--span").expect("id"));
        claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(ObservationId::new("observation--span").expect("id")),
                claim_id("claim--attribution"),
                ClaimLinkKind::Supports,
            ))
            .expect("link");

        Self {
            claims,
            observations,
            sources,
            evidence: EvidenceRecordStore::new(),
            records: VerificationRecordStore::new(),
        }
    }

    fn run(
        &mut self,
        registry: &VerifierRegistry,
        verifier_id: &str,
        version: &str,
        transaction: &str,
    ) -> Result<graph_core::VerificationRecordId, GraphError> {
        let context = VerificationContext::new(
            &self.claims,
            &self.observations,
            &self.sources,
            &self.evidence,
        );
        registry.run(
            verifier_id,
            version,
            &claim_id("claim--attribution"),
            &context,
            &mut self.records,
            stamp(transaction),
        )
    }

    fn context(&self) -> VerificationContext<'_> {
        VerificationContext::new(
            &self.claims,
            &self.observations,
            &self.sources,
            &self.evidence,
        )
    }
}

//
// Verify that a run resolves the claim, builds a request carrying its active
// links, observations, and sources, and appends a record whose provenance the
// registry owns.
#[test]
fn a_run_builds_the_request_and_appends_a_record() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(probe(
            "verifier.fixture",
            "1.0.0",
            VerificationResult::Pass,
        ))))
        .expect("registration");

    let record_id = fixture
        .run(
            &registry,
            "verifier.fixture",
            "1.0.0",
            "2026-08-30T10:00:00Z",
        )
        .expect("run");

    let record = fixture
        .records
        .record_by_id(&record_id)
        .expect("record should be stored");
    assert_eq!(record.verifier_id(), "verifier.fixture");
    assert_eq!(record.verifier_version(), "1.0.0");
    assert!(record.deterministic());
    assert_eq!(record.result(), VerificationResult::Pass);
    assert_eq!(record.inputs().claim_id(), &claim_id("claim--attribution"));
    assert_eq!(
        record.rationale(),
        Some("1 links, 1 observations"),
        "the request carried the active link and its observation"
    );
    assert_eq!(
        record.evidence_consumed(),
        ["observation:observation--span"]
    );
    assert_eq!(record.limits(), ["fixture verifier; checks nothing"]);
    assert_eq!(
        record.stamp().transaction_time.as_str(),
        "2026-08-30T10:00:00Z"
    );
    assert_eq!(
        record.inputs().observation_ids(),
        [ObservationId::new("observation--span").expect("id")]
    );
    assert!(
        record.id().as_str().contains("verifier.fixture"),
        "the registry mints an identifier naming the verifier: {}",
        record.id().as_str()
    );
}

//
// Verify that the registry, not the verifier, owns the `deterministic` flag:
// a verifier declaring itself deterministic cannot emit a non-deterministic
// record, and the reverse holds too.
#[test]
fn the_registry_owns_provenance_not_the_verifier() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(Probe {
            id: "verifier.model",
            version: "2.0.0",
            deterministic: false,
            result: VerificationResult::Pass,
        })))
        .expect("registration");

    let record_id = fixture
        .run(&registry, "verifier.model", "2.0.0", "2026-08-30T10:00:00Z")
        .expect("run");
    let record = fixture.records.record_by_id(&record_id).expect("record");
    assert!(!record.deterministic());
    assert_eq!(
        record.verifier_version(),
        "2.0.0",
        "the record carries the registered version, not one the outcome chose"
    );

    // Two runs of the same verifier at different transaction times produce two
    // records: the registry never reuses an identifier.
    let second = fixture
        .run(&registry, "verifier.model", "2.0.0", "2026-08-30T11:00:00Z")
        .expect("second run");
    assert_ne!(second, record_id);
    assert_eq!(fixture.records.len(), 2);
}

//
// Verify registration rules: an identifier and version pair is claimed once.
// The registry cannot prove that two implementations under the same pair
// agree, and records already written under it must stay reproducible, so a
// second registration is refused rather than silently accepted. A new version
// registers beside the old one.
#[test]
fn an_id_and_version_pair_is_claimed_once_and_versions_coexist() {
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(probe(
            "verifier.fixture",
            "1.0.0",
            VerificationResult::Pass,
        ))))
        .expect("first registration");
    assert_eq!(registry.len(), 1);

    let duplicate = registry.register(VerifierSpec::new(Box::new(probe(
        "verifier.fixture",
        "1.0.0",
        VerificationResult::Fail,
    ))));
    assert!(
        matches!(duplicate, Err(GraphError::InvalidVerifierRegistration(message))
            if message.contains("verifier.fixture") && message.contains("1.0.0")),
        "the pair is already claimed; the registry cannot prove the implementations agree"
    );
    assert_eq!(registry.len(), 1);

    for (id, version) in [("", "1.0.0"), ("verifier.blank", "  ")] {
        let blank = registry.register(VerifierSpec::new(Box::new(Probe {
            id: Box::leak(id.to_owned().into_boxed_str()),
            version: Box::leak(version.to_owned().into_boxed_str()),
            deterministic: true,
            result: VerificationResult::Pass,
        })));
        assert!(matches!(
            blank,
            Err(GraphError::InvalidVerifierRegistration(_))
        ));
    }

    registry
        .register(VerifierSpec::new(Box::new(probe(
            "verifier.fixture",
            "1.1.0",
            VerificationResult::Fail,
        ))))
        .expect("a new version registers beside the old one");
    assert_eq!(registry.len(), 2);
    assert_eq!(registry.versions_of("verifier.fixture"), ["1.0.0", "1.1.0"]);
    assert_eq!(registry.latest_version("verifier.fixture"), Some("1.1.0"));
}

//
// Verify that both verifier versions remain runnable and that their records
// coexist, so a logic change never rewrites verification history.
#[test]
fn both_versions_stay_runnable_and_their_records_coexist() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    for (version, result) in [
        ("1.0.0", VerificationResult::Pass),
        ("1.1.0", VerificationResult::Fail),
    ] {
        registry
            .register(VerifierSpec::new(Box::new(probe(
                "verifier.fixture",
                version,
                result,
            ))))
            .expect("registration");
    }

    let old = fixture
        .run(
            &registry,
            "verifier.fixture",
            "1.0.0",
            "2026-08-30T10:00:00Z",
        )
        .expect("old version runs");
    let new = fixture
        .run(
            &registry,
            "verifier.fixture",
            "1.1.0",
            "2026-08-30T11:00:00Z",
        )
        .expect("new version runs");

    assert_ne!(old, new);
    assert_eq!(fixture.records.len(), 2);
    let records = fixture
        .records
        .records_for_claim(&claim_id("claim--attribution"));
    assert_eq!(records[0].verifier_version(), "1.0.0");
    assert_eq!(records[0].result(), VerificationResult::Pass);
    assert_eq!(records[1].verifier_version(), "1.1.0");
    assert_eq!(records[1].result(), VerificationResult::Fail);
}

//
// Verify the error paths: an unknown verifier, an unknown version of a known
// verifier, and an unknown claim are typed errors, and none of them writes a
// record.
#[test]
fn unknown_verifier_version_or_claim_are_typed_errors() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(probe(
            "verifier.fixture",
            "1.0.0",
            VerificationResult::Pass,
        ))))
        .expect("registration");

    assert!(matches!(
        fixture.run(&registry, "verifier.missing", "1.0.0", "2026-08-30T10:00:00Z"),
        Err(GraphError::VerifierNotFound { id, version }) if id == "verifier.missing" && version == "1.0.0"
    ));
    assert!(matches!(
        fixture.run(&registry, "verifier.fixture", "9.9.9", "2026-08-30T10:00:00Z"),
        Err(GraphError::VerifierNotFound { id, version }) if id == "verifier.fixture" && version == "9.9.9"
    ));

    let context = VerificationContext::new(
        &fixture.claims,
        &fixture.observations,
        &fixture.sources,
        &fixture.evidence,
    );
    let missing_claim = registry.run(
        "verifier.fixture",
        "1.0.0",
        &claim_id("claim--missing"),
        &context,
        &mut fixture.records,
        stamp("2026-08-30T10:00:00Z"),
    );
    assert!(
        matches!(missing_claim, Err(GraphError::ClaimNotFound(id)) if id == claim_id("claim--missing"))
    );
    assert!(fixture.records.is_empty(), "no failed run writes a record");
}

//
// Verify that a verifier failing with a typed error propagates it and writes
// no record: a crash is not an inconclusive result.
#[test]
fn a_failing_verifier_writes_no_record() {
    struct Broken;
    impl Verifier for Broken {
        fn id(&self) -> &str {
            "verifier.broken"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn deterministic(&self) -> bool {
            true
        }
        fn cost_class(&self) -> VerifierCostClass {
            VerifierCostClass::Low
        }
        fn verify(
            &self,
            _request: &VerificationRequest<'_>,
        ) -> Result<VerificationOutcome, GraphError> {
            Err(GraphError::InvalidPropertyValue(
                "verifier blew up".to_owned(),
            ))
        }
    }

    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(Broken)))
        .expect("registration");

    let result = fixture.run(
        &registry,
        "verifier.broken",
        "1.0.0",
        "2026-08-30T10:00:00Z",
    );
    assert!(matches!(
        result,
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("blew up")
    ));
    assert!(fixture.records.is_empty());
}

//
// Verify the request exposes the source path behind each observation, which is
// what the deterministic verifiers of items 2 and 3 will read.
#[test]
fn the_request_exposes_the_source_behind_every_observation() {
    let fixture = Fixture::new();
    let request = VerificationRequest::build(
        &claim_id("claim--attribution"),
        &fixture.context(),
        &stamp("2026-08-30T10:00:00Z"),
    )
    .expect("request should build");

    assert_eq!(request.claim().id(), &claim_id("claim--attribution"));
    assert_eq!(request.links().len(), 1);
    assert_eq!(request.observations().len(), 1);
    assert_eq!(request.sources().len(), 1);
    assert_eq!(
        request.sources()[0].uri(),
        "https://vendor.example/report.pdf"
    );
    assert_eq!(
        request
            .source_of(request.observations()[0])
            .expect("source path")
            .id()
            .as_str(),
        "source--report"
    );
    assert!(request.evidence_records().is_empty());
}
