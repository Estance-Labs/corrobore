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
    Confidence, ExtractionRunId, Graph, NodeInput, RelationshipInput, TemporalTimestamp,
};

//
// Verify node records preserve PRD 10.1 optional epistemic tracking fields.
#[test]
fn node_preserves_optional_epistemic_fields() {
    let mut graph = Graph::new();

    let node_id = graph
        .create_node(
            NodeInput::new(["Entity"])
                .with_first_seen(
                    TemporalTimestamp::new("2026-07-01T00:00:00Z")
                        .expect("timestamp should be valid"),
                )
                .with_last_seen(
                    TemporalTimestamp::new("2026-07-06T00:00:00Z")
                        .expect("timestamp should be valid"),
                )
                .with_source_reliability(Confidence::new(0.83).expect("confidence should be valid"))
                .with_information_credibility(
                    Confidence::new(0.76).expect("confidence should be valid"),
                )
                .with_extraction_run_id(
                    ExtractionRunId::new("extraction-run--node-196")
                        .expect("extraction run ID should be valid"),
                ),
        )
        .expect("node should be created");

    let node = graph
        .get_node(&node_id)
        .expect("node lookup should succeed")
        .expect("node should exist");

    assert_eq!(node.first_seen(), Some("2026-07-01T00:00:00Z"));
    assert_eq!(node.last_seen(), Some("2026-07-06T00:00:00Z"));
    assert_eq!(node.source_reliability().map(Confidence::value), Some(0.83));
    assert_eq!(
        node.information_credibility().map(Confidence::value),
        Some(0.76)
    );
    assert_eq!(
        node.extraction_run_id().map(ExtractionRunId::as_str),
        Some("extraction-run--node-196")
    );
}

//
// Verify relationship records preserve PRD 10.1 optional epistemic tracking
// fields independently from node metadata.
#[test]
fn relationship_preserves_optional_epistemic_fields() {
    let mut graph = Graph::new();

    let source = graph
        .create_node(NodeInput::new(["Entity"]))
        .expect("source node should be created");
    let target = graph
        .create_node(NodeInput::new(["Entity"]))
        .expect("target node should be created");

    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "related_to", target.clone())
                .expect("relationship input should be valid")
                .with_first_seen(
                    TemporalTimestamp::new("2026-07-02T00:00:00Z")
                        .expect("timestamp should be valid"),
                )
                .with_last_seen(
                    TemporalTimestamp::new("2026-07-06T00:00:00Z")
                        .expect("timestamp should be valid"),
                )
                .with_source_reliability(Confidence::new(0.79).expect("confidence should be valid"))
                .with_information_credibility(
                    Confidence::new(0.71).expect("confidence should be valid"),
                )
                .with_extraction_run_id(
                    ExtractionRunId::new("extraction-run--relationship-196")
                        .expect("extraction run ID should be valid"),
                ),
        )
        .expect("relationship should be created");

    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should succeed")
        .expect("relationship should exist");

    assert_eq!(relationship.first_seen(), Some("2026-07-02T00:00:00Z"));
    assert_eq!(relationship.last_seen(), Some("2026-07-06T00:00:00Z"));
    assert_eq!(
        relationship.source_reliability().map(Confidence::value),
        Some(0.79)
    );
    assert_eq!(
        relationship
            .information_credibility()
            .map(Confidence::value),
        Some(0.71)
    );
    assert_eq!(
        relationship
            .extraction_run_id()
            .map(ExtractionRunId::as_str),
        Some("extraction-run--relationship-196")
    );
}
