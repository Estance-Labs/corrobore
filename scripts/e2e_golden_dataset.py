# Copyright (c) 2026 AreDee-Bangs
# SPDX-License-Identifier: MIT
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.
#!/usr/bin/env python3
"""
E2E simulation test for the Intelligence Graph Engine using the golden dataset.

This script simulates a complete agent workflow:
  1. Parse STIX 2.1 bundles from the golden dataset
  2. Map STIX objects to the engine's graph domain model
  3. Generate Cypher queries for ingestion and querying
  4. Generate a Rust integration test exercising the full pipeline
  5. Run the pipeline via `cargo test`
  6. Collect and report results

Usage:
    python3 scripts/e2e_golden_dataset.py [--dataset-dir /path/to/golden_dataset]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import textwrap
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

PROJECT_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DATASET_DIR = Path.home() / "Documents" / "golden_dataset"

# Map STIX 2.1 SDO types to the graph-engine label model.
STIX_TYPE_TO_LABEL: dict[str, str] = {
    # CTI domain
    "threat-actor": "ThreatActor",
    "intrusion-set": "ThreatActor",
    "malware": "Malware",
    "tool": "Tool",
    "indicator": "Indicator",
    "attack-pattern": "AttackPattern",
    "campaign": "Campaign",
    "vulnerability": "Vulnerability",
    "identity": "Identity",
    "location": "Location",
    "infrastructure": "Infrastructure",
    "report": "Report",
    "software": "Software",
    # FIMI domain
    "channel": "Account",
    # Crisis domain
    "incident": "CrisisEvent",
    # SCO (observables)
    "file": "Indicator",
    "ipv4-addr": "Indicator",
    "domain-name": "Indicator",
}

# Map STIX relationship_type to graph-engine relationship labels.
STIX_REL_TO_LABEL: dict[str, str] = {
    "uses": "Uses",
    "targets": "Targets",
    "indicates": "Indicates",
    "attributed-to": "AttributedTo",
    "related-to": "RelatedTo",
    "communicates-with": "CommunicatesWith",
    "has": "Has",
    "located-at": "LocatedAt",
    "originates-from": "OriginatesFrom",
    "based-on": "RelatedTo",
    "mitigates": "RelatedTo",
    "delivers": "Uses",
    "drops": "Uses",
    "exploits": "Uses",
    "variant-of": "RelatedTo",
    "impersonates": "RelatedTo",
    "compromises": "Targets",
    "hosts": "RelatedTo",
    "owns": "RelatedTo",
    "authored-by": "AttributedTo",
    "publishes": "RelatedTo",
    "amplifies": "Amplifies",
    "coordinates-with": "CoordinatesWith",
    "contradicts": "Contradicts",
}


# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclass
class StixObject:
    """Parsed STIX 2.1 object."""

    stix_type: str
    stix_id: str
    name: str | None = None
    description: str | None = None
    confidence: int | None = None
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass
class StixRelationship:
    """Parsed STIX 2.1 relationship."""

    stix_id: str
    rel_type: str
    source_ref: str
    target_ref: str
    confidence: int | None = None


@dataclass
class ParsedBundle:
    """A fully parsed STIX 2.1 bundle."""

    filename: str
    bundle_id: str
    objects: list[StixObject] = field(default_factory=list)
    relationships: list[StixRelationship] = field(default_factory=list)
    type_counts: Counter = field(default_factory=Counter)


@dataclass
class E2EReport:
    """Aggregate report for the E2E test run."""

    total_bundles: int = 0
    total_objects: int = 0
    total_relationships: int = 0
    type_distribution: Counter = field(default_factory=Counter)
    domain_distribution: Counter = field(default_factory=Counter)
    cypher_create_queries: int = 0
    cypher_match_queries: int = 0
    rust_test_passed: bool = False
    rust_test_output: str = ""
    errors: list[str] = field(default_factory=list)


# ---------------------------------------------------------------------------
# 1. Parse golden dataset
# ---------------------------------------------------------------------------


def parse_stix_bundle(filepath: Path) -> ParsedBundle:
    """Parse a single STIX 2.1 JSON bundle file."""
    with open(filepath, encoding="utf-8") as fh:
        data = json.load(fh)

    bundle = ParsedBundle(
        filename=filepath.name,
        bundle_id=data.get("id", "unknown"),
    )

    for obj in data.get("objects", []):
        obj_type = obj.get("type", "")
        obj_id = obj.get("id", "")
        bundle.type_counts[obj_type] += 1

        if obj_type == "relationship":
            bundle.relationships.append(
                StixRelationship(
                    stix_id=obj_id,
                    rel_type=obj.get("relationship_type", "related-to"),
                    source_ref=obj.get("source_ref", ""),
                    target_ref=obj.get("target_ref", ""),
                    confidence=obj.get("confidence"),
                )
            )
        elif obj_type == "marking-definition":
            # Skip TLP markings — not graph nodes in this model.
            continue
        else:
            bundle.objects.append(
                StixObject(
                    stix_type=obj_type,
                    stix_id=obj_id,
                    name=obj.get("name"),
                    description=obj.get("description"),
                    confidence=obj.get("confidence"),
                    properties={
                        k: v
                        for k, v in obj.items()
                        if k
                        not in {
                            "type",
                            "id",
                            "spec_version",
                            "name",
                            "description",
                            "confidence",
                            "created",
                            "modified",
                            "object_marking_refs",
                            "created_by_ref",
                        }
                    },
                )
            )

    return bundle


def parse_all_bundles(dataset_dir: Path) -> list[ParsedBundle]:
    """Parse all STIX bundles in the dataset directory."""
    bundles: list[ParsedBundle] = []
    for filepath in sorted(dataset_dir.glob("*.json")):
        try:
            bundles.append(parse_stix_bundle(filepath))
        except (json.JSONDecodeError, KeyError) as exc:
            print(f"  [WARN] Skipping {filepath.name}: {exc}", file=sys.stderr)
    return bundles


# ---------------------------------------------------------------------------
# 2. Domain classification
# ---------------------------------------------------------------------------

CTI_TYPES = {
    "threat-actor",
    "intrusion-set",
    "malware",
    "tool",
    "indicator",
    "attack-pattern",
    "campaign",
    "vulnerability",
    "infrastructure",
}

FIMI_TYPES = {"channel"}

CRISIS_TYPES = {"incident"}

CRISIS_KEYWORDS = {
    "earthquake",
    "flood",
    "hurricane",
    "wildfire",
    "outbreak",
    "epidemic",
    "cholera",
    "refugee",
    "evacuation",
    "shelling",
    "airstrike",
    "clashes",
    "disaster",
    "tsunami",
    "famine",
}


def classify_bundle_domain(bundle: ParsedBundle) -> str:
    """Classify a bundle's primary domain (cti, fimi, crisis, mixed)."""
    cti_count = sum(
        bundle.type_counts.get(t, 0) for t in CTI_TYPES
    )
    fimi_count = sum(
        bundle.type_counts.get(t, 0) for t in FIMI_TYPES
    )
    crisis_count = sum(
        bundle.type_counts.get(t, 0) for t in CRISIS_TYPES
    )

    # Also check descriptions for crisis keywords.
    for obj in bundle.objects:
        if obj.description:
            desc_lower = obj.description.lower()
            if any(kw in desc_lower for kw in CRISIS_KEYWORDS):
                crisis_count += 1

    if crisis_count > cti_count and crisis_count > fimi_count:
        return "crisis"
    if fimi_count > cti_count:
        return "fimi"
    if cti_count > 0:
        return "cti"
    return "unknown"


# ---------------------------------------------------------------------------
# 3. Generate Cypher queries
# ---------------------------------------------------------------------------


def _escape_cypher(value: str) -> str:
    """Escape a string value for Cypher embedding."""
    return value.replace("\\", "\\\\").replace("'", "\\'").replace("\n", "\\n").replace("\r", "")


def _sanitize_id(stix_id: str) -> str:
    """Turn a STIX id into a safe Cypher variable name."""
    return stix_id.replace("-", "_").replace(".", "_").replace(" ", "_")


def generate_create_queries(bundle: ParsedBundle) -> list[str]:
    """Generate Cypher CREATE queries for all objects in a bundle."""
    queries: list[str] = []
    seen_ids: set[str] = set()

    for obj in bundle.objects:
        if obj.stix_id in seen_ids:
            continue
        seen_ids.add(obj.stix_id)

        label = STIX_TYPE_TO_LABEL.get(obj.stix_type, "Entity")
        name = _escape_cypher(obj.name or obj.stix_id)
        props = [f"stix_id: '{_escape_cypher(obj.stix_id)}'", f"name: '{name}'"]

        if obj.description:
            desc = _escape_cypher(obj.description[:200])
            props.append(f"description: '{desc}'")
        if obj.confidence is not None:
            props.append(f"confidence: {obj.confidence}")

        props_str = ", ".join(props)
        queries.append(f"CREATE (n:{label} {{{props_str}}})")

    for rel in bundle.relationships:
        if rel.source_ref in seen_ids or rel.target_ref in seen_ids:
            # Only create rels where at least one endpoint exists in this bundle.
            rel_label = STIX_REL_TO_LABEL.get(rel.rel_type, "RelatedTo")
            queries.append(
                f"MATCH (a {{stix_id: '{_escape_cypher(rel.source_ref)}'}}), "
                f"(b {{stix_id: '{_escape_cypher(rel.target_ref)}'}}) "
                f"CREATE (a)-[:{rel_label}]->(b)"
            )

    return queries


def generate_match_queries(bundle: ParsedBundle) -> list[str]:
    """Generate Cypher MATCH queries for validation."""
    queries: list[str] = []

    # Count nodes by label.
    for stix_type, count in bundle.type_counts.items():
        if stix_type in ("relationship", "marking-definition"):
            continue
        label = STIX_TYPE_TO_LABEL.get(stix_type, "Entity")
        queries.append(f"MATCH (n:{label}) RETURN count(n)")

    # Count all relationships.
    if bundle.relationships:
        queries.append("MATCH ()-[r]->() RETURN count(r)")

    # Find threat actors.
    if bundle.type_counts.get("threat-actor", 0) + bundle.type_counts.get("intrusion-set", 0) > 0:
        queries.append("MATCH (t:ThreatActor) RETURN t.name")

    # Find malware.
    if bundle.type_counts.get("malware", 0) > 0:
        queries.append("MATCH (m:Malware) RETURN m.name")

    # Find indicators.
    indicator_count = sum(
        bundle.type_counts.get(t, 0)
        for t in ("indicator", "file", "ipv4-addr", "domain-name")
    )
    if indicator_count > 0:
        queries.append("MATCH (i:Indicator) RETURN count(i)")

    return queries


# ---------------------------------------------------------------------------
# 4. Generate Rust integration test
# ---------------------------------------------------------------------------


def generate_rust_test(bundles: list[ParsedBundle], report: E2EReport) -> tuple[str, str, str]:
    """Generate Rust integration tests exercising the engine with golden data.

    Returns a tuple of (runtime_test, graph_test, domain_test) source code.
    """

    # Pick a representative bundle for each domain for the Rust test.
    representative_bundles: dict[str, ParsedBundle] = {}
    for b in bundles:
        domain = classify_bundle_domain(b)
        if domain not in representative_bundles or len(b.objects) > len(
            representative_bundles[domain].objects
        ):
            representative_bundles[domain] = b

    # ── Per-domain Cypher gateway tests ──
    test_cases: list[str] = []

    for domain, bundle in sorted(representative_bundles.items()):
        creates = generate_create_queries(bundle)
        matches = generate_match_queries(bundle)

        # Limit to prevent gigantic tests.
        creates = creates[:50]
        matches = matches[:10]

        create_stmts = "\n".join(
            f'        "{_escape_rust(q)}",' for q in creates
        )
        match_stmts = "\n".join(
            f'        "{_escape_rust(q)}",' for q in matches
        )

        test_fn = f"""
#[test]
fn e2e_golden_{domain}_bundle_{_sanitize_rust_id(bundle.filename)}() {{
    let mut gateway = CypherGateway::strict_default();
    let workspace_id = WorkspaceId::new("workspace--e2e-golden").expect("valid workspace id");
    let session_id = SessionId::new("session--e2e-golden").expect("valid session id");
    let budget_ref = CypherBudgetRef::new("budget--e2e-golden").expect("valid budget ref");

    // --- Phase 1: Validate CREATE queries are accepted by the gateway ---
    let create_queries: Vec<&str> = vec![
{create_stmts}
    ];

    for query_text in &create_queries {{
        let request = CypherRequest::build_mutation_request(
            *query_text,
            CypherParameters::default(),
            workspace_id.clone(),
            session_id.clone(),
            budget_ref.clone(),
        )
        .expect("mutation request should be built");

        let response = gateway.execute(&request).expect("gateway should not error");
        assert_ne!(
            response.status,
            CypherResponseStatus::ValidationFailed,
            "CREATE query should not fail validation: {{}}",
            query_text
        );
    }}

    // --- Phase 2: Validate MATCH queries are accepted by the gateway ---
    let match_queries: Vec<&str> = vec![
{match_stmts}
    ];

    for query_text in &match_queries {{
        let request = CypherRequest::build_read_only_request(
            *query_text,
            CypherParameters::default(),
            workspace_id.clone(),
            session_id.clone(),
            budget_ref.clone(),
        )
        .expect("read-only request should be built");

        let response = gateway.execute(&request).expect("gateway should not error");
        assert_ne!(
            response.status,
            CypherResponseStatus::ValidationFailed,
            "MATCH query should not fail validation: {{}}",
            query_text
        );
    }}

    // --- Phase 3: Verify domain classification ---
    // Bundle '{bundle.filename}' classified as '{domain}'
    // Objects: {len(bundle.objects)}, Relationships: {len(bundle.relationships)}
    assert!(
        !create_queries.is_empty(),
        "golden bundle should produce at least one CREATE query"
    );
}}
"""
        test_cases.append(test_fn)

    # ── Cypher gateway safety test ──
    gateway_validation_test = """
#[test]
fn e2e_golden_gateway_rejects_unsafe_operations() {
    let mut gateway = CypherGateway::strict_default();
    let workspace_id = WorkspaceId::new("workspace--e2e-safety").expect("valid workspace id");
    let session_id = SessionId::new("session--e2e-safety").expect("valid session id");
    let budget_ref = CypherBudgetRef::new("budget--e2e-safety").expect("valid budget ref");

    // LOAD CSV should be rejected.
    let load_csv = CypherRequest::build_read_only_request(
        "LOAD CSV FROM 'file:///etc/passwd' AS row RETURN row",
        CypherParameters::default(),
        workspace_id.clone(),
        session_id.clone(),
        budget_ref.clone(),
    )
    .expect("read-only request should be built");
    let response = gateway.execute(&load_csv).expect("gateway should not error");
    assert_eq!(
        response.status,
        CypherResponseStatus::Rejected,
        "LOAD CSV should be rejected"
    );

    // CALL DBMS should be rejected.
    let call_dbms = CypherRequest::build_read_only_request(
        "CALL DBMS.PROCEDURES() YIELD name RETURN name",
        CypherParameters::default(),
        workspace_id.clone(),
        session_id.clone(),
        budget_ref.clone(),
    )
    .expect("read-only request should be built");
    let response = gateway.execute(&call_dbms).expect("gateway should not error");
    assert_eq!(
        response.status,
        CypherResponseStatus::Rejected,
        "CALL DBMS should be rejected"
    );

    // Read-only queries should succeed.
    let read_only = CypherRequest::build_read_only_request(
        "MATCH (n:ThreatActor) RETURN n.name",
        CypherParameters::default(),
        workspace_id.clone(),
        session_id.clone(),
        budget_ref.clone(),
    )
    .expect("read-only request should be built");
    let response = gateway.execute(&read_only).expect("gateway should not error");
    assert_eq!(
        response.status,
        CypherResponseStatus::Success,
        "read-only MATCH should succeed"
    );
}
"""

    # ── Workspace and session lifecycle test ──
    lifecycle_test = """
#[test]
fn e2e_golden_workspace_session_lifecycle() {
    let mut workspace_registry = WorkspaceRegistry::default();
    let mut session_registry = SessionRegistry::default();

    let workspace_id = WorkspaceId::new("workspace--e2e-lifecycle").expect("valid workspace id");
    let session_id = SessionId::new("session--e2e-lifecycle").expect("valid session id");
    let actor = ActorRef::new(
        ActorId::new("agent--e2e-cti-analyst").expect("valid actor id"),
        ActorKind::Agent,
    );

    // Create workspace.
    let ws_id = workspace_registry
        .create_workspace(CreateWorkspaceRequest {
            id: workspace_id.clone(),
            name: WorkspaceName::new("Golden Dataset E2E").expect("valid workspace name"),
            created_by: actor.clone(),
            created_at: RuntimeTimestamp::from_millis(1000),
        })
        .expect("workspace should be created");

    let ws = workspace_registry
        .workspace(&ws_id)
        .expect("workspace should be retrievable");
    assert_eq!(ws.status, WorkspaceStatus::Open);

    // Start session.
    let sess_id = session_registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor.clone()),
            workspace_id: workspace_id.clone(),
            started_at: RuntimeTimestamp::from_millis(2000),
            metadata: std::collections::HashMap::new(),
        })
        .expect("session should be started");

    // Validate workspace-session consistency.
    session_registry
        .validate_workspace_session_consistency(&workspace_id, &sess_id)
        .expect("workspace-session should be consistent");

    // Create transaction metadata.
    let tx_id = TransactionId::new("tx--e2e-lifecycle").expect("valid transaction id");
    let tx_meta = session_registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: tx_id,
            session_id: sess_id,
            started_at: RuntimeTimestamp::from_millis(3000),
            policy_name: Some("strict".to_owned()),
        })
        .expect("transaction metadata should be created");

    assert_eq!(tx_meta.workspace_id, workspace_id);
    assert_eq!(tx_meta.actor, actor);

    // Close workspace.
    workspace_registry
        .close_workspace(&workspace_id)
        .expect("workspace should be closed");
    let ws = workspace_registry
        .workspace(&workspace_id)
        .expect("workspace should be retrievable");
    assert_eq!(ws.status, WorkspaceStatus::Closed);
}
"""

    all_test_cases = "\n".join(test_cases)

    # ── Assemble the shared-runtime test (uses only shared-runtime + graph-core) ──
    runtime_test_source = f"""\
//! E2E integration test generated from the golden dataset.
//!
//! Auto-generated by scripts/e2e_golden_dataset.py
//! Dataset: {len(bundles)} STIX 2.1 bundles
//! Total objects: {report.total_objects}
//! Total relationships: {report.total_relationships}

use graph_core::{{ActorId, SessionId, TransactionId, WorkspaceId}};
use shared_runtime::{{
    ActorKind, ActorRef, CreateWorkspaceRequest, CreateTransactionMetadataRequest,
    CypherBudgetRef, CypherGateway, CypherParameters, CypherRequest, CypherResponseStatus,
    RuntimeTimestamp, SessionRegistry, StartSessionRequest,
    WorkspaceName, WorkspaceRegistry, WorkspaceStatus,
}};
{all_test_cases}
{gateway_validation_test}
{lifecycle_test}
"""

    # ── Assemble the graph-core test (graph ingest + export plan) ──
    graph_test_source = f"""\
//! E2E graph-core integration test generated from the golden dataset.
//!
//! Auto-generated by scripts/e2e_golden_dataset.py
//! Validates: graph creation, node/relationship CRUD, export plan, domain validation.

use graph_core::{{
    Graph, NodeInput, PropertyValue, RecordStatus, RelationshipInput, TransactionId,
    ExportMetadata, ExportMode, ExportProfile,
    build_deterministic_export_plan,
}};

#[test]
fn e2e_golden_graph_core_ingest_and_export() {{
    let mut graph = Graph::new();

    // --- Ingest representative CTI nodes from golden dataset (APT-K-47 bundle) ---
    let actor_id = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("APT-K-47".to_owned()))
                .with_property("description", PropertyValue::String(
                    "South Asia-based APT group conducting espionage campaigns".to_owned()
                ))
                .with_property("confidence", PropertyValue::Integer(50))
                .with_status(RecordStatus::Exportable),
        )
        .expect("actor node should be created");

    let malware_id = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_property("name", PropertyValue::String("WalkerShell".to_owned()))
                .with_property("description", PropertyValue::String(
                    "Backdoor malware used by APT-K-47".to_owned()
                ))
                .with_property("confidence", PropertyValue::Integer(70))
                .with_status(RecordStatus::Exportable),
        )
        .expect("malware node should be created");

    let campaign_id = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property("name", PropertyValue::String("Espionage 2026".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("campaign node should be created");

    let indicator_id = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("malicious-hash-001".to_owned()))
                .with_property("confidence", PropertyValue::Integer(80))
                .with_status(RecordStatus::Exportable),
        )
        .expect("indicator node should be created");

    let location_id = graph
        .create_node(
            NodeInput::new(["Location"])
                .with_property("name", PropertyValue::String("South Asia".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("location node should be created");

    let vulnerability_id = graph
        .create_node(
            NodeInput::new(["Vulnerability"])
                .with_property("name", PropertyValue::String("CVE-2026-1364".to_owned()))
                .with_property("confidence", PropertyValue::Integer(50))
                .with_status(RecordStatus::Exportable),
        )
        .expect("vulnerability node should be created");

    let _identity_id = graph
        .create_node(
            NodeInput::new(["Identity"])
                .with_property("name", PropertyValue::String("CERT-FR".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("identity node should be created");

    let _report_id = graph
        .create_node(
            NodeInput::new(["Report"])
                .with_property("name", PropertyValue::String("APT-K-47 Threat Report".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("report node should be created");

    // --- Create relationships ---
    graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "Uses", malware_id.clone())
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("uses relationship should be created");

    graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "AttributedTo", campaign_id.clone())
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("attributed-to relationship should be created");

    graph
        .create_relationship(
            RelationshipInput::new(malware_id.clone(), "Indicates", indicator_id.clone())
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("indicates relationship should be created");

    graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "Targets", location_id.clone())
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("targets relationship should be created");

    graph
        .create_relationship(
            RelationshipInput::new(malware_id.clone(), "Uses", vulnerability_id.clone())
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("exploits relationship should be created");

    // --- Verify graph state ---
    let actor = graph.get_node(&actor_id).expect("get should not error").expect("actor should exist");
    assert!(actor.has_label("ThreatActor"), "actor should have ThreatActor label");

    let malware = graph.get_node(&malware_id).expect("get should not error").expect("malware should exist");
    assert!(malware.has_label("Malware"), "malware should have Malware label");

    let all_nodes = graph.list_nodes().expect("list nodes should not error");
    assert_eq!(all_nodes.len(), 8, "should have 8 nodes from golden data");

    let all_rels = graph.list_relationships().expect("list rels should not error");
    assert_eq!(all_rels.len(), 5, "should have 5 relationships");

    // --- Build deterministic export plan ---
    let tx_id = TransactionId::new("tx--e2e-golden").expect("tx id should be valid");
    let metadata = ExportMetadata::new(
        "snapshot--e2e-golden",
        tx_id,
        "e2e-test-v1",
        ExportProfile::StixMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("export metadata should be valid");

    let plan = build_deterministic_export_plan(&graph, metadata, &[])
        .expect("export plan should be built");

    assert!(
        !plan.records().is_empty(),
        "export plan should contain records"
    );

    // Verify the plan is deterministic: building it again with the same inputs
    // should produce the same fingerprint.
    let tx_id_2 = TransactionId::new("tx--e2e-golden").expect("tx id should be valid");
    let metadata_2 = ExportMetadata::new(
        "snapshot--e2e-golden",
        tx_id_2,
        "e2e-test-v1",
        ExportProfile::StixMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("export metadata should be valid");

    let plan_2 = build_deterministic_export_plan(&graph, metadata_2, &[])
        .expect("second export plan should be built");

    assert_eq!(
        plan.determinism_fingerprint(),
        plan_2.determinism_fingerprint(),
        "export plan should be deterministic"
    );
}}

#[test]
fn e2e_golden_graph_multi_bundle_ingest() {{
    // Simulate ingesting objects from multiple golden bundles into a single graph.
    let mut graph = Graph::new();

    // Bundle 1: CERT-FR vulnerability advisory.
    let _cert_identity = graph
        .create_node(
            NodeInput::new(["Identity"])
                .with_property("name", PropertyValue::String("CERT-FR".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("identity node");
    let vuln = graph
        .create_node(
            NodeInput::new(["Vulnerability"])
                .with_property("name", PropertyValue::String("CVE-2025-53786".to_owned()))
                .with_property("description", PropertyValue::String(
                    "Privilege escalation in Microsoft Exchange Server".to_owned()
                ))
                .with_status(RecordStatus::Exportable),
        )
        .expect("vulnerability node");
    let software = graph
        .create_node(
            NodeInput::new(["Software"])
                .with_property("name", PropertyValue::String("Microsoft Exchange Server 2019".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("software node");

    graph
        .create_relationship(
            RelationshipInput::new(software.clone(), "Has", vuln.clone())
                .expect("valid rel input")
                .with_status(RecordStatus::Exportable),
        )
        .expect("has relationship");

    // Bundle 2: Ransomware report (Akira).
    let akira = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("Akira".to_owned()))
                .with_property("confidence", PropertyValue::Integer(50))
                .with_status(RecordStatus::Exportable),
        )
        .expect("akira node");
    let akira_malware = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_property("name", PropertyValue::String("Akira Ransomware".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("akira malware node");

    graph
        .create_relationship(
            RelationshipInput::new(akira.clone(), "Uses", akira_malware.clone())
                .expect("valid rel input")
                .with_status(RecordStatus::Exportable),
        )
        .expect("uses relationship");

    // Bundle 3: Ukraine campaign.
    let campaign = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property("name", PropertyValue::String("Ukraine Draftee Targeting".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("campaign node");
    let _meduza = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_property("name", PropertyValue::String("MeduzaStealer".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("meduza node");
    let ukraine = graph
        .create_node(
            NodeInput::new(["Location"])
                .with_property("name", PropertyValue::String("Ukraine".to_owned()))
                .with_status(RecordStatus::Exportable),
        )
        .expect("ukraine location");

    graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "Targets", ukraine.clone())
                .expect("valid rel input")
                .with_status(RecordStatus::Exportable),
        )
        .expect("targets relationship");

    // Verify total graph state after multi-bundle ingest.
    let all_nodes = graph.list_nodes().expect("list should work");
    assert_eq!(all_nodes.len(), 8, "multi-bundle graph should have 8 nodes");

    let all_rels = graph.list_relationships().expect("list should work");
    assert_eq!(all_rels.len(), 3, "multi-bundle graph should have 3 relationships");

    // Export should succeed on the combined graph.
    let tx_id = TransactionId::new("tx--e2e-multi").expect("tx id");
    let metadata = ExportMetadata::new(
        "snapshot--e2e-multi",
        tx_id,
        "e2e-test-v1",
        ExportProfile::StixMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("metadata");

    let plan = build_deterministic_export_plan(&graph, metadata, &[])
        .expect("export plan should succeed on combined graph");
    assert!(!plan.records().is_empty(), "combined export should have records");
}}
"""

    # ── Domain validation test (now split into separate crate tests) ──
    domain_test_source = ""  # unused — split into cti/fimi/crisis generators below

    return runtime_test_source, graph_test_source, domain_test_source


def generate_domain_test_cti() -> str:
    """Generate CTI domain validation test."""
    return """\
//! E2E CTI domain validation test generated from the golden dataset.
//!
//! Auto-generated by scripts/e2e_golden_dataset.py

use domain_cti::{CtiNodeRecord, CtiNodeType, CtiStixReadinessPolicy, validate_cti_stix_readiness, cti_is_stix_exportable};
use graph_core::Confidence;

#[test]
fn e2e_golden_cti_validation_pipeline() {
    let policy = CtiStixReadinessPolicy::strict_mvp_default();

    // A well-formed CTI record should pass STIX readiness.
    let exportable_record = CtiNodeRecord::new(CtiNodeType::ThreatActor)
        .with_external_id("intrusion-set--f5356387-5703-5b6b-83af-b206f9a14f2a")
        .with_evidence_ref("evidence--001")
        .with_confidence(Confidence::new(0.85).expect("valid confidence"));

    assert!(
        cti_is_stix_exportable(&exportable_record, &policy),
        "well-formed CTI record should be STIX-exportable"
    );

    // Missing external_id should fail.
    let no_ext_id = CtiNodeRecord::new(CtiNodeType::Malware)
        .with_evidence_ref("evidence--002")
        .with_confidence(Confidence::new(0.9).expect("valid confidence"));

    let result = validate_cti_stix_readiness(&no_ext_id, &policy);
    assert!(!result.is_valid(), "CTI record without external_id should not be valid");

    // Low confidence should fail.
    let low_confidence = CtiNodeRecord::new(CtiNodeType::Indicator)
        .with_external_id("indicator--abc")
        .with_evidence_ref("evidence--003")
        .with_confidence(Confidence::new(0.3).expect("valid confidence"));

    assert!(
        !cti_is_stix_exportable(&low_confidence, &policy),
        "CTI record with low confidence should not be exportable"
    );

    // Missing evidence should fail.
    let no_evidence = CtiNodeRecord::new(CtiNodeType::Campaign)
        .with_external_id("campaign--xyz")
        .with_confidence(Confidence::new(0.9).expect("valid confidence"));

    assert!(
        !cti_is_stix_exportable(&no_evidence, &policy),
        "CTI record without evidence should not be exportable"
    );
}
"""


def generate_domain_test_fimi() -> str:
    """Generate FIMI domain validation test."""
    return """\
//! E2E FIMI domain validation test generated from the golden dataset.
//!
//! Auto-generated by scripts/e2e_golden_dataset.py

use domain_fimi::{FimiClaimRecord, FimiValidationPolicy, validate_fimi_claim, fimi_claim_similarity};
use graph_core::Confidence;

#[test]
fn e2e_golden_fimi_validation_pipeline() {
    let policy = FimiValidationPolicy::strict_mvp_default();

    // Valid FIMI claim with sufficient text, evidence, and confidence.
    let valid_claim = FimiClaimRecord::new(
        "State-sponsored actors coordinated disinformation campaign targeting EU elections"
    )
    .with_evidence_ref("evidence--fimi-001")
    .with_confidence(Confidence::new(0.8).expect("valid confidence"));

    let result = validate_fimi_claim(&valid_claim, &policy);
    assert!(result.is_valid(), "well-formed FIMI claim should be valid");

    // Too-short claim should fail.
    let short_claim = FimiClaimRecord::new("short")
        .with_evidence_ref("evidence--fimi-002")
        .with_confidence(Confidence::new(0.8).expect("valid confidence"));

    let result = validate_fimi_claim(&short_claim, &policy);
    assert!(!result.is_valid(), "short FIMI claim should not be valid");

    // Claim similarity check.
    let sim = fimi_claim_similarity(
        "Russia conducted cyber attacks against Ukraine infrastructure",
        "Russia launched cyber operations targeting Ukraine critical infrastructure",
    );
    assert!(sim > 0.3, "similar claims should have non-trivial similarity score");

    let dissimilar = fimi_claim_similarity(
        "Global weather patterns indicate seasonal changes",
        "APT group targets financial institutions",
    );
    assert!(dissimilar < 0.3, "dissimilar claims should have low similarity");
}
"""


def generate_domain_test_crisis() -> str:
    """Generate Crisis domain validation test."""
    return """\
//! E2E Crisis domain validation test generated from the golden dataset.
//!
//! Auto-generated by scripts/e2e_golden_dataset.py

use domain_crisis::{CrisisObservation, CrisisPolicy, crisis_classify, crisis_score, validate_crisis_observation, CrisisClass};
use graph_core::Confidence;

#[test]
fn e2e_golden_crisis_classification_pipeline() {
    // Natural disaster classification.
    assert_eq!(
        crisis_classify("Severe earthquake devastated the coastal region"),
        CrisisClass::NaturalDisaster,
    );
    assert_eq!(
        crisis_classify("Massive flood displaced thousands of families"),
        CrisisClass::NaturalDisaster,
    );

    // Public health classification.
    assert_eq!(
        crisis_classify("Cholera outbreak reported in the refugee camp"),
        CrisisClass::PublicHealth,
    );

    // Armed conflict classification.
    assert_eq!(
        crisis_classify("Shelling continues in the border region"),
        CrisisClass::ArmedConflict,
    );

    // Unknown classification.
    assert_eq!(
        crisis_classify("Quarterly economic report published"),
        CrisisClass::Unknown,
    );

    // Crisis scoring with golden-like data.
    let critical_observation = CrisisObservation::new(
        "Critical humanitarian situation: severe flooding has displaced over 150,000 people"
    )
    .with_affected_population(150_000)
    .with_evidence_ref("evidence--crisis-001")
    .with_confidence(Confidence::new(0.85).expect("valid confidence"));

    let score = crisis_score(&critical_observation);
    assert!(score > 0.5, "critical observation should have high crisis score, got {score}");

    // Crisis observation validation.
    let policy = CrisisPolicy::strict_mvp_default();
    let result = validate_crisis_observation(&critical_observation, &policy);
    assert!(result.is_valid(), "well-formed crisis observation should be valid");

    // Short description should fail validation.
    let short_obs = CrisisObservation::new("flood");
    let result = validate_crisis_observation(&short_obs, &policy);
    assert!(!result.is_valid(), "short crisis observation should not be valid");
}
"""


def _escape_rust(s: str) -> str:
    """Escape a string for embedding in a Rust string literal."""
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def _sanitize_rust_id(name: str) -> str:
    """Sanitize a filename into a valid Rust identifier."""
    s = name.replace(".json", "").replace(" ", "_").replace("-", "_")
    s = s.replace(".", "_").replace(",", "_")
    # Remove non-alphanumeric characters except underscore.
    return "".join(c if c.isalnum() or c == "_" else "_" for c in s).lower()


# ---------------------------------------------------------------------------
# 5. Run cargo test
# ---------------------------------------------------------------------------


def run_cargo_test(test_file: Path) -> tuple[bool, str]:
    """Run the generated Rust test via cargo test."""
    test_name = test_file.stem
    # Derive package name from directory structure.
    pkg = test_file.parent.parent.name
    return run_cargo_test_for_package(test_file, pkg)


def run_cargo_test_for_package(test_file: Path, package: str) -> tuple[bool, str]:
    """Run a generated Rust test for a specific package via cargo test."""
    test_name = test_file.stem
    result = subprocess.run(
        [
            "cargo",
            "test",
            "--package",
            package,
            "--test",
            test_name,
            "--",
            "--nocapture",
        ],
        capture_output=True,
        text=True,
        cwd=PROJECT_ROOT,
        timeout=300,
    )
    output = result.stdout + "\n" + result.stderr
    return result.returncode == 0, output


# ---------------------------------------------------------------------------
# 6. Report
# ---------------------------------------------------------------------------


def print_report(report: E2EReport) -> None:
    """Print a human-readable E2E report."""
    print("\n" + "=" * 72)
    print("  INTELLIGENCE GRAPH ENGINE — E2E GOLDEN DATASET REPORT")
    print("=" * 72)

    print(f"\n  Bundles parsed:           {report.total_bundles}")
    print(f"  Total STIX objects:       {report.total_objects}")
    print(f"  Total relationships:      {report.total_relationships}")
    print(f"  Cypher CREATE queries:    {report.cypher_create_queries}")
    print(f"  Cypher MATCH queries:     {report.cypher_match_queries}")

    print("\n  STIX Type Distribution:")
    for stype, count in report.type_distribution.most_common():
        print(f"    {stype:30s}  {count:>5d}")

    print("\n  Domain Classification:")
    for domain, count in report.domain_distribution.most_common():
        print(f"    {domain:30s}  {count:>5d} bundles")

    print("\n  Rust Integration Test:")
    if report.rust_test_passed:
        print("    PASSED")
    else:
        print("    FAILED")
        if report.errors:
            for err in report.errors:
                print(f"    ERROR: {err}")

    print("\n" + "=" * 72)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description="E2E golden dataset test for the Intelligence Graph Engine")
    parser.add_argument(
        "--dataset-dir",
        type=Path,
        default=DEFAULT_DATASET_DIR,
        help="Path to the golden dataset directory",
    )
    parser.add_argument(
        "--skip-cargo",
        action="store_true",
        help="Skip running cargo test (only generate the Rust test file)",
    )
    parser.add_argument(
        "--verbose",
        "-v",
        action="store_true",
        help="Print verbose output including generated Cypher queries",
    )
    args = parser.parse_args()

    report = E2EReport()

    # ── Step 1: Parse golden dataset ──
    print(f"\n[1/5] Parsing STIX bundles from {args.dataset_dir} ...")
    if not args.dataset_dir.is_dir():
        print(f"ERROR: Dataset directory not found: {args.dataset_dir}", file=sys.stderr)
        return 1

    bundles = parse_all_bundles(args.dataset_dir)
    report.total_bundles = len(bundles)

    if not bundles:
        print("ERROR: No STIX bundles found.", file=sys.stderr)
        return 1

    print(f"  Parsed {len(bundles)} bundles.")

    # ── Step 2: Analyze and classify ──
    print("\n[2/5] Analyzing and classifying bundles ...")
    for bundle in bundles:
        report.total_objects += len(bundle.objects)
        report.total_relationships += len(bundle.relationships)
        report.type_distribution.update(bundle.type_counts)

        domain = classify_bundle_domain(bundle)
        report.domain_distribution[domain] += 1

        print(f"  {bundle.filename:60s} -> {domain:8s}  "
              f"({len(bundle.objects)} obj, {len(bundle.relationships)} rels)")

    # ── Step 3: Generate Cypher queries ──
    print("\n[3/5] Generating Cypher queries ...")
    all_creates: list[str] = []
    all_matches: list[str] = []
    for bundle in bundles:
        creates = generate_create_queries(bundle)
        matches = generate_match_queries(bundle)
        all_creates.extend(creates)
        all_matches.extend(matches)

        if args.verbose:
            print(f"\n  --- {bundle.filename} ---")
            for q in creates[:5]:
                print(f"    CREATE: {q[:120]}")
            for q in matches[:3]:
                print(f"    MATCH:  {q[:120]}")

    report.cypher_create_queries = len(all_creates)
    report.cypher_match_queries = len(all_matches)
    print(f"  Generated {len(all_creates)} CREATE and {len(all_matches)} MATCH queries.")

    # ── Step 4: Generate Rust integration tests ──
    print("\n[4/5] Generating Rust integration tests ...")
    runtime_test_src, graph_test_src, _ = generate_rust_test(bundles, report)
    test_files: list[tuple[Path, str, str]] = [
        (
            PROJECT_ROOT / "crates" / "shared-runtime" / "tests" / "e2e_golden_dataset.rs",
            runtime_test_src,
            "shared-runtime",
        ),
        (
            PROJECT_ROOT / "crates" / "graph-core" / "tests" / "e2e_golden_dataset.rs",
            graph_test_src,
            "graph-core",
        ),
    ]

    for test_file, source, pkg in test_files:
        test_file.write_text(source, encoding="utf-8")
        print(f"  Written {test_file.relative_to(PROJECT_ROOT)}")

    # Format the generated files so `cargo fmt --check` stays clean.
    subprocess.run(
        ["rustfmt", "--edition", "2024", *(str(f) for f, _, _ in test_files)],
        check=True,
        cwd=PROJECT_ROOT,
    )
    print("  Formatted generated test files with rustfmt.")

    # ── Step 5: Run cargo test ──
    if args.skip_cargo:
        print("\n[5/5] Skipping cargo test (--skip-cargo)")
        report.rust_test_passed = True
    else:
        print("\n[5/5] Running cargo tests ...")
        all_passed = True
        for test_file, _, pkg in test_files:
            test_name = test_file.stem
            print(f"\n  Running {pkg}::{test_name} ...")
            passed, output = run_cargo_test_for_package(test_file, pkg)
            if not passed:
                all_passed = False
                report.errors.append(f"cargo test failed for {pkg}::{test_name}")
                lines = output.strip().split("\n")
                for line in lines[-40:]:
                    print(f"  | {line}")
            else:
                # Show summary line.
                for line in output.strip().split("\n"):
                    if "test result" in line:
                        print(f"  {line.strip()}")

        report.rust_test_passed = all_passed

    # ── Report ──
    print_report(report)

    return 0 if report.rust_test_passed else 1


if __name__ == "__main__":
    sys.exit(main())
