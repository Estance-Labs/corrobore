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
use graph_core::{
    GraphError, LoadingProfile, LoadingProfileErrorCode, LoadingProfileKind, RelationshipType,
    default_crisis_investigation_profile, default_cti_investigation_profile,
    default_fimi_investigation_profile, default_generic_loading_profile, lookup_loading_profile,
};

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("acceptance relationship type should be valid")
}

fn assert_hot_labels_include(profile: &LoadingProfile, expected_labels: &[&str]) {
    for expected_label in expected_labels {
        assert!(
            profile
                .hot_labels
                .iter()
                .any(|label| label == expected_label),
            "expected hot label {expected_label} in {:?}",
            profile.hot_labels
        );
    }
}

fn assert_relationship_types_include(
    actual: &[RelationshipType],
    expected_relationship_types: &[&str],
) {
    for expected_relationship_type in expected_relationship_types {
        assert!(
            actual.contains(&rel_type(expected_relationship_type)),
            "expected relationship type {expected_relationship_type} in {:?}",
            actual
        );
    }
}

//
// Validate the public CTI loading-profile contract through the `graph_core`
// facade, not private module paths.
//
// Given the default CTI loading profile,
// when a caller inspects the externally visible profile value,
// then CTI labels and relationship policies should match the acceptance contract.
#[test]
fn cti_loading_profile_is_public_and_cti_focused() {
    let profile = default_cti_investigation_profile();

    assert_eq!(profile.kind, LoadingProfileKind::CtiInvestigation);
    assert_hot_labels_include(
        &profile,
        &[
            "ThreatActor",
            "Malware",
            "Tool",
            "Infrastructure",
            "Indicator",
            "EvidenceSpan",
        ],
    );
    assert_relationship_types_include(
        &profile.prioritized_relationship_types,
        &[
            "USES",
            "INDICATES",
            "COMMUNICATES_WITH",
            "ATTRIBUTED_TO",
            "EXPLOITS",
        ],
    );
    assert_relationship_types_include(&profile.cautious_relationship_types, &["RELATED_TO"]);
    assert_relationship_types_include(
        &profile.blocked_by_default_relationship_types,
        &["MENTIONS"],
    );
}

//
// Validate the public FIMI loading-profile contract through the `graph_core`
// facade, not private module paths.
//
// Given the default FIMI loading profile,
// when a caller inspects the externally visible profile value,
// then FIMI investigation labels and relationship policies should match the
// acceptance contract.
#[test]
fn fimi_loading_profile_is_public_and_fimi_focused() {
    let profile = default_fimi_investigation_profile();

    assert_eq!(profile.kind, LoadingProfileKind::FimiInvestigation);
    assert_hot_labels_include(
        &profile,
        &[
            "Campaign",
            "Narrative",
            "Claim",
            "Actor",
            "Account",
            "Post",
            "Source",
        ],
    );
    assert_relationship_types_include(
        &profile.prioritized_relationship_types,
        &[
            "PROMOTES",
            "AMPLIFIES",
            "MAKES_CLAIM",
            "COORDINATES_WITH",
            "SUPPORTS",
        ],
    );
    assert_relationship_types_include(
        &profile.cautious_relationship_types,
        &["TARGETS", "RELATED_TO"],
    );
    assert_relationship_types_include(
        &profile.blocked_by_default_relationship_types,
        &["MENTIONS"],
    );
}

//
// Validate the public crisis loading-profile contract through the `graph_core`
// facade, not private module paths.
//
// Given the default crisis loading profile,
// when a caller inspects the externally visible profile value,
// then crisis investigation labels and relationship policies should match the
// acceptance contract.
#[test]
fn crisis_loading_profile_is_public_and_crisis_focused() {
    let profile = default_crisis_investigation_profile();

    assert_eq!(profile.kind, LoadingProfileKind::CrisisInvestigation);
    assert_hot_labels_include(
        &profile,
        &[
            "Event",
            "Location",
            "Source",
            "Post",
            "Severity",
            "HumanitarianCategory",
        ],
    );
    assert_relationship_types_include(
        &profile.prioritized_relationship_types,
        &[
            "LOCATED_IN",
            "REPORTS",
            "AFFECTS",
            "OCCURRED_AT",
            "SUPPORTS",
        ],
    );
    assert_relationship_types_include(&profile.cautious_relationship_types, &["MENTIONS"]);
    assert_relationship_types_include(
        &profile.blocked_by_default_relationship_types,
        &["RELATED_TO"],
    );
}

//
// Validate that the generic profile is usable as a conservative public fallback.
//
// Given the default generic loading profile,
// when a caller inspects the externally visible profile value,
// then it should avoid domain-specific hot labels while preserving relationship
// policy classes.
#[test]
fn generic_loading_profile_is_public_and_conservative() {
    let profile = default_generic_loading_profile();

    assert_eq!(profile.kind, LoadingProfileKind::Generic);
    assert!(profile.hot_labels.is_empty());
    assert_relationship_types_include(&profile.prioritized_relationship_types, &["SUPPORTS"]);
    assert_relationship_types_include(
        &profile.cautious_relationship_types,
        &["RELATED_TO", "MENTIONS"],
    );
    assert!(profile.blocked_by_default_relationship_types.is_empty());
}

//
// Validate the public loading-profile registry contract through stable names.
//
// Given the stable built-in profile names,
// when callers use the public lookup function,
// then every name should resolve to the matching deterministic profile kind.
#[test]
fn loading_profile_lookup_resolves_builtin_profiles_by_stable_name() {
    let cases = [
        ("cti_investigation", LoadingProfileKind::CtiInvestigation),
        ("fimi_investigation", LoadingProfileKind::FimiInvestigation),
        (
            "crisis_investigation",
            LoadingProfileKind::CrisisInvestigation,
        ),
        ("generic", LoadingProfileKind::Generic),
    ];

    for (name, expected_kind) in cases {
        let profile = lookup_loading_profile(name)
            .expect("stable built-in profile name should resolve through public API");
        assert_eq!(profile.kind, expected_kind);
    }
}

//
// Validate the public unknown-profile error contract.
//
// Given a profile name outside the built-in registry,
// when callers use the public lookup function,
// then the error should remain typed and machine-readable through `GraphError`.
#[test]
fn loading_profile_lookup_rejects_unknown_names_through_public_error() {
    let error = lookup_loading_profile("not_a_profile")
        .expect_err("unknown profile should fail with a typed public error");

    assert!(matches!(
    error,
    GraphError::UnknownLoadingProfile(payload)
    if payload.code == LoadingProfileErrorCode::UnknownLoadingProfile
    && payload.requested_name == "not_a_profile"
    && payload.fix_hint.contains("cti_investigation")
    && payload.fix_hint.contains("fimi_investigation")
    && payload.fix_hint.contains("crisis_investigation")
    && payload.fix_hint.contains("generic")
    ));
}

//
// Validate that loading profiles are deterministic public values.
//
// Given repeated construction and lookup of the same built-in profile,
// when the resulting values are compared,
// then the values should be identical without relying on external registries,
// timestamps, randomization, or hash-map ordering.
#[test]
fn loading_profiles_are_deterministic_public_values() {
    assert_eq!(
        default_cti_investigation_profile(),
        lookup_loading_profile("cti_investigation").expect("CTI lookup should succeed")
    );
    assert_eq!(
        default_fimi_investigation_profile(),
        lookup_loading_profile("fimi_investigation").expect("FIMI lookup should succeed")
    );
    assert_eq!(
        default_crisis_investigation_profile(),
        lookup_loading_profile("crisis_investigation").expect("crisis lookup should succeed")
    );
    assert_eq!(
        default_generic_loading_profile(),
        lookup_loading_profile("generic").expect("generic lookup should succeed")
    );
}
