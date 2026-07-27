// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Cross-domain composition contract.
//!
//! A clinical trial is simultaneously a MEDICAL `Study` and a RESEARCH `Study`.
//! It must round-trip as one record carrying one type assertion per pack, with
//! each pack validating only its own required properties, rather than as two
//! parallel nodes joined by an identity relationship.

use domain_common::{DomainTypeAssertion, MultiTypingPolicy, validate_multi_typing};
use domain_medical::{
    MedicalNodeRecord, MedicalNodeType, MedicalValidationPolicy, StudyDesign,
    medical_provider_api_v1, validate_medical_node,
};
use domain_provider_abi::{DomainProviderBuffer, DomainProviderSlice, ProviderMetadata, STATUS_OK};
use domain_research::{
    ResearchNodeRecord, ResearchNodeType, research_provider_api_v1, validate_research_node,
};

/// One trial, expressed as one node carrying both packs' typing.
struct ComposedTrial {
    base_kind: &'static str,
    assertions: Vec<DomainTypeAssertion>,
    medical: MedicalNodeRecord,
    research: ResearchNodeRecord,
}

fn composed_trial() -> ComposedTrial {
    ComposedTrial {
        base_kind: "study",
        assertions: vec![
            DomainTypeAssertion::new("medical", "ClinicalTrial").unwrap(),
            DomainTypeAssertion::new("research", "Study").unwrap(),
        ],
        medical: MedicalNodeRecord::new(MedicalNodeType::ClinicalTrial)
            .with_external_id("nct--0001")
            .with_evidence_ref("evidence--1")
            .with_study_design(StudyDesign::RandomizedControlledTrial),
        research: ResearchNodeRecord::new(ResearchNodeType::Study)
            .with_external_id("nct--0001")
            .with_evidence_ref("evidence--1"),
    }
}

#[test]
fn a_trial_is_one_node_with_one_assertion_per_pack() {
    let trial = composed_trial();

    let result = validate_multi_typing(
        trial.base_kind,
        &trial.assertions,
        MultiTypingPolicy::default_bounded(),
    );
    assert!(result.is_valid(), "a trial must compose on one node");

    // Both packs accept the same record, each validating only its own rules.
    assert!(
        validate_medical_node(&trial.medical, &MedicalValidationPolicy::strict_default())
            .is_valid()
    );
    assert!(validate_research_node(&trial.research).is_valid());

    // One shared external identity, not two parallel nodes.
    assert_eq!(trial.medical.external_id, trial.research.external_id);
}

#[test]
fn each_pack_validates_only_its_own_required_properties() {
    // A RESEARCH Claim rule must not be imposed on a MEDICAL record, and a
    // MEDICAL adverse-event rule must not be imposed on a RESEARCH record.
    let research_claim = ResearchNodeRecord::new(ResearchNodeType::Claim);
    assert!(
        !validate_research_node(&research_claim).is_valid(),
        "RESEARCH enforces claim attribution"
    );

    // The same conceptual node typed only for MEDICAL is unaffected by that
    // rule, because attribution is not a MEDICAL requirement.
    let medical_study = MedicalNodeRecord::new(MedicalNodeType::Study);
    assert!(
        validate_medical_node(&medical_study, &MedicalValidationPolicy::strict_default())
            .is_valid(),
        "MEDICAL does not enforce RESEARCH claim attribution"
    );
}

#[test]
fn one_pack_absence_does_not_invalidate_the_other() {
    // A record written under RESEARCH alone stays valid without any MEDICAL
    // assertion, and vice versa.
    let research_only = validate_multi_typing(
        "study",
        &[DomainTypeAssertion::new("research", "Study").unwrap()],
        MultiTypingPolicy::default_bounded(),
    );
    assert!(research_only.is_valid());

    let medical_only = validate_multi_typing(
        "study",
        &[DomainTypeAssertion::new("medical", "ClinicalTrial").unwrap()],
        MultiTypingPolicy::default_bounded(),
    );
    assert!(medical_only.is_valid());
}

/// Reads a provider's declared domain through its own uniquely-named accessor.
fn declared_domain(api: *const domain_provider_abi::DomainProviderApiV1) -> String {
    // SAFETY: the accessor returns a valid pointer to a static table.
    let api = unsafe { &*api };
    let metadata_fn = api.metadata.expect("metadata callback must be present");
    let free_fn = api
        .free_buffer
        .expect("free_buffer callback must be present");

    let mut output = DomainProviderBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
    };
    // SAFETY: null host context per v1 contract and a valid output pointer.
    let status = unsafe {
        metadata_fn(
            DomainProviderSlice {
                ptr: std::ptr::null(),
                len: 0,
            },
            &mut output,
        )
    };
    assert_eq!(status, STATUS_OK);
    // SAFETY: the provider owns this allocation until free_buffer is called.
    let bytes = unsafe { std::slice::from_raw_parts(output.ptr, output.len).to_vec() };
    // SAFETY: output ownership returns to the provider free callback.
    unsafe { free_fn(output) };

    let metadata: ProviderMetadata =
        serde_json::from_slice(&bytes).expect("metadata must be valid JSON");
    metadata.domain.as_str().to_owned()
}

#[test]
fn statically_linked_packs_resolve_to_their_own_provider() {
    // Regression guard for the duplicate-symbol defect this file first exposed.
    //
    // The shared `dlsym` entry point `corrobore_domain_provider_get_api_v1` now
    // lives only in each pack's `-provider` cdylib, never in its rlib, so this
    // binary can link both packs at all. Merely compiling this test is half the
    // assertion: with the symbol back in the rlibs, linking fails outright on
    // GNU/Linux and silently collapses onto one definition on macOS.
    //
    // The rest asserts each uniquely-named accessor reaches its own provider.
    let medical = medical_provider_api_v1();
    let research = research_provider_api_v1();

    assert_ne!(
        medical as usize, research as usize,
        "each pack must expose its own function table"
    );
    assert_eq!(declared_domain(medical), "medical");
    assert_eq!(declared_domain(research), "research");
}

#[test]
fn a_pack_cannot_assert_two_types_on_one_node() {
    // Composition places one assertion per pack. A second MEDICAL assertion is
    // rejected with an issue naming the provider, so one pack cannot silently
    // overwrite a peer's typing.
    let result = validate_multi_typing(
        "study",
        &[
            DomainTypeAssertion::new("medical", "ClinicalTrial").unwrap(),
            DomainTypeAssertion::new("medical", "Study").unwrap(),
            DomainTypeAssertion::new("research", "Study").unwrap(),
        ],
        MultiTypingPolicy::default_bounded(),
    );
    assert!(!result.is_valid());
    let issue = result
        .issues()
        .iter()
        .find(|issue| issue.code == "MULTI_TYPING_DUPLICATE_PROVIDER")
        .expect("duplicate provider must be reported");
    assert!(issue.message.contains("medical"));
}
