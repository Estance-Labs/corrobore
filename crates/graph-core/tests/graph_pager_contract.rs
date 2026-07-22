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
// Graph pager contract tests.
//
// Graph pager contract tests.
// - add executable contract tests for the `GraphPager` abstraction;
// - keep coverage independent from any disk-backed storage backend;
// - use the current in-memory `Graph` pager plus controlled fake pagers for
// missing-page and corrupted-page error behavior.
//
// These tests intentionally do not add production storage, lazy page-in,
// prefetching, eviction, or graph working-set implementation logic.

use graph_core::{
    AdjacencyDirection, Graph, GraphPager, GraphPagerError, GraphPagerResult, GraphRecordMetadata,
    GraphRecordRef, LoadingState, NodeId, NodeInput, PagedAdjacency, PagedNode, PagedRelationship,
    PropertyValue, RecordStatus, RelationshipId, RelationshipInput, RelationshipType, StorageRef,
    WarmAdjacencyEntry, WarmAdjacencyEntryInput,
};

// Hold the in-memory pager data used by all successful contract tests.
// The fixture contains one campaign node, one outgoing
// narrative edge, and one incoming actor edge so payload, adjacency, and metadata
// contracts can be tested through the public `GraphPager` trait.
// Fixture setup should fail loudly if the public graph API rejects
// any test data.
struct GraphPagerContractFixture {
    graph: Graph,
    campaign_id: NodeId,
    narrative_id: NodeId,
    actor_id: NodeId,
    promotes_relationship_id: RelationshipId,
    attributed_relationship_id: RelationshipId,
}

// Create the shared in-memory pager contract fixture.
// All graph records are created through public graph-core APIs
// and then consumed only through the `GraphPager` abstraction.
// Invalid fixture wiring is treated as a test setup failure, not a
// pager contract result.
fn graph_pager_contract_fixture() -> GraphPagerContractFixture {
    let mut graph = Graph::new();

    let campaign_id = graph
        .create_node(
            NodeInput::new(["Campaign", "FIMI"])
                .with_property(
                    "name",
                    PropertyValue::String("Migration Narrative".to_owned()),
                )
                .with_property(
                    "source_reliability",
                    PropertyValue::String("high".to_owned()),
                )
                .with_status(RecordStatus::Candidate),
        )
        .expect("campaign node should be created");

    let narrative_id = graph
        .create_node(
            NodeInput::new(["Narrative"])
                .with_property("name", PropertyValue::String("Border Crisis".to_owned()))
                .with_status(RecordStatus::Candidate),
        )
        .expect("narrative node should be created");

    let actor_id = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("Actor Alpha".to_owned()))
                .with_status(RecordStatus::Candidate),
        )
        .expect("actor node should be created");

    let promotes_relationship_id = graph
        .create_relationship(
            RelationshipInput::new(campaign_id.clone(), "PROMOTES", narrative_id.clone())
                .expect("PROMOTES relationship input should be valid")
                .with_property("weight", PropertyValue::Float(0.82))
                .with_status(RecordStatus::Candidate),
        )
        .expect("PROMOTES relationship should be created");

    let attributed_relationship_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "ATTRIBUTED_TO", campaign_id.clone())
                .expect("ATTRIBUTED_TO relationship input should be valid")
                .with_property(
                    "confidence_note",
                    PropertyValue::String("analyst".to_owned()),
                )
                .with_status(RecordStatus::Candidate),
        )
        .expect("ATTRIBUTED_TO relationship should be created");

    GraphPagerContractFixture {
        graph,
        campaign_id,
        narrative_id,
        actor_id,
        promotes_relationship_id,
        attributed_relationship_id,
    }
}

// Construct the deterministic in-memory storage reference expected for
// a node payload or metadata record.
// Matches the public in-memory pager contract while remaining
// backend-neutral through `StorageRef`.
// None expected because the caller provides a typed node ID.
fn in_memory_node_ref(node_id: &NodeId) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/nodes/{}", node_id.as_str()),
    }
}

// Construct the deterministic in-memory storage reference expected for
// a relationship payload or metadata record.
// Matches the public in-memory pager contract while remaining
// backend-neutral through `StorageRef`.
// None expected because the caller provides a typed relationship ID.
fn in_memory_relationship_ref(relationship_id: &RelationshipId) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/relationships/{}", relationship_id.as_str()),
    }
}

// Construct the deterministic in-memory storage reference expected for
// an adjacency page.
// Includes direction and owner node identity without exposing
// the graph's private adjacency indexes.
// None expected because direction and node ID are typed.
fn in_memory_adjacency_ref(node_id: &NodeId, direction: AdjacencyDirection) -> StorageRef {
    let direction_segment = match direction {
        AdjacencyDirection::Outgoing => "outgoing",
        AdjacencyDirection::Incoming => "incoming",
    };

    StorageRef::External {
        uri: format!(
            "memory://graph/adjacency/{}/{}",
            direction_segment,
            node_id.as_str()
        ),
    }
}

// Reserve a fake pager that reports missing physical pages.
// Every pager method returns `GraphPagerError::MissingPage`
// using the configured storage reference.
// None expected because this is a controlled test double.
struct MissingPagePager {
    storage_ref: StorageRef,
}

impl GraphPager for MissingPagePager {
    fn load_node_payload(&self, _node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        Err(GraphPagerError::MissingPage {
            storage_ref: self.storage_ref.clone(),
        })
    }

    fn load_relationship_payload(
        &self,
        _relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship> {
        Err(GraphPagerError::MissingPage {
            storage_ref: self.storage_ref.clone(),
        })
    }

    fn load_outgoing_adjacency(&self, _node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        Err(GraphPagerError::MissingPage {
            storage_ref: self.storage_ref.clone(),
        })
    }

    fn load_incoming_adjacency(&self, _node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        Err(GraphPagerError::MissingPage {
            storage_ref: self.storage_ref.clone(),
        })
    }

    fn load_indexed_metadata(
        &self,
        _record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        Err(GraphPagerError::MissingPage {
            storage_ref: self.storage_ref.clone(),
        })
    }
}

// Reserve a fake pager that reports corrupted physical pages.
// Every pager method returns `GraphPagerError::CorruptedPage`
// using the configured storage reference and reason.
// None expected because this is a controlled test double.
struct CorruptedPagePager {
    storage_ref: StorageRef,
    reason: String,
}

impl GraphPager for CorruptedPagePager {
    fn load_node_payload(&self, _node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        Err(self.error())
    }

    fn load_relationship_payload(
        &self,
        _relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship> {
        Err(self.error())
    }

    fn load_outgoing_adjacency(&self, _node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        Err(self.error())
    }

    fn load_incoming_adjacency(&self, _node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        Err(self.error())
    }

    fn load_indexed_metadata(
        &self,
        _record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        Err(self.error())
    }
}

impl CorruptedPagePager {
    fn error(&self) -> GraphPagerError {
        GraphPagerError::CorruptedPage {
            storage_ref: self.storage_ref.clone(),
            reason: self.reason.clone(),
        }
    }
}

// Build a typed node ID for missing-record tests.
// Returns a valid ID that is not present in the fixture graph.
// Invalid hard-coded IDs should fail test setup.
fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

// Build a typed relationship ID for missing-record tests.
// Returns a valid ID that is not present in the fixture graph.
// Invalid hard-coded IDs should fail test setup.
fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

// Verify node payload loading by stable ID.
// Given: a pager fixture containing a known campaign node.
// When: `GraphPager::load_node_payload` is called with that node ID.
// Then: the result contains the full node payload and the storage reference that
// produced it.
#[test]
fn load_node_payload_by_id_returns_payload_and_storage_ref() {
    let fixture = graph_pager_contract_fixture();

    let paged_node = fixture
        .graph
        .load_node_payload(&fixture.campaign_id)
        .expect("node payload should load through GraphPager");

    assert_eq!(paged_node.node.id(), &fixture.campaign_id);
    assert!(paged_node.node.has_label("Campaign"));
    assert!(paged_node.node.has_label("FIMI"));
    assert_eq!(
        paged_node.node.property("name"),
        Some(&PropertyValue::String("Migration Narrative".to_owned()))
    );
    assert_eq!(
        paged_node.storage_ref,
        Some(in_memory_node_ref(&fixture.campaign_id))
    );
}

// Verify relationship payload loading by stable ID.
// Given: a pager fixture containing a known PROMOTES relationship.
// When: `GraphPager::load_relationship_payload` is called with that relationship ID.
// Then: the result contains the full relationship payload and the storage
// reference that produced it.
#[test]
fn load_relationship_payload_by_id_returns_payload_and_storage_ref() {
    let fixture = graph_pager_contract_fixture();

    let paged_relationship = fixture
        .graph
        .load_relationship_payload(&fixture.promotes_relationship_id)
        .expect("relationship payload should load through GraphPager");

    assert_eq!(
        paged_relationship.relationship.id(),
        &fixture.promotes_relationship_id
    );
    assert_eq!(
        paged_relationship.relationship.source(),
        &fixture.campaign_id
    );
    assert_eq!(
        paged_relationship.relationship.target(),
        &fixture.narrative_id
    );
    assert_eq!(
        paged_relationship.relationship.rel_type().as_str(),
        "PROMOTES"
    );
    assert_eq!(
        paged_relationship.relationship.property("weight"),
        Some(&PropertyValue::Float(0.82))
    );
    assert_eq!(
        paged_relationship.storage_ref,
        Some(in_memory_relationship_ref(
            &fixture.promotes_relationship_id
        ))
    );
}

// Verify outgoing adjacency loading by owner node ID.
// Given: a pager fixture containing a campaign -> narrative edge.
// When: `GraphPager::load_outgoing_adjacency` is called for the campaign node.
// Then: the result is an outgoing lightweight adjacency page with relationship,
// neighbor, relationship-type hint, and lazy-load storage refs.
#[test]
fn load_outgoing_adjacency_by_node_id_returns_lightweight_frontier() {
    let fixture = graph_pager_contract_fixture();

    let page = fixture
        .graph
        .load_outgoing_adjacency(&fixture.campaign_id)
        .expect("outgoing adjacency should load through GraphPager");

    assert_eq!(page.owner_node_id, fixture.campaign_id);
    assert_eq!(page.direction, AdjacencyDirection::Outgoing);
    assert_eq!(
        page.storage_ref,
        Some(in_memory_adjacency_ref(
            &fixture.campaign_id,
            AdjacencyDirection::Outgoing
        ))
    );
    assert_eq!(page.entries.len(), 1);

    let entry = &page.entries[0];
    assert_eq!(entry.relationship_id, fixture.promotes_relationship_id);
    assert_eq!(entry.neighbor_node_id, fixture.narrative_id);
    assert_eq!(
        entry
            .relationship_type
            .as_ref()
            .map(RelationshipType::as_str),
        Some("PROMOTES")
    );
    assert_eq!(
        entry.relationship_storage_ref,
        Some(in_memory_relationship_ref(
            &fixture.promotes_relationship_id
        ))
    );
    assert_eq!(
        entry.neighbor_storage_ref,
        Some(in_memory_node_ref(&fixture.narrative_id))
    );
}

// Verify incoming adjacency loading by owner node ID.
// Given: a pager fixture containing an actor -> campaign edge.
// When: `GraphPager::load_incoming_adjacency` is called for the campaign node.
// Then: the result is an incoming lightweight adjacency page with relationship,
// neighbor, relationship-type hint, and lazy-load storage refs.
#[test]
fn load_incoming_adjacency_by_node_id_returns_lightweight_frontier() {
    let fixture = graph_pager_contract_fixture();

    let page = fixture
        .graph
        .load_incoming_adjacency(&fixture.campaign_id)
        .expect("incoming adjacency should load through GraphPager");

    assert_eq!(page.owner_node_id, fixture.campaign_id);
    assert_eq!(page.direction, AdjacencyDirection::Incoming);
    assert_eq!(
        page.storage_ref,
        Some(in_memory_adjacency_ref(
            &fixture.campaign_id,
            AdjacencyDirection::Incoming
        ))
    );
    assert_eq!(page.entries.len(), 1);

    let entry = &page.entries[0];
    assert_eq!(entry.relationship_id, fixture.attributed_relationship_id);
    assert_eq!(entry.neighbor_node_id, fixture.actor_id);
    assert_eq!(
        entry
            .relationship_type
            .as_ref()
            .map(RelationshipType::as_str),
        Some("ATTRIBUTED_TO")
    );
    assert_eq!(
        entry.relationship_storage_ref,
        Some(in_memory_relationship_ref(
            &fixture.attributed_relationship_id
        ))
    );
    assert_eq!(
        entry.neighbor_storage_ref,
        Some(in_memory_node_ref(&fixture.actor_id))
    );
}

// Verify indexed node metadata can be loaded without a full payload.
// Given: a pager fixture containing indexed campaign metadata.
// When: `GraphPager::load_indexed_metadata` is called with a node record ref.
// Then: the result exposes lightweight labels, indexed properties, loading state,
// and storage ref without returning a `Node` payload.
#[test]
fn load_indexed_metadata_for_node_returns_metadata_without_payload() {
    let fixture = graph_pager_contract_fixture();

    let metadata = fixture
        .graph
        .load_indexed_metadata(&GraphRecordRef::Node(fixture.campaign_id.clone()))
        .expect("node metadata should load through GraphPager");

    assert_eq!(
        metadata.record_ref,
        GraphRecordRef::Node(fixture.campaign_id.clone())
    );
    assert_eq!(
        metadata.storage_ref,
        Some(in_memory_node_ref(&fixture.campaign_id))
    );
    assert_eq!(metadata.loading_state, LoadingState::Indexed);
    assert!(metadata.labels.contains(&"Campaign".to_owned()));
    assert!(metadata.labels.contains(&"FIMI".to_owned()));
    assert_eq!(metadata.relationship_type, None);
    assert_eq!(
        metadata.indexed_properties.get("source_reliability"),
        Some(&PropertyValue::String("high".to_owned()))
    );
}

// Verify indexed relationship metadata can be loaded without a full payload.
// Given: a pager fixture containing indexed PROMOTES relationship metadata.
// When: `GraphPager::load_indexed_metadata` is called with a relationship record ref.
// Then: the result exposes identity, relationship-type hint, loading state, and
// storage ref without returning a `Relationship` payload.
#[test]
fn load_indexed_metadata_for_relationship_returns_metadata_without_payload() {
    let fixture = graph_pager_contract_fixture();

    let metadata = fixture
        .graph
        .load_indexed_metadata(&GraphRecordRef::Relationship(
            fixture.promotes_relationship_id.clone(),
        ))
        .expect("relationship metadata should load through GraphPager");

    assert_eq!(
        metadata.record_ref,
        GraphRecordRef::Relationship(fixture.promotes_relationship_id.clone())
    );
    assert_eq!(
        metadata.storage_ref,
        Some(in_memory_relationship_ref(
            &fixture.promotes_relationship_id
        ))
    );
    assert_eq!(metadata.loading_state, LoadingState::Indexed);
    assert!(metadata.labels.is_empty());
    assert_eq!(
        metadata
            .relationship_type
            .as_ref()
            .map(RelationshipType::as_str),
        Some("PROMOTES")
    );
    assert_eq!(
        metadata.indexed_properties.get("weight"),
        Some(&PropertyValue::Float(0.82))
    );
}

// Verify missing logical node payload behavior on the in-memory pager.
// Given: a valid node ID absent from the graph.
// When: `GraphPager::load_node_payload` is called.
// Then: the pager returns a typed unavailable-record error instead of panicking or
// returning an empty successful payload.
#[test]
fn missing_node_payload_returns_typed_unavailable_record_error() {
    let fixture = graph_pager_contract_fixture();
    let missing_id = node_id("node--missing-for-pager-contract");

    let error = fixture
        .graph
        .load_node_payload(&missing_id)
        .expect_err("missing node should return a typed pager error");

    assert!(matches!(
    error,
    GraphPagerError::UnavailableRecord { record_ref }
    if record_ref == GraphRecordRef::Node(missing_id)
    ));
}

// Verify missing logical relationship payload behavior on the in-memory pager.
// Given: a valid relationship ID absent from the graph.
// When: `GraphPager::load_relationship_payload` is called.
// Then: the pager returns a typed unavailable-record error instead of panicking or
// returning an empty successful payload.
#[test]
fn missing_relationship_payload_returns_typed_unavailable_record_error() {
    let fixture = graph_pager_contract_fixture();
    let missing_id = relationship_id("relationship--missing-for-pager-contract");

    let error = fixture
        .graph
        .load_relationship_payload(&missing_id)
        .expect_err("missing relationship should return a typed pager error");

    assert!(matches!(
    error,
    GraphPagerError::UnavailableRecord { record_ref }
    if record_ref == GraphRecordRef::Relationship(missing_id)
    ));
}

// Verify physical missing node page behavior through a controlled fake pager.
// Given: a fake pager whose node payload location is missing.
// When: the node payload is requested.
// Then: the pager returns `GraphPagerError::MissingPage` with the expected storage ref.
#[test]
fn missing_node_page_returns_typed_missing_page_error() {
    let missing_ref = StorageRef::Page {
        segment: "nodes".to_owned(),
        page_id: 404,
    };
    let pager = MissingPagePager {
        storage_ref: missing_ref.clone(),
    };

    let error = pager
        .load_node_payload(&node_id("node--missing-page"))
        .expect_err("missing node page should return a typed pager error");

    assert!(matches!(
    error,
    GraphPagerError::MissingPage { storage_ref } if storage_ref == missing_ref
    ));
}

// Verify physical missing relationship page behavior through a controlled fake pager.
// Given: a fake pager whose relationship payload location is missing.
// When: the relationship payload is requested.
// Then: the pager returns `GraphPagerError::MissingPage` with the expected storage ref.
#[test]
fn missing_relationship_page_returns_typed_missing_page_error() {
    let missing_ref = StorageRef::Page {
        segment: "relationships".to_owned(),
        page_id: 405,
    };
    let pager = MissingPagePager {
        storage_ref: missing_ref.clone(),
    };

    let error = pager
        .load_relationship_payload(&relationship_id("relationship--missing-page"))
        .expect_err("missing relationship page should return a typed pager error");

    assert!(matches!(
    error,
    GraphPagerError::MissingPage { storage_ref } if storage_ref == missing_ref
    ));
}

// Verify corrupted page behavior through a controlled fake pager.
// Given: a fake pager whose backend page cannot be decoded safely.
// When: any pager load operation reaches that page.
// Then: the pager returns `GraphPagerError::CorruptedPage` with the expected
// storage ref and reason.
#[test]
fn corrupted_page_returns_typed_corrupted_page_error() {
    let corrupted_ref = StorageRef::Page {
        segment: "adjacency/outgoing".to_owned(),
        page_id: 500,
    };
    let pager = CorruptedPagePager {
        storage_ref: corrupted_ref.clone(),
        reason: "checksum mismatch".to_owned(),
    };

    let error = pager
        .load_outgoing_adjacency(&node_id("node--corrupted-page"))
        .expect_err("corrupted page should return a typed pager error");

    assert!(matches!(
    error,
    GraphPagerError::CorruptedPage { storage_ref, reason }
    if storage_ref == corrupted_ref && reason == "checksum mismatch"
    ));
}

// Verify pager adjacency results can feed warm adjacency entries.
// Given: a `PagedAdjacency` result returned by the in-memory pager.
// When: test code converts its first lightweight entry into a warm adjacency entry.
// Then: the entry carries enough relationship, neighbor, direction, and
// storage-reference information for future working-set warm frontier construction.
#[test]
fn pager_adjacency_result_can_feed_warm_adjacency_entries() {
    let fixture = graph_pager_contract_fixture();
    let page = fixture
        .graph
        .load_outgoing_adjacency(&fixture.campaign_id)
        .expect("outgoing adjacency should load through GraphPager");
    let entry = page
        .entries
        .first()
        .expect("fixture should contain one outgoing adjacency entry");
    let relationship_type = entry
        .relationship_type
        .clone()
        .expect("pager adjacency should include a relationship type hint");

    let warm_entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            entry.relationship_id.clone(),
            relationship_type,
            page.owner_node_id.clone(),
            entry.neighbor_node_id.clone(),
            Vec::new(),
            page.direction,
        )
        .with_target_loading_state(LoadingState::Warm)
        .with_storage_refs(
            entry.relationship_storage_ref.clone(),
            entry.neighbor_storage_ref.clone(),
        ),
    )
    .expect("paged adjacency should contain enough data for a warm entry");

    assert_eq!(
        warm_entry.relationship_id(),
        &fixture.promotes_relationship_id
    );
    assert_eq!(warm_entry.relationship_type().as_str(), "PROMOTES");
    assert_eq!(warm_entry.source_node_id(), &fixture.campaign_id);
    assert_eq!(warm_entry.target_node_id(), &fixture.narrative_id);
    assert_eq!(warm_entry.direction(), AdjacencyDirection::Outgoing);
    assert_eq!(warm_entry.target_loading_state(), LoadingState::Warm);
    assert!(warm_entry.is_target_unloaded());
    assert_eq!(
        warm_entry.relationship_storage_ref(),
        Some(&in_memory_relationship_ref(
            &fixture.promotes_relationship_id
        ))
    );
    assert_eq!(
        warm_entry.target_storage_ref(),
        Some(&in_memory_node_ref(&fixture.narrative_id))
    );
}
