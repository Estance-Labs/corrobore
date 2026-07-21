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
const PAGED_STORAGE_LAYOUT_DOC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../project-documents/feature-0003-persistent-storage/artifacts/paged-graph-storage-layout.md"
));

fn assert_contains_all(document: &str, expected_fragments: &[&str]) {
    for expected_fragment in expected_fragments {
        assert!(
            document.contains(expected_fragment),
            "expected storage layout documentation to contain fragment: {expected_fragment}"
        );
    }
}

//
// Ensure the storage architecture document remains present and explicitly tied to Epic 0002.
//
// Given:
// A documentation issue whose purpose is to constrain future working-set streaming storage decisions.
//
// When:
// The paged storage layout document is loaded by the test suite.
//
// Then:
// It must exist, have a stable title, and link back to the Graph Working Set Streaming epic.
#[test]
fn paged_storage_layout_document_exists_and_links_epic_0002() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "# Paged Graph Storage Layout",
            ": Graph Working Set Streaming",
            "../../feature-0002-working-set-streaming/artifacts/0002-graph-working-set-streaming.md",
        ],
    );
}

//
// Ensure the target layout keeps loadable graph areas separate instead of normalizing around a monolithic graph file.
//
// Given:
// Future working set streaming needs independently addressable catalog, payload, adjacency, semantic, log, and snapshot areas.
//
// When:
// The target logical layout section is reviewed.
//
// Then:
// It must name each required top-level storage area and the direction-specific adjacency sub-areas.
#[test]
fn target_logical_layout_names_required_storage_areas() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "storage/",
            "catalog/",
            "nodes/",
            "relationships/",
            "adjacency/",
            "outgoing/",
            "incoming/",
            "semantic/",
            "logs/",
            "snapshots/",
        ],
    );
}

//
// Ensure node payload responsibilities are documented independently from relationship payload responsibilities.
//
// Given:
// Working set streaming must be able to page in node payloads without forcing relationship payload loading, and vice versa.
//
// When:
// The storage layout documentation describes payload responsibilities.
//
// Then:
// It must include separate sections for node pages and relationship pages with explicit payload separation language.
#[test]
fn document_separates_node_and_relationship_payload_responsibilities() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## Node page responsibilities",
            "## Relationship page responsibilities",
            "node payloads",
            "relationship payloads",
            "loadable independently from relationships and adjacency",
            "separable from node payloads and adjacency shards",
        ],
    );
}

//
// Ensure adjacency is treated as its own loadable graph area rather than as a side effect of payload loading.
//
// Given:
// Warm frontier construction and lazy page-in require lightweight traversal data before full payloads are hot.
//
// When:
// The storage layout documentation describes outgoing and incoming adjacency.
//
// Then:
// It must document both shard directions and explain that adjacency supports bounded expansion without full payload reads.
#[test]
fn document_separates_adjacency_from_full_payload_loading() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## Outgoing adjacency shard responsibilities",
            "## Incoming adjacency shard responsibilities",
            "outgoing adjacency shards support seed expansion",
            "incoming adjacency shards support reverse expansion",
            "without full payload reads",
            "Reverse traversal",
        ],
    );
}

//
// Ensure the catalog responsibilities preserve fast graph opening and indexed lookup without full graph loading.
//
// Given:
// The engine must be able to open a persistent graph and know what exists before loading every record.
//
// When:
// The catalog section is reviewed.
//
// Then:
// It must name schema, labels, relationship types, properties, ID indexes, adjacency summaries, checkpoint metadata, transaction log metadata, and semantic references.
#[test]
fn document_catalog_responsibilities_for_early_graph_opening() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## Catalog responsibilities",
            "schema registry metadata",
            "labels",
            "relationship types",
            "property keys",
            "ID indexes",
            "label-to-node indexes",
            "selected property indexes",
            "adjacency summaries",
            "checkpoint metadata",
            "transaction log metadata",
            "semantic index references",
        ],
    );
}

//
// Ensure semantic storage responsibilities map natural-language entry points back to graph node IDs.
//
// Given:
// defines semantic search as an entry point, not as a replacement for graph traversal.
//
// When:
// The semantic storage section is reviewed.
//
// Then:
// It must distinguish vector index data from node embedding mappings and require graph seed node identifiers.
#[test]
fn document_semantic_index_and_node_embedding_mapping_responsibilities() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## Semantic index and node embedding map responsibilities",
            "natural-language intent",
            "graph seed node IDs",
            "vector index data",
            "node embedding map",
            "graph node identifiers",
            "storage references",
        ],
    );
}

//
// Ensure logs and snapshots remain visible future-compatible storage areas.
//
// Given:
// The engine must preserve recovery, auditability, reproducibility, deterministic export, and future graph diff workflows.
//
// When:
// The log and snapshot sections are reviewed.
//
// Then:
// They must distinguish WAL from audit logs and reserve snapshot metadata responsibilities.
#[test]
fn document_logs_and_snapshots_as_future_compatible_areas() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## WAL and audit log placement",
            "transaction recovery",
            "mutation auditability",
            "WAL-oriented recovery data",
            "audit-oriented explainability data",
            "## Snapshot metadata placement",
            "reproducibility",
            "deterministic export",
            "graph diff workflows",
        ],
    );
}

//
// Ensure the storage layout documentation explains how separable areas support Graph Working Set Streaming.
//
// Given:
// The target runtime flow is semantic seed selection, hot loading, warm frontier construction, lazy page-in, and budgeted traversal.
//
// When:
// The separation and working-set sections are reviewed.
//
// Then:
// They must connect storage separability to bounded loading, prefetch, eviction, supernode protection, and hot/warm/cold loading states.
#[test]
fn document_explains_how_separation_supports_working_set_streaming() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "## Separation requirements",
            "bounded loading",
            "warm frontier construction",
            "lazy page-in",
            "prefetch",
            "eviction",
            "supernode protection",
            "## Working set streaming support",
            "hot records",
            "warming frontier adjacency",
            "hot, warm, indexed, or cold loading states",
        ],
    );
}

//
// Ensure the documentation leaves room for simple initial storage before binary pages exist.
//
// Given:
// Issue #45 explicitly allows the first implementation to use mocks or simple files before binary pages.
//
// When:
// The non-goals and layout sections are reviewed.
//
// Then:
// They must state that no production storage implementation is required and that mocks or simple files are acceptable before binary pages.
#[test]
fn document_allows_mock_or_simple_file_storage_before_binary_pages() {
    assert_contains_all(
        PAGED_STORAGE_LAYOUT_DOC,
        &[
            "no production storage implementation is required",
            "mocks, simple files, or text-based page representations before binary pages",
            "not as a mandatory phase-1\non-disk binary format",
        ],
    );
}

//
// Ensure phase-3 documentation work replaces phase-1 scaffolding with final explanatory content.
//
// Given:
// Verifies that finalized documentation does not contain scaffolding markers.
//
// When:
// The documentation is considered complete for the issue.
//
// Then:
// The final document must no longer contain phase scaffolding markers.
#[test]
fn finalized_documentation_does_not_contain_phase_scaffolding() {
    for forbidden_fragment in [
        "TODO(phase-3)",
        "Phase 1 skeleton",
        "This document is intentionally limited",
    ] {
        assert!(
            !PAGED_STORAGE_LAYOUT_DOC.contains(forbidden_fragment),
            "final storage layout documentation should not contain phase scaffolding fragment: {forbidden_fragment}"
        );
    }
}
