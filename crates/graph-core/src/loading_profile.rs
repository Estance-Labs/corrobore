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
//! Loading profile contracts for bounded graph working sets.
//!
//! A loading profile describes which graph labels and relationship types should
//! be treated as hot, prioritized, cautious, or blocked by default when a future
//! working-set manager expands around seed nodes.
//!
//! Implementation boundary for issue 38:
//!
//! - Declare the public loading-profile model.
//! - Declare typed lookup error payloads.
//! - Provide deterministic built-in profile constructors.
//! - Provide deterministic built-in profile lookup by stable user-facing name.
//! - Do not implement graph expansion, traversal, prefetch, eviction, semantic
//!   search, or storage behavior here.

use serde::{Deserialize, Serialize};

use crate::{GraphError, LabelSet, RelationshipType};

/// Built-in loading profile families known by the graph-core contract.
///
///
/// - Keep profile selection typed instead of passing unstructured strings through
///   graph expansion internals.
/// - Allow CTI, FIMI, crisis, and generic workloads to evolve independently.
/// - Keep this enum focused on built-in defaults only; custom runtime policies
///   belong to later working-set manager or configuration issues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoadingProfileKind {
    /// Cyber threat intelligence investigation profile.
    CtiInvestigation,

    /// Foreign information manipulation and interference investigation profile.
    FimiInvestigation,

    /// Crisis or emergency investigation profile.
    CrisisInvestigation,

    /// Domain-neutral graph loading profile.
    Generic,
}

/// Declarative profile used by future working-set expansion policy.
///
///
/// - Represent hot node labels independently from relationship policy.
/// - Distinguish prioritized relationships from cautious relationships.
/// - Represent relationship types that should not be expanded by default.
/// - Preserve deterministic ordered collections so public tests can compare
///   default profile content without depending on hash-map ordering.
///
/// This structure is only data contract. It does not execute traversal, estimate
/// costs, inspect node degree, or mutate a working set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadingProfile {
    /// Built-in profile family represented by this profile value.
    pub kind: LoadingProfileKind,

    /// Node labels that should become hot early for the selected investigation flow.
    pub hot_labels: LabelSet,

    /// Relationship types that should be preferred during controlled expansion.
    pub prioritized_relationship_types: Vec<RelationshipType>,

    /// Relationship types that may be useful but should be expanded with guards.
    pub cautious_relationship_types: Vec<RelationshipType>,

    /// Relationship types that should not be expanded unless explicitly requested.
    pub blocked_by_default_relationship_types: Vec<RelationshipType>,
}

/// Stable machine-readable loading-profile error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadingProfileErrorCode {
    /// The requested profile name is not known by the built-in profile registry.
    #[serde(rename = "UNKNOWN_LOADING_PROFILE")]
    UnknownLoadingProfile,
}

/// Error payload returned when a loading profile lookup cannot resolve a name.
///
///
/// - Make unknown profile failures explicit and matchable.
/// - Preserve the requested name for diagnostics.
/// - Provide a stable fix hint for agents and public API clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownLoadingProfile {
    /// Stable machine-readable error code.
    pub code: LoadingProfileErrorCode,

    /// Profile name requested by the caller.
    pub requested_name: String,

    /// Human-readable remediation hint.
    pub fix_hint: String,
}

/// Build the default CTI investigation profile.
///
/// The profile prioritizes CTI investigation paths without implementing traversal
/// itself. Broad relationship types remain cautious or blocked by default so a
/// future working-set manager can avoid accidental supernode expansion.
pub fn default_cti_investigation_profile() -> LoadingProfile {
    LoadingProfile {
        // Kind.
        kind: LoadingProfileKind::CtiInvestigation,
        // Hot labels.
        hot_labels: labels(&[
            "ThreatActor",
            "Malware",
            "Tool",
            "Infrastructure",
            "Indicator",
            "EvidenceSpan",
        ]),
        // Prioritized relationship types.
        prioritized_relationship_types: relationship_types(&[
            "USES",
            "INDICATES",
            "COMMUNICATES_WITH",
            "ATTRIBUTED_TO",
            "EXPLOITS",
        ]),
        // Cautious relationship types.
        cautious_relationship_types: relationship_types(&["RELATED_TO"]),
        // Blocked by default relationship types.
        blocked_by_default_relationship_types: relationship_types(&["MENTIONS"]),
    }
}

/// Build the default FIMI investigation profile.
///
/// The profile prioritizes campaign, narrative, claim, actor, account, post, and
/// source workflows while keeping broad targeting and relatedness edges guarded.
pub fn default_fimi_investigation_profile() -> LoadingProfile {
    LoadingProfile {
        // Kind.
        kind: LoadingProfileKind::FimiInvestigation,
        // Hot labels.
        hot_labels: labels(&[
            "Campaign",
            "Narrative",
            "Claim",
            "Actor",
            "Account",
            "Post",
            "Source",
        ]),
        // Prioritized relationship types.
        prioritized_relationship_types: relationship_types(&[
            "PROMOTES",
            "AMPLIFIES",
            "MAKES_CLAIM",
            "COORDINATES_WITH",
            "SUPPORTS",
        ]),
        // Cautious relationship types.
        cautious_relationship_types: relationship_types(&["TARGETS", "RELATED_TO"]),
        // Blocked by default relationship types.
        blocked_by_default_relationship_types: relationship_types(&["MENTIONS"]),
    }
}

/// Build the default crisis investigation profile.
///
/// The profile prioritizes event, location, source, post, severity, and
/// humanitarian-category workflows while keeping broad relationship edges guarded.
pub fn default_crisis_investigation_profile() -> LoadingProfile {
    LoadingProfile {
        // Kind.
        kind: LoadingProfileKind::CrisisInvestigation,
        // Hot labels.
        hot_labels: labels(&[
            "Event",
            "Location",
            "Source",
            "Post",
            "Severity",
            "HumanitarianCategory",
        ]),
        // Prioritized relationship types.
        prioritized_relationship_types: relationship_types(&[
            "LOCATED_IN",
            "REPORTS",
            "AFFECTS",
            "OCCURRED_AT",
            "SUPPORTS",
        ]),
        // Cautious relationship types.
        cautious_relationship_types: relationship_types(&["MENTIONS"]),
        // Blocked by default relationship types.
        blocked_by_default_relationship_types: relationship_types(&["RELATED_TO"]),
    }
}

/// Build the default generic loading profile.
///
/// The generic profile avoids domain-specific hot labels while still preserving
/// explicit relationship policy classes for callers that need a conservative
/// default.
pub fn default_generic_loading_profile() -> LoadingProfile {
    LoadingProfile {
        // Kind.
        kind: LoadingProfileKind::Generic,
        // Hot labels.
        hot_labels: Vec::new(),
        // Prioritized relationship types.
        prioritized_relationship_types: relationship_types(&["SUPPORTS"]),
        // Cautious relationship types.
        cautious_relationship_types: relationship_types(&["RELATED_TO", "MENTIONS"]),
        // Blocked by default relationship types.
        blocked_by_default_relationship_types: Vec::new(),
    }
}

/// Look up a built-in loading profile by user-facing name.
///
/// Accepted names are intentionally stable snake_case identifiers:
///
/// - `cti_investigation`
/// - `fimi_investigation`
/// - `crisis_investigation`
/// - `generic`
///
/// Unknown names fail explicitly with `GraphError::UnknownLoadingProfile`.
pub fn lookup_loading_profile(name: &str) -> Result<LoadingProfile, GraphError> {
    match name {
        "cti_investigation" => Ok(default_cti_investigation_profile()),
        "fimi_investigation" => Ok(default_fimi_investigation_profile()),
        "crisis_investigation" => Ok(default_crisis_investigation_profile()),
        "generic" => Ok(default_generic_loading_profile()),
        _ => Err(unknown_loading_profile(name)),
    }
}

fn labels(values: &[&str]) -> LabelSet {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn relationship_types(values: &[&str]) -> Vec<RelationshipType> {
    values
        .iter()
        .map(|value| {
            RelationshipType::new(*value).expect("built-in relationship type must be valid")
        })
        .collect()
}

fn unknown_loading_profile(name: &str) -> GraphError {
    GraphError::UnknownLoadingProfile(UnknownLoadingProfile {
        code: LoadingProfileErrorCode::UnknownLoadingProfile,
        requested_name: name.to_owned(),
        fix_hint:
            "Use one of: cti_investigation, fimi_investigation, crisis_investigation, generic."
                .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    fn assert_labels_include(profile: &LoadingProfile, expected_labels: &[&str]) {
        for expected_label in expected_labels {
            assert!(
                profile
                    .hot_labels
                    .iter()
                    .any(|label| label == expected_label),
                "expected hot label {expected_label} to be present in {:?}",
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
                "expected relationship type {expected_relationship_type} to be present in {:?}",
                actual
            );
        }
    }

    //
    // Verify that the loading profile contract can represent distinct expansion
    // classes before any default profile data or traversal behavior exists.
    //
    // Given a manually built loading profile,
    // when hot labels, prioritized relationships, cautious relationships, and
    // blocked-by-default relationships are inspected,
    // then each policy class should remain distinct and deterministic.
    #[test]
    fn loading_profile_represents_distinct_relationship_policies() {
        let profile = LoadingProfile {
            kind: LoadingProfileKind::Generic,
            hot_labels: vec!["Seed".to_owned(), "Evidence".to_owned()],
            prioritized_relationship_types: vec![rel_type("SUPPORTS")],
            cautious_relationship_types: vec![rel_type("RELATED_TO")],
            blocked_by_default_relationship_types: vec![rel_type("MENTIONS")],
        };

        assert_eq!(profile.kind, LoadingProfileKind::Generic);
        assert_eq!(
            profile.hot_labels,
            vec!["Seed".to_owned(), "Evidence".to_owned()]
        );
        assert_relationship_types_include(&profile.prioritized_relationship_types, &["SUPPORTS"]);
        assert_relationship_types_include(&profile.cautious_relationship_types, &["RELATED_TO"]);
        assert_relationship_types_include(
            &profile.blocked_by_default_relationship_types,
            &["MENTIONS"],
        );
        assert_ne!(
            profile.prioritized_relationship_types,
            profile.cautious_relationship_types
        );
        assert_ne!(
            profile.cautious_relationship_types,
            profile.blocked_by_default_relationship_types
        );
    }

    //
    // Verify that the default CTI profile captures the CTI acceptance
    // contract before implementation fills the profile data.
    //
    // Given the default CTI investigation profile,
    // when callers inspect its hot labels and relationship classes,
    // then CTI-relevant labels and relationships should be prioritized while broad
    // relationships remain cautious or blocked by default.
    #[test]
    fn default_cti_profile_prioritizes_cti_investigation_workflows() {
        let profile = default_cti_investigation_profile();

        assert_eq!(profile.kind, LoadingProfileKind::CtiInvestigation);
        assert_labels_include(
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
    // Verify that the default FIMI profile captures the FIMI acceptance
    // contract before implementation fills the profile data.
    //
    // Given the default FIMI investigation profile,
    // when callers inspect its hot labels and relationship classes,
    // then campaign, narrative, claim, actor, account, post, and source workflows
    // should be prioritized while broad relationships remain guarded.
    #[test]
    fn default_fimi_profile_prioritizes_fimi_investigation_workflows() {
        let profile = default_fimi_investigation_profile();

        assert_eq!(profile.kind, LoadingProfileKind::FimiInvestigation);
        assert_labels_include(
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
    // Verify that the default crisis profile captures the crisis
    // acceptance contract before implementation fills the profile data.
    //
    // Given the default crisis investigation profile,
    // when callers inspect its hot labels and relationship classes,
    // then event, location, source, post, severity, and humanitarian-category
    // workflows should be prioritized while broad relationships remain guarded.
    #[test]
    fn default_crisis_profile_prioritizes_crisis_investigation_workflows() {
        let profile = default_crisis_investigation_profile();

        assert_eq!(profile.kind, LoadingProfileKind::CrisisInvestigation);
        assert_labels_include(
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
    // Verify that the generic profile remains conservative and domain-neutral.
    //
    // Given the default generic profile,
    // when callers inspect its content,
    // then it should not smuggle CTI, FIMI, or crisis-specific assumptions into
    // graph-core while still preserving explicit relationship policy classes.
    #[test]
    fn default_generic_profile_is_conservative_and_domain_neutral() {
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
    // Verify that the built-in profile registry accepts stable user-facing names.
    //
    // Given stable loading profile names,
    // when callers request profiles through the lookup function,
    // then each name should resolve to the matching built-in profile kind.
    #[test]
    fn lookup_loading_profile_accepts_stable_builtin_names() {
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
                .expect("known loading profile name should resolve successfully");
            assert_eq!(profile.kind, expected_kind);
        }
    }

    //
    // Verify that unknown loading profile names fail explicitly and remain
    // matchable through the public graph error model.
    //
    // Given an unknown loading profile name,
    // when callers request it through the lookup function,
    // then lookup should fail with `GraphError::UnknownLoadingProfile` and a
    // stable machine-readable error code.
    #[test]
    fn lookup_loading_profile_rejects_unknown_names_with_typed_error() {
        let error = lookup_loading_profile("unknown-profile")
            .expect_err("unknown loading profile should fail explicitly");

        assert!(matches!(
        error,
        GraphError::UnknownLoadingProfile(payload)
        if payload.code == LoadingProfileErrorCode::UnknownLoadingProfile
        && payload.requested_name == "unknown-profile"
        && payload.fix_hint.contains("cti_investigation")
        && payload.fix_hint.contains("fimi_investigation")
        && payload.fix_hint.contains("crisis_investigation")
        && payload.fix_hint.contains("generic")
        ));
    }

    //
    // Verify that default profiles are deterministic values that can be compared
    // directly by public tests and callers.
    //
    // Given two profiles built from the same default constructor,
    // when the values are compared,
    // then they should be identical without relying on hash-map ordering,
    // external registries, timestamps, or randomization.
    #[test]
    fn default_loading_profiles_are_deterministic() {
        assert_eq!(
            default_cti_investigation_profile(),
            default_cti_investigation_profile()
        );
        assert_eq!(
            default_fimi_investigation_profile(),
            default_fimi_investigation_profile()
        );
        assert_eq!(
            default_crisis_investigation_profile(),
            default_crisis_investigation_profile()
        );
        assert_eq!(
            default_generic_loading_profile(),
            default_generic_loading_profile()
        );
    }
}
