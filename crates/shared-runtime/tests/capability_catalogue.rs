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
//! One catalogue, several adapters, and no protocol shape in the definition.
#![allow(clippy::unwrap_used)]

use shared_runtime::{Capability, CapabilityAuthorization, CapabilityCatalogue, CapabilityEffect};

fn catalogue() -> CapabilityCatalogue {
    CapabilityCatalogue::v1()
}

//
// The catalogue is the single definition: identities are unique and ordered,
// and no mutation can be reached with session authorization alone.
#[test]
fn the_catalogue_defines_each_capability_once_and_validates_its_own_invariants() {
    let catalogue = catalogue();
    catalogue.validate().unwrap();

    assert_eq!(catalogue.catalogue_version(), "capabilities-v1");
    assert!(!catalogue.capabilities().is_empty());
    let ids: Vec<&str> = catalogue
        .capabilities()
        .iter()
        .map(Capability::id)
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted, "identities are unique and ordered");
    for capability in catalogue.capabilities() {
        assert!(!capability.summary().trim().is_empty());
        assert!(!capability.runtime_operation().trim().is_empty());
        if capability.writes() {
            assert_ne!(
                capability.authorization(),
                CapabilityAuthorization::Session,
                "{} writes and must need more than a session",
                capability.id()
            );
        }
    }
    assert_eq!(
        catalogue
            .capability("memory.consolidate")
            .unwrap()
            .authorization(),
        CapabilityAuthorization::OperatorApproval
    );
    assert!(catalogue.capability("absent").is_none());
}

//
// One definition serves several adapters, and an adapter nobody has written
// yet already has its projection: adding one changes no capability.
#[test]
fn one_definition_projects_onto_every_adapter_including_a_new_one() {
    let catalogue = catalogue();
    let mcp = catalogue.adapter_view("mcp");
    let http = catalogue.adapter_view("http");
    let plugin = catalogue.adapter_view("agent-plugin");
    let unwritten = catalogue.adapter_view("webmcp");

    assert_eq!(http.len(), catalogue.capabilities().len(), "http sees all");
    assert!(mcp.len() < http.len(), "a restricted capability stays out");
    assert!(
        mcp.iter()
            .all(|capability| capability.id() != "cypher.write"),
        "raw mutation query execution is not an agent tool"
    );
    assert_eq!(
        plugin
            .iter()
            .map(|capability| capability.id())
            .collect::<Vec<_>>(),
        mcp.iter()
            .map(|capability| capability.id())
            .collect::<Vec<_>>(),
        "two adapters with no restriction between them see the same set"
    );
    assert_eq!(
        unwritten
            .iter()
            .map(|capability| capability.id())
            .collect::<Vec<_>>(),
        mcp.iter()
            .map(|capability| capability.id())
            .collect::<Vec<_>>(),
        "an adapter that does not exist yet already has a projection"
    );

    // Effect and authorization come from the definition, so two adapters can
    // never disagree about whether a capability writes.
    for capability in &mcp {
        let defined = catalogue.capability(capability.id()).unwrap();
        assert_eq!(capability.effect(), defined.effect());
        assert_eq!(capability.authorization(), defined.authorization());
    }
}

//
// The definition carries no protocol shape: there is no route, tool name,
// envelope or protocol version anywhere in it, so a protocol version bump
// cannot reach the evidence graph.
#[test]
fn the_definition_carries_no_protocol_shape_or_version() {
    let catalogue = catalogue();
    let serialized = serde_json::to_string(&catalogue).unwrap().to_lowercase();

    for shape in [
        "mcp",
        "a2a",
        "jsonrpc",
        "json-rpc",
        "http",
        "route",
        "endpoint",
        "tool_name",
        "protocol",
    ] {
        // `restricted_to` names adapters, which is an exposure decision rather
        // than a protocol shape, so it is checked separately below.
        let occurrences = serialized.matches(shape).count();
        let allowed = serialized.matches(&format!("\"{shape}\"")).count();
        assert_eq!(
            occurrences, allowed,
            "{shape} may only appear as an adapter name in restricted_to"
        );
    }
    assert!(
        catalogue
            .capabilities()
            .iter()
            .all(|capability| capability.runtime_operation().contains('.')),
        "a capability dispatches to a runtime operation, not to a protocol route"
    );
}

//
// The committed artifact every adapter reads is exactly this catalogue, so a
// cross-language adapter cannot drift from the definition silently.
#[test]
fn the_committed_artifact_matches_the_catalogue_exactly() {
    let committed: serde_json::Value = serde_json::from_str(include_str!(
        "../../../compatibility/capabilities/v1/catalogue.json"
    ))
    .expect("the committed catalogue is valid JSON");

    assert_eq!(
        committed,
        serde_json::to_value(catalogue()).unwrap(),
        "regenerate compatibility/capabilities/v1/catalogue.json from the catalogue"
    );
}

//
// A refused definition is refused at construction, so a malformed capability
// never reaches an adapter.
#[test]
fn a_malformed_capability_is_refused_at_construction() {
    for (id, summary, operation) in [
        ("", "summary", "operation"),
        ("id", "  ", "operation"),
        ("id", "summary", ""),
    ] {
        assert!(
            Capability::new(
                id,
                summary,
                CapabilityEffect::Read,
                CapabilityAuthorization::Session,
                operation,
            )
            .is_err()
        );
    }
}
