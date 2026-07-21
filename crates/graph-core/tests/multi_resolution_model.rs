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
    DerivationLink, DerivationLinkId, FactId, GraphError, MultiResolutionModel, ResolutionArtifact,
    ResolutionArtifactId, ResolutionLevel, ResolutionRecordRef,
};

fn artifact_id(value: &str) -> ResolutionArtifactId {
    ResolutionArtifactId::new(value).expect("test artifact ID should be valid")
}

fn derivation_link_id(value: &str) -> DerivationLinkId {
    DerivationLinkId::new(value).expect("test derivation-link ID should be valid")
}

fn fact_id(value: &str) -> FactId {
    FactId::new(value).expect("test fact ID should be valid")
}

fn tactical_ref(value: &str) -> ResolutionRecordRef {
    ResolutionRecordRef::new(ResolutionLevel::Tactical, artifact_id(value))
}

fn operational_ref(value: &str) -> ResolutionRecordRef {
    ResolutionRecordRef::new(ResolutionLevel::Operational, artifact_id(value))
}

#[test]
fn resolution_levels_are_typed_and_deterministically_ordered() {
    let model = MultiResolutionModel::new();
    let levels = model.level_metadata();

    assert_eq!(levels.len(), 3);
    assert_eq!(levels[0].level(), ResolutionLevel::Tactical);
    assert_eq!(levels[1].level(), ResolutionLevel::Operational);
    assert_eq!(levels[2].level(), ResolutionLevel::Strategic);
}

#[test]
fn derivation_links_preserve_backward_traceability_to_lower_level_sources() {
    let model = MultiResolutionModel::new()
        .register_artifact(
            ResolutionArtifact::new(
                tactical_ref("record--post-1"),
                "Observed post with account and URL context",
            )
            .expect("tactical artifact should be valid"),
        )
        .expect("tactical artifact should register")
        .register_artifact(
            ResolutionArtifact::new(
                tactical_ref("record--post-2"),
                "Second observed post in campaign context",
            )
            .expect("tactical artifact should be valid"),
        )
        .expect("second tactical artifact should register")
        .register_artifact(
            ResolutionArtifact::new(
                operational_ref("record--narrative-1"),
                "Campaign narrative summary",
            )
            .expect("operational artifact should be valid"),
        )
        .expect("operational artifact should register")
        .add_derivation_link(
            DerivationLink::new(
                derivation_link_id("derivation--narrative-1"),
                operational_ref("record--narrative-1"),
                vec![
                    tactical_ref("record--post-1"),
                    tactical_ref("record--post-2"),
                ],
                vec![fact_id("fact--source-1"), fact_id("fact--source-2")],
            )
            .expect("derivation link should be valid"),
        )
        .expect("derivation link should register");

    let derivations = model.derivation_links_for(&operational_ref("record--narrative-1"));
    assert_eq!(derivations.len(), 1);
    assert_eq!(derivations[0].supporting_sources().len(), 2);
    assert_eq!(derivations[0].provenance_fact_refs().len(), 2);
}

#[test]
fn deterministic_serialization_is_stable_independent_of_insertion_order() {
    let first = MultiResolutionModel::new()
        .register_artifact(
            ResolutionArtifact::new(
                tactical_ref("record--post-z"),
                "Observed tactical item Z before A",
            )
            .expect("artifact z should be valid"),
        )
        .expect("artifact z should register")
        .register_artifact(
            ResolutionArtifact::new(tactical_ref("record--post-a"), "Observed tactical item A")
                .expect("artifact a should be valid"),
        )
        .expect("artifact a should register");

    let second = MultiResolutionModel::new()
        .register_artifact(
            ResolutionArtifact::new(tactical_ref("record--post-a"), "Observed tactical item A")
                .expect("artifact a should be valid"),
        )
        .expect("artifact a should register")
        .register_artifact(
            ResolutionArtifact::new(
                tactical_ref("record--post-z"),
                "Observed tactical item Z before A",
            )
            .expect("artifact z should be valid"),
        )
        .expect("artifact z should register");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first model should serialize"),
        serde_json::to_string(&second).expect("second model should serialize")
    );
}

#[test]
fn invalid_resolution_transitions_and_missing_provenance_are_rejected() {
    let downward_error = MultiResolutionModel::new()
        .register_artifact(
            ResolutionArtifact::new(
                tactical_ref("record--post-1"),
                "Observed tactical source artifact",
            )
            .expect("artifact should be valid"),
        )
        .expect("artifact should register")
        .register_artifact(
            ResolutionArtifact::new(
                operational_ref("record--narrative-1"),
                "Operational narrative artifact",
            )
            .expect("artifact should be valid"),
        )
        .expect("artifact should register")
        .add_derivation_link(
            DerivationLink::new(
                derivation_link_id("derivation--downward"),
                tactical_ref("record--post-1"),
                vec![operational_ref("record--narrative-1")],
                vec![fact_id("fact--source-1")],
            )
            .expect("derivation link should be syntactically valid"),
        )
        .expect_err("downward derivation should fail");
    assert!(matches!(
        downward_error,
        GraphError::InvalidResolutionModel(_)
    ));

    let missing_provenance_error = DerivationLink::new(
        derivation_link_id("derivation--no-provenance"),
        operational_ref("record--narrative-1"),
        vec![tactical_ref("record--post-1")],
        Vec::new(),
    )
    .expect_err("missing provenance should fail");
    assert!(matches!(
        missing_provenance_error,
        GraphError::InvalidResolutionModel(_)
    ));
}
