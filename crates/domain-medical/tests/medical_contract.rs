// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Consumer-level contract for the MEDICAL public surface.

use domain_common::{DomainTypeAssertion, MultiTypingPolicy, validate_multi_typing};
use domain_medical::{
    DeidentificationState, EffectEstimate, EffectMeasure, EvidenceLevelResult, MedicalNodeRecord,
    MedicalNodeType, MedicalValidationPolicy, StudyDesign, medical_deidentification_status,
    medical_evidence_level, medical_interval_contains_null, medical_study_design,
    validate_medical_node,
};

#[test]
fn domain_types_are_constructible_and_labelled() {
    assert_eq!(MedicalNodeType::Study.as_str(), "Study");
    assert_eq!(
        MedicalNodeType::from_label("EffectEstimate"),
        Some(MedicalNodeType::EffectEstimate)
    );
    assert_eq!(MedicalNodeType::from_label("Patient"), None);
}

#[test]
fn validated_finding_with_evidence_passes_and_reports_design() {
    let policy = MedicalValidationPolicy::strict_default();
    let record = MedicalNodeRecord::new(MedicalNodeType::Finding)
        .with_evidence_ref("evidence--1")
        .with_study_design(StudyDesign::RandomizedControlledTrial)
        .intended_validated();

    assert!(validate_medical_node(&record, &policy).is_valid());
    assert_eq!(
        medical_study_design(&record),
        Some(StudyDesign::RandomizedControlledTrial)
    );
    assert_eq!(
        medical_evidence_level(&record, "oxford-cebm-2011"),
        EvidenceLevelResult::Level("2".to_owned())
    );
    assert_eq!(
        medical_deidentification_status(&record),
        DeidentificationState::NotParticipantLevel
    );
}

#[test]
fn interval_null_check_is_available_to_consumers() {
    let estimate = EffectEstimate {
        measure: EffectMeasure::Ratio,
        point: 0.82,
        interval: Some((0.70, 0.96)),
    };
    assert_eq!(medical_interval_contains_null(&estimate), Some(false));
}

#[test]
fn a_trial_composes_as_one_node_across_medical_and_research() {
    // The composition boundary from the PRDs: one node, one assertion per pack.
    let result = validate_multi_typing(
        "study",
        &[
            DomainTypeAssertion::new("medical", "ClinicalTrial").unwrap(),
            DomainTypeAssertion::new("research", "Study").unwrap(),
        ],
        MultiTypingPolicy::default_bounded(),
    );
    assert!(result.is_valid());
}
