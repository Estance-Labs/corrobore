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
    BranchCreationInput, BranchId, BranchStatus, GraphError, HypothesisWorldModel, WorldId,
};

fn world_id(value: &str) -> WorldId {
    WorldId::new(value).expect("test world ID should be valid")
}

fn branch_id(value: &str) -> BranchId {
    BranchId::new(value).expect("test branch ID should be valid")
}

fn fact(value: &str) -> graph_core::FactId {
    graph_core::FactId::new(value).expect("test fact ID should be valid")
}

//
// Verify world and branch identifiers are typed and deterministically ordered.
#[test]
fn world_and_branch_identifiers_are_typed_and_lexically_ordered() {
    let mut worlds = [world_id("world--zeta"), world_id("world--alpha")];
    worlds.sort();
    assert_eq!(worlds[0].as_str(), "world--alpha");
    assert_eq!(worlds[1].as_str(), "world--zeta");

    let mut branches = [branch_id("branch--2"), branch_id("branch--1")];
    branches.sort();
    assert_eq!(branches[0].as_str(), "branch--1");
    assert_eq!(branches[1].as_str(), "branch--2");
}

//
// Verify multiple worlds can coexist over one shared immutable base.
#[test]
fn multiple_worlds_coexist_over_shared_immutable_base_facts() {
    let model =
        HypothesisWorldModel::new(vec![fact("fact--confirmed-a"), fact("fact--confirmed-b")])
            .expect("world model should be valid")
            .create_world(
                world_id("world--h1"),
                "Actor A operates campaign C".to_owned(),
            )
            .expect("world h1 should be created")
            .create_world(
                world_id("world--h2"),
                "Actor B operates campaign C".to_owned(),
            )
            .expect("world h2 should be created");

    assert_eq!(model.base_facts().len(), 2);
    assert_eq!(model.worlds().len(), 2);
    assert_eq!(
        model
            .world(&world_id("world--h1"))
            .expect("world h1 should exist")
            .base_facts()
            .len(),
        2
    );
    assert_eq!(
        model
            .world(&world_id("world--h2"))
            .expect("world h2 should exist")
            .base_facts()
            .len(),
        2
    );
}

//
// Verify branch creation captures origin, lineage, and deterministic descriptors.
#[test]
fn branch_creation_records_lineage_and_is_deterministic() {
    let first = HypothesisWorldModel::new(vec![fact("fact--confirmed-a")])
        .expect("world model should be valid")
        .create_world(world_id("world--h1"), "Actor A hypothesis".to_owned())
        .expect("world should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--root"), "root hypothesis".to_owned()),
        )
        .expect("root branch should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--child"), "child hypothesis".to_owned())
                .with_parent_branch_id(branch_id("branch--root")),
        )
        .expect("child branch should be created");

    let second = HypothesisWorldModel::new(vec![fact("fact--confirmed-a")])
        .expect("world model should be valid")
        .create_world(world_id("world--h1"), "Actor A hypothesis".to_owned())
        .expect("world should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--root"), "root hypothesis".to_owned()),
        )
        .expect("root branch should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--child"), "child hypothesis".to_owned())
                .with_parent_branch_id(branch_id("branch--root")),
        )
        .expect("child branch should be created");

    let child = first
        .world(&world_id("world--h1"))
        .and_then(|world| world.branch(&branch_id("branch--child")))
        .expect("child branch should exist");
    assert_eq!(child.parent_branch_id(), Some(&branch_id("branch--root")));
    assert_eq!(child.lineage(), &[branch_id("branch--root")]);
    assert_eq!(child.status(), BranchStatus::Active);
    assert_eq!(first, second);
}

//
// Verify invalid lineage and duplicate branch creation return typed errors.
#[test]
fn invalid_lineage_and_duplicate_branch_creation_return_typed_errors() {
    let model = HypothesisWorldModel::new(vec![fact("fact--confirmed-a")])
        .expect("world model should be valid")
        .create_world(world_id("world--h1"), "Actor A hypothesis".to_owned())
        .expect("world should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--root"), "root hypothesis".to_owned()),
        )
        .expect("root branch should be created");

    let missing_parent = model
        .clone()
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--x"), "invalid lineage".to_owned())
                .with_parent_branch_id(branch_id("branch--missing")),
        )
        .expect_err("missing parent should fail");
    assert!(matches!(
        missing_parent,
        GraphError::InvalidWorldBranchModel(_)
    ));

    let duplicate = model
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--root"), "duplicate".to_owned()),
        )
        .expect_err("duplicate branch should fail");
    assert!(matches!(duplicate, GraphError::InvalidWorldBranchModel(_)));
}

//
// Verify branch scope cannot mutate immutable shared base facts.
#[test]
fn base_fact_mutation_attempts_from_branch_scope_are_rejected() {
    let model = HypothesisWorldModel::new(vec![fact("fact--confirmed-a")])
        .expect("world model should be valid")
        .create_world(world_id("world--h1"), "Actor A hypothesis".to_owned())
        .expect("world should be created")
        .create_branch(
            &world_id("world--h1"),
            BranchCreationInput::new(branch_id("branch--root"), "root hypothesis".to_owned()),
        )
        .expect("root branch should be created");

    let error = model
        .attempt_branch_base_fact_mutation(
            &world_id("world--h1"),
            &branch_id("branch--root"),
            vec![fact("fact--new")],
        )
        .expect_err("base fact mutation should fail");
    assert!(matches!(error, GraphError::InvalidWorldBranchModel(_)));
}
