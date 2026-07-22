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
    GraphPager, GraphPagerError, GraphPagerResult, GraphRecordRef, NodeId, PageIdentity,
    PageIdentityKind, PageInRequest, PageInResult, PageInStatus, RelationshipId, StorageRef,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

//
// Validate that page identity captures stable location and kind metadata without
// exposing storage-specific internals.
#[test]
fn page_identity_contract_is_explicit_and_matchable() {
    let page = PageIdentity {
        kind: PageIdentityKind::Adjacency,
        segment: "adjacency/outgoing".to_owned(),
        page_id: "node--campaign-1/outgoing/page-7".to_owned(),
        storage_ref: Some(StorageRef::Page {
            segment: "adjacency/outgoing".to_owned(),
            page_id: 7,
        }),
    };

    assert!(matches!(page.kind, PageIdentityKind::Adjacency));
    assert_eq!(page.segment, "adjacency/outgoing");
    assert_eq!(page.page_id, "node--campaign-1/outgoing/page-7");
    assert!(matches!(
    page.storage_ref,
    Some(StorageRef::Page { segment, page_id })
    if segment == "adjacency/outgoing" && page_id == 7
    ));
}

//
// Validate that page-in request and response contracts preserve status and
// loaded references for deterministic caller handling.
#[test]
fn page_in_contract_represents_hit_miss_and_load_outcomes() {
    let request = PageInRequest {
        page: PageIdentity {
            kind: PageIdentityKind::NodePayload,
            segment: "nodes".to_owned(),
            page_id: "node--42".to_owned(),
            storage_ref: Some(StorageRef::Record {
                collection: "nodes".to_owned(),
                key: "node--42".to_owned(),
            }),
        },
        record_refs: vec![GraphRecordRef::Node(node_id("node--42"))],
    };

    let result = PageInResult {
        request: request.clone(),
        status: PageInStatus::Loaded,
        loaded_record_refs: request.record_refs.clone(),
    };

    assert_eq!(result.request, request);
    assert_eq!(result.status, PageInStatus::Loaded);
    assert_eq!(result.loaded_record_refs.len(), 1);

    assert_eq!(PageInStatus::Hit, PageInStatus::Hit);
    assert_eq!(PageInStatus::Miss, PageInStatus::Miss);
    assert_eq!(PageInStatus::Loaded, PageInStatus::Loaded);
    assert_eq!(PageInStatus::Rejected, PageInStatus::Rejected);
}

//
// Validate that GraphPager exposes a page-in method and that test doubles can
// provide explicit page-in support errors deterministically.
#[test]
fn graph_pager_supports_page_in_contract() {
    struct NoPageInPager;

    impl GraphPager for NoPageInPager {
        fn load_node_payload(&self, _node_id: &NodeId) -> GraphPagerResult<graph_core::PagedNode> {
            panic!("load_node_payload should not be called in this page_in contract test")
        }

        fn load_relationship_payload(
            &self,
            _relationship_id: &RelationshipId,
        ) -> GraphPagerResult<graph_core::PagedRelationship> {
            panic!("load_relationship_payload should not be called in this page_in contract test")
        }

        fn load_outgoing_adjacency(
            &self,
            _node_id: &NodeId,
        ) -> GraphPagerResult<graph_core::PagedAdjacency> {
            panic!("load_outgoing_adjacency should not be called in this page_in contract test")
        }

        fn load_incoming_adjacency(
            &self,
            _node_id: &NodeId,
        ) -> GraphPagerResult<graph_core::PagedAdjacency> {
            panic!("load_incoming_adjacency should not be called in this page_in contract test")
        }

        fn load_indexed_metadata(
            &self,
            _record_ref: &GraphRecordRef,
        ) -> GraphPagerResult<graph_core::GraphRecordMetadata> {
            panic!("load_indexed_metadata should not be called in this page_in contract test")
        }
    }

    let pager = NoPageInPager;
    let request = PageInRequest {
        page: PageIdentity {
            kind: PageIdentityKind::RelationshipPayload,
            segment: "relationships".to_owned(),
            page_id: "relationship--9".to_owned(),
            storage_ref: Some(StorageRef::Record {
                collection: "relationships".to_owned(),
                key: "relationship--9".to_owned(),
            }),
        },
        record_refs: vec![GraphRecordRef::Relationship(relationship_id(
            "relationship--9",
        ))],
    };

    let error = pager
        .page_in(&request)
        .expect_err("default page_in should return not-supported error");

    assert!(matches!(
    error,
    GraphPagerError::PageInNotSupported { page } if page == request.page
    ));
}
