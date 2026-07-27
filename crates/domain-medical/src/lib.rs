// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![warn(missing_docs)]

//! MIT-licensed MEDICAL domain pack: clinical and biomedical evidence types,
//! fail-closed validation, deterministic built-ins, and a reproducible
//! evidence-table exporter over Corrobore agentic memory.
//!
//! The pack is not a medical device and not a clinical decision support system.
//! No node type represents an identifiable patient or research participant, and
//! participant-level content is rejected without an attested de-identification
//! status. Evidence level and confidence are kept as distinct fields; no
//! built-in converts one into the other.

mod export;
mod provider_abi;

pub use export::{
    EvidenceTableExport, EvidenceTableExporter, EvidenceTableRow, EvidenceTableStudy, ExportError,
    ExportMode,
};
pub use provider_abi::medical_provider_api_v1;

use domain_common::{
    ConfidencePolicy, DomainValidationIssue, DomainValidationResult, DomainValidationSeverity,
    EvidenceRequirement, validate_evidence_requirement,
};
use graph_core::Confidence;
use serde::{Deserialize, Serialize};

/// MEDICAL node type.
///
/// The set intentionally has no type for an identifiable patient or research
/// participant. `Population` is an aggregate cohort definition, and `Person`
/// covers investigators, authors, and reviewers, whose professional identity is
/// distinct from participant-level clinical data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicalNodeType {
    /// Study.
    Study,
    /// Clinical trial.
    ClinicalTrial,
    /// Protocol.
    Protocol,
    /// Population, an aggregate cohort definition.
    Population,
    /// Intervention.
    Intervention,
    /// Comparator.
    Comparator,
    /// Condition.
    Condition,
    /// Outcome.
    Outcome,
    /// Finding.
    Finding,
    /// Effect estimate.
    EffectEstimate,
    /// Adverse event.
    AdverseEvent,
    /// Biomarker.
    Biomarker,
    /// Specimen.
    Specimen,
    /// Measurement.
    Measurement,
    /// Publication.
    Publication,
    /// Guideline.
    Guideline,
    /// Regulatory decision.
    RegulatoryDecision,
    /// Dataset.
    Dataset,
    /// Person, an investigator, author, or reviewer.
    Person,
    /// Institution.
    Institution,
    /// Evidence.
    Evidence,
}

impl MedicalNodeType {
    /// Returns the stable label string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Study => "Study",
            Self::ClinicalTrial => "ClinicalTrial",
            Self::Protocol => "Protocol",
            Self::Population => "Population",
            Self::Intervention => "Intervention",
            Self::Comparator => "Comparator",
            Self::Condition => "Condition",
            Self::Outcome => "Outcome",
            Self::Finding => "Finding",
            Self::EffectEstimate => "EffectEstimate",
            Self::AdverseEvent => "AdverseEvent",
            Self::Biomarker => "Biomarker",
            Self::Specimen => "Specimen",
            Self::Measurement => "Measurement",
            Self::Publication => "Publication",
            Self::Guideline => "Guideline",
            Self::RegulatoryDecision => "RegulatoryDecision",
            Self::Dataset => "Dataset",
            Self::Person => "Person",
            Self::Institution => "Institution",
            Self::Evidence => "Evidence",
        }
    }

    /// Resolves a label string to a node type.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        let resolved = match label {
            "Study" => Self::Study,
            "ClinicalTrial" => Self::ClinicalTrial,
            "Protocol" => Self::Protocol,
            "Population" => Self::Population,
            "Intervention" => Self::Intervention,
            "Comparator" => Self::Comparator,
            "Condition" => Self::Condition,
            "Outcome" => Self::Outcome,
            "Finding" => Self::Finding,
            "EffectEstimate" => Self::EffectEstimate,
            "AdverseEvent" => Self::AdverseEvent,
            "Biomarker" => Self::Biomarker,
            "Specimen" => Self::Specimen,
            "Measurement" => Self::Measurement,
            "Publication" => Self::Publication,
            "Guideline" => Self::Guideline,
            "RegulatoryDecision" => Self::RegulatoryDecision,
            "Dataset" => Self::Dataset,
            "Person" => Self::Person,
            "Institution" => Self::Institution,
            "Evidence" => Self::Evidence,
            _ => return None,
        };
        Some(resolved)
    }
}

/// MEDICAL relationship type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicalRelationshipType {
    /// Investigates.
    Investigates,
    /// Enrolls.
    Enrolls,
    /// Compares.
    Compares,
    /// Administers.
    Administers,
    /// Measures.
    Measures,
    /// Reports finding.
    ReportsFinding,
    /// Estimates effect.
    EstimatesEffect,
    /// Associated with.
    AssociatedWith,
    /// Contraindicated with.
    ContraindicatedWith,
    /// Caused adverse event.
    CausedAdverseEvent,
    /// Derived from.
    DerivedFrom,
    /// Conflicts with.
    ConflictsWith,
    /// Supersedes.
    Supersedes,
    /// Retracts.
    Retracts,
    /// Published in.
    PublishedIn,
    /// Conducted by.
    ConductedBy,
    /// Supported by.
    SupportedBy,
}

/// Study design classification, the structural basis of evidence level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StudyDesign {
    /// Systematic review or meta-analysis.
    SystematicReview,
    /// Randomized controlled trial.
    RandomizedControlledTrial,
    /// Cohort study.
    CohortStudy,
    /// Case-control study.
    CaseControlStudy,
    /// Case series.
    CaseSeries,
    /// Case report.
    CaseReport,
    /// Expert opinion.
    ExpertOpinion,
}

/// Intended lifecycle status of a record.
///
/// Evidence-bearing records may be created as candidates. A `Finding` or an
/// `EffectEstimate` cannot reach `Validated` without evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordStatus {
    /// Candidate, not yet validated.
    #[default]
    Candidate,
    /// Validated.
    Validated,
}

/// Attested de-identification method for participant-level content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeidentificationMethod {
    /// Safe-harbor identifier removal.
    SafeHarbor,
    /// Expert-determination de-identification.
    ExpertDetermination,
    /// Aggregate-only reporting.
    Aggregate,
    /// Synthetic data.
    SyntheticData,
}

/// De-identification attestation attached to participant-level content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeidentificationAttestation {
    /// Method.
    pub method: DeidentificationMethod,
    /// Attested by.
    pub attested_by: String,
}

impl DeidentificationAttestation {
    /// Returns `true` when the attestation names a non-empty attester.
    #[must_use]
    pub fn is_attested(&self) -> bool {
        !self.attested_by.trim().is_empty()
    }
}

/// De-identification state reported for a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeidentificationState {
    /// The record carries no participant-level content.
    NotParticipantLevel,
    /// Participant-level content with a valid attestation.
    Attested,
    /// Participant-level content without a valid attestation.
    Missing,
}

/// Bounded observation window for an adverse event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationWindow {
    /// Start.
    pub start: String,
    /// End.
    pub end: String,
}

impl ObservationWindow {
    /// Returns `true` when both bounds are present.
    #[must_use]
    pub fn is_bounded(&self) -> bool {
        !self.start.trim().is_empty() && !self.end.trim().is_empty()
    }
}

/// Effect measure family, which fixes the null (no-effect) value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectMeasure {
    /// Ratio measure whose null value is one, such as a risk or odds ratio.
    Ratio,
    /// Difference measure whose null value is zero, such as a mean difference.
    Difference,
}

impl EffectMeasure {
    /// Returns the null (no-effect) value for the measure family.
    #[must_use]
    pub const fn null_value(self) -> f64 {
        match self {
            Self::Ratio => 1.0,
            Self::Difference => 0.0,
        }
    }
}

/// Effect estimate reported by a study.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectEstimate {
    /// Measure.
    pub measure: EffectMeasure,
    /// Point.
    pub point: f64,
    /// Confidence interval, as `(low, high)`, when reported.
    pub interval: Option<(f64, f64)>,
}

/// Named evidence scale used to render an evidence level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceScale {
    /// Oxford CEBM 2011 levels of evidence.
    OxfordCebm2011,
}

impl EvidenceScale {
    /// Resolves a scale identifier string.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier {
            "oxford-cebm-2011" => Some(Self::OxfordCebm2011),
            _ => None,
        }
    }
}

/// Result of resolving an evidence level for a named scale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceLevelResult {
    /// A resolved level under the requested scale.
    Level(String),
    /// No study design was recorded.
    Unrated,
    /// The requested scale identifier is not supported.
    UnknownScale,
}

/// A MEDICAL node record for validation, built-ins, and export assembly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MedicalNodeRecord {
    /// Node type.
    pub node_type: MedicalNodeType,
    /// External id.
    pub external_id: Option<String>,
    /// Evidence refs.
    pub evidence_refs: Vec<String>,
    /// Confidence, an epistemic weight distinct from evidence level.
    pub confidence: Option<Confidence>,
    /// Study design, the structural basis of evidence level.
    pub study_design: Option<StudyDesign>,
    /// Study references, for an `EffectEstimate` exactly one is required.
    pub study_refs: Vec<String>,
    /// Effect estimate payload when the node is an `EffectEstimate`.
    pub effect_estimate: Option<EffectEstimate>,
    /// Whether the record carries participant-level content.
    pub contains_participant_level: bool,
    /// De-identification attestation for participant-level content.
    pub deidentification: Option<DeidentificationAttestation>,
    /// Observation window when the node is an `AdverseEvent`.
    pub observation_window: Option<ObservationWindow>,
    /// Intended lifecycle status.
    pub intended_status: RecordStatus,
}

impl MedicalNodeRecord {
    /// Creates a new candidate record of the given type.
    #[must_use]
    pub fn new(node_type: MedicalNodeType) -> Self {
        Self {
            node_type,
            external_id: None,
            evidence_refs: Vec::new(),
            confidence: None,
            study_design: None,
            study_refs: Vec::new(),
            effect_estimate: None,
            contains_participant_level: false,
            deidentification: None,
            observation_window: None,
            intended_status: RecordStatus::Candidate,
        }
    }

    /// Sets the external id.
    #[must_use]
    pub fn with_external_id(mut self, external_id: impl Into<String>) -> Self {
        self.external_id = Some(external_id.into());
        self
    }

    /// Adds an evidence reference.
    #[must_use]
    pub fn with_evidence_ref(mut self, evidence_ref: impl Into<String>) -> Self {
        self.evidence_refs.push(evidence_ref.into());
        self
    }

    /// Sets the confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the study design.
    #[must_use]
    pub fn with_study_design(mut self, design: StudyDesign) -> Self {
        self.study_design = Some(design);
        self
    }

    /// Adds a study reference.
    #[must_use]
    pub fn with_study_ref(mut self, study_ref: impl Into<String>) -> Self {
        self.study_refs.push(study_ref.into());
        self
    }

    /// Sets the effect estimate.
    #[must_use]
    pub fn with_effect_estimate(mut self, estimate: EffectEstimate) -> Self {
        self.effect_estimate = Some(estimate);
        self
    }

    /// Marks the record as intended to be validated.
    #[must_use]
    pub fn intended_validated(mut self) -> Self {
        self.intended_status = RecordStatus::Validated;
        self
    }

    /// Marks the record as carrying participant-level content.
    #[must_use]
    pub fn with_participant_level(
        mut self,
        attestation: Option<DeidentificationAttestation>,
    ) -> Self {
        self.contains_participant_level = true;
        self.deidentification = attestation;
        self
    }

    /// Sets the observation window.
    #[must_use]
    pub fn with_observation_window(mut self, window: ObservationWindow) -> Self {
        self.observation_window = Some(window);
        self
    }
}

/// Validation policy for MEDICAL node records.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MedicalValidationPolicy {
    /// Confidence policy applied when confidence is present.
    pub confidence_policy: ConfidencePolicy,
}

impl MedicalValidationPolicy {
    /// Strict default policy.
    ///
    /// # Panics
    ///
    /// Never panics in practice; the fixed thresholds are valid by construction.
    #[must_use]
    pub fn strict_default() -> Self {
        Self {
            confidence_policy: ConfidencePolicy::new(0.6, 0.8)
                .expect("default confidence thresholds should be valid"),
        }
    }
}

fn error_issue(code: &str, message: &str, field: &str) -> DomainValidationIssue {
    DomainValidationIssue::new(
        code,
        message,
        Some(field.to_owned()),
        DomainValidationSeverity::Error,
    )
    .expect("static issue payload should be valid")
}

/// Validates a MEDICAL node record fail-closed.
///
/// Enforced rules:
///
/// - participant-level content requires an attested de-identification status;
/// - a `Finding` or `EffectEstimate` cannot reach validated status without
///   evidence;
/// - an `EffectEstimate` references exactly one originating study;
/// - an `AdverseEvent` carries a bounded observation window;
/// - confidence, when present, is a valid probability distinct from evidence
///   level.
#[must_use]
pub fn validate_medical_node(
    record: &MedicalNodeRecord,
    _policy: &MedicalValidationPolicy,
) -> DomainValidationResult {
    let mut issues = Vec::new();

    // De-identification is a precondition, enforced rather than assumed.
    if record.contains_participant_level {
        let attested = record
            .deidentification
            .as_ref()
            .is_some_and(DeidentificationAttestation::is_attested);
        if !attested {
            issues.push(error_issue(
                "MEDICAL_DEIDENTIFICATION_REQUIRED",
                "participant-level content requires an attested de-identification status",
                "deidentification",
            ));
        }
    }

    // Evidence is required before a finding or estimate can be validated.
    let evidence_bearing = matches!(
        record.node_type,
        MedicalNodeType::Finding | MedicalNodeType::EffectEstimate
    );
    if evidence_bearing && record.intended_status == RecordStatus::Validated {
        let evidence_result =
            validate_evidence_requirement(EvidenceRequirement::Required, &record.evidence_refs);
        issues.extend(evidence_result.issues().iter().cloned());
    }

    // An effect estimate is attributed to exactly one originating study.
    if record.node_type == MedicalNodeType::EffectEstimate && record.study_refs.len() != 1 {
        issues.push(error_issue(
            "MEDICAL_EFFECT_ESTIMATE_STUDY_REF",
            "an effect estimate must reference exactly one originating study",
            "study_refs",
        ));
    }

    // Adverse events carry a bounded observation window.
    if record.node_type == MedicalNodeType::AdverseEvent {
        let bounded = record
            .observation_window
            .as_ref()
            .is_some_and(ObservationWindow::is_bounded);
        if !bounded {
            issues.push(error_issue(
                "MEDICAL_ADVERSE_EVENT_WINDOW_REQUIRED",
                "an adverse event requires a bounded observation window",
                "observation_window",
            ));
        }
    }

    if issues.is_empty() {
        DomainValidationResult::pass()
    } else {
        DomainValidationResult::fail(issues)
    }
}

/// Built-in `medical.study_design`: returns the stored study design.
#[must_use]
pub fn medical_study_design(record: &MedicalNodeRecord) -> Option<StudyDesign> {
    record.study_design
}

/// Built-in `medical.evidence_level`: renders an evidence level for a named
/// scale, an explicit unknown for an unsupported scale, or unrated when no
/// design is stored.
///
/// Evidence level is derived only from study design; it never reads confidence.
#[must_use]
pub fn medical_evidence_level(record: &MedicalNodeRecord, scale: &str) -> EvidenceLevelResult {
    let Some(scale) = EvidenceScale::from_identifier(scale) else {
        return EvidenceLevelResult::UnknownScale;
    };

    let Some(design) = record.study_design else {
        return EvidenceLevelResult::Unrated;
    };

    let level = match scale {
        EvidenceScale::OxfordCebm2011 => match design {
            StudyDesign::SystematicReview => "1",
            StudyDesign::RandomizedControlledTrial => "2",
            StudyDesign::CohortStudy => "3",
            StudyDesign::CaseControlStudy => "3",
            StudyDesign::CaseSeries => "4",
            StudyDesign::CaseReport => "4",
            StudyDesign::ExpertOpinion => "5",
        },
    };

    EvidenceLevelResult::Level(level.to_owned())
}

/// Built-in `medical.interval_contains_null`: returns whether an effect
/// estimate's interval spans its measure's null value, or `None` when the
/// interval is absent.
#[must_use]
pub fn medical_interval_contains_null(estimate: &EffectEstimate) -> Option<bool> {
    let (low, high) = estimate.interval?;
    let null = estimate.measure.null_value();
    Some(low <= null && null <= high)
}

/// Built-in `medical.deidentification_status`: reports the de-identification
/// state of a record.
#[must_use]
pub fn medical_deidentification_status(record: &MedicalNodeRecord) -> DeidentificationState {
    if !record.contains_participant_level {
        return DeidentificationState::NotParticipantLevel;
    }
    let attested = record
        .deidentification
        .as_ref()
        .is_some_and(DeidentificationAttestation::is_attested);
    if attested {
        DeidentificationState::Attested
    } else {
        DeidentificationState::Missing
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn no_node_type_labels_a_patient_or_participant() {
        // The privacy boundary: the model has no identifiable-participant type.
        let forbidden = ["Patient", "Participant", "Subject", "HumanSubject"];
        for label in forbidden {
            assert!(
                MedicalNodeType::from_label(label).is_none(),
                "unexpected participant-level node type {label}"
            );
        }
    }

    #[test]
    fn participant_level_content_requires_attestation() {
        let policy = MedicalValidationPolicy::strict_default();
        let record =
            MedicalNodeRecord::new(MedicalNodeType::Population).with_participant_level(None);

        let result = validate_medical_node(&record, &policy);
        assert!(!result.is_valid());
        assert!(
            result
                .issues()
                .iter()
                .any(|issue| issue.code == "MEDICAL_DEIDENTIFICATION_REQUIRED")
        );
    }

    #[test]
    fn attested_participant_level_content_passes() {
        let policy = MedicalValidationPolicy::strict_default();
        let record = MedicalNodeRecord::new(MedicalNodeType::Population).with_participant_level(
            Some(DeidentificationAttestation {
                method: DeidentificationMethod::SafeHarbor,
                attested_by: "steward--1".to_owned(),
            }),
        );

        assert!(validate_medical_node(&record, &policy).is_valid());
        assert_eq!(
            medical_deidentification_status(&record),
            DeidentificationState::Attested
        );
    }

    #[test]
    fn finding_cannot_be_validated_without_evidence() {
        let policy = MedicalValidationPolicy::strict_default();
        let record = MedicalNodeRecord::new(MedicalNodeType::Finding).intended_validated();

        let result = validate_medical_node(&record, &policy);
        assert!(!result.is_valid());
        assert!(
            result
                .issues()
                .iter()
                .any(|issue| issue.code == "EVIDENCE_REQUIRED")
        );
    }

    #[test]
    fn effect_estimate_requires_exactly_one_study() {
        let policy = MedicalValidationPolicy::strict_default();
        let estimate = EffectEstimate {
            measure: EffectMeasure::Ratio,
            point: 0.8,
            interval: Some((0.6, 1.05)),
        };

        let none = MedicalNodeRecord::new(MedicalNodeType::EffectEstimate)
            .with_evidence_ref("evidence--1")
            .with_effect_estimate(estimate.clone());
        assert!(
            validate_medical_node(&none, &policy)
                .issues()
                .iter()
                .any(|issue| issue.code == "MEDICAL_EFFECT_ESTIMATE_STUDY_REF")
        );

        let two = MedicalNodeRecord::new(MedicalNodeType::EffectEstimate)
            .with_evidence_ref("evidence--1")
            .with_study_ref("study--a")
            .with_study_ref("study--b")
            .with_effect_estimate(estimate.clone());
        assert!(
            validate_medical_node(&two, &policy)
                .issues()
                .iter()
                .any(|issue| issue.code == "MEDICAL_EFFECT_ESTIMATE_STUDY_REF")
        );

        let one = MedicalNodeRecord::new(MedicalNodeType::EffectEstimate)
            .with_evidence_ref("evidence--1")
            .with_study_ref("study--a")
            .with_effect_estimate(estimate);
        assert!(validate_medical_node(&one, &policy).is_valid());
    }

    #[test]
    fn adverse_event_requires_bounded_window() {
        let policy = MedicalValidationPolicy::strict_default();
        let missing =
            MedicalNodeRecord::new(MedicalNodeType::AdverseEvent).with_evidence_ref("evidence--ae");
        assert!(
            validate_medical_node(&missing, &policy)
                .issues()
                .iter()
                .any(|issue| issue.code == "MEDICAL_ADVERSE_EVENT_WINDOW_REQUIRED")
        );

        let bounded = missing.clone().with_observation_window(ObservationWindow {
            start: "2026-01-01".to_owned(),
            end: "2026-03-01".to_owned(),
        });
        assert!(validate_medical_node(&bounded, &policy).is_valid());
    }

    #[test]
    fn evidence_level_is_scale_aware_and_ignores_confidence() {
        let record = MedicalNodeRecord::new(MedicalNodeType::Study)
            .with_study_design(StudyDesign::RandomizedControlledTrial)
            .with_confidence(Confidence::new(0.99).unwrap());

        assert_eq!(
            medical_evidence_level(&record, "oxford-cebm-2011"),
            EvidenceLevelResult::Level("2".to_owned())
        );
        // An unsupported scale is an explicit unknown, not a guess.
        assert_eq!(
            medical_evidence_level(&record, "made-up-scale"),
            EvidenceLevelResult::UnknownScale
        );
        // A high confidence does not manufacture an evidence level.
        let no_design = MedicalNodeRecord::new(MedicalNodeType::Study)
            .with_confidence(Confidence::new(0.99).unwrap());
        assert_eq!(
            medical_evidence_level(&no_design, "oxford-cebm-2011"),
            EvidenceLevelResult::Unrated
        );
    }

    #[test]
    fn interval_contains_null_is_measure_aware() {
        let ratio = EffectEstimate {
            measure: EffectMeasure::Ratio,
            point: 0.9,
            interval: Some((0.7, 1.2)),
        };
        assert_eq!(medical_interval_contains_null(&ratio), Some(true));

        let ratio_significant = EffectEstimate {
            measure: EffectMeasure::Ratio,
            point: 0.7,
            interval: Some((0.5, 0.9)),
        };
        assert_eq!(
            medical_interval_contains_null(&ratio_significant),
            Some(false)
        );

        let difference = EffectEstimate {
            measure: EffectMeasure::Difference,
            point: 2.0,
            interval: Some((-1.0, 5.0)),
        };
        assert_eq!(medical_interval_contains_null(&difference), Some(true));

        let unknown = EffectEstimate {
            measure: EffectMeasure::Difference,
            point: 2.0,
            interval: None,
        };
        assert_eq!(medical_interval_contains_null(&unknown), None);
    }
}
