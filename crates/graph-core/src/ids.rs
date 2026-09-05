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
//! Typed identifier primitives for graph-core records and metadata.
//!
//! Module boundary:
//! this module owns string-backed identifier construction, validation, and cheap
//! borrowed access. It must not own graph storage, record lifecycle semantics,
//! domain-specific STIX, CTI, FIMI, or crisis rules.

use serde::{Deserialize, Serialize};

use crate::GraphError;

macro_rules! string_id {
    ($name:ident) => {
        #[doc = concat!("Typed graph-core identifier for `", stringify!($name), "`.")]
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name {
            value: String,
        }

        impl $name {
            /// Build a typed identifier from a string-like value.
            ///
            /// This constructor is the only public way to create the identifier.
            /// It converts the incoming value into a `String`, validates it,
            /// and returns a strongly typed wrapper.
            ///
            ///
            /// 1. Convert `value` into a `String`.
            /// 2. Reject empty strings.
            /// 3. Reject strings that contain only whitespace.
            /// 4. Return `GraphError::InvalidIdentifier(...)` when validation fails.
            /// 5. Return `Ok(Self { value })` when validation succeeds.
            ///
            /// Design notes:
            ///
            /// - The identifier is validated with `trim().is_empty()` but stored
            ///   exactly as provided.
            /// - This keeps validation separate from normalization.
            /// - The error payload uses the ID type name so callers know which
            ///   identifier failed validation.
            pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
                let value = value.into();

                if value.trim().is_empty() {
                    return Err(GraphError::InvalidIdentifier(stringify!($name).to_owned()));
                }

                Ok(Self { value })
            }

            /// Return the identifier as a borrowed string slice.
            ///
            /// This method exposes the inner string without cloning it.
            /// It is useful when callers need to serialize, log, compare, or pass
            /// the identifier to another API as `&str`.
            ///
            ///
            /// 1. Borrow the inner `String` stored in `value`.
            /// 2. Return it as `&str`.
            /// 3. Do not allocate.
            /// 4. Do not mutate the identifier.
            pub fn as_str(&self) -> &str {
                self.value.as_str()
            }
        }
    };
}

// Stable graph record identifiers.
string_id!(NodeId);
string_id!(RelationshipId);

// Version identifiers used by full record versioning.
string_id!(NodeVersionId);
string_id!(RelationshipVersionId);

// Evidence and transaction metadata identifiers.
string_id!(EvidenceId);
string_id!(SourceId);
string_id!(ObservationId);
string_id!(SourceVersionId);
string_id!(ValidationErrorId);
string_id!(ClaimId);
string_id!(ClaimVersionId);
string_id!(SnapshotId);
string_id!(TransactionId);
string_id!(WorkspaceId);
string_id!(HypothesisWorkspaceId);
string_id!(ActorId);
string_id!(SessionId);
string_id!(RequestId);
string_id!(ExtractionRunId);
string_id!(RuntimeId);
string_id!(FactId);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashMap;

    use super::*;

    //
    // Verify that a normal graph node identifier can be constructed and read
    // through the public API. This is the most basic happy path for the ID
    // wrapper because node IDs will be used everywhere in graph-core.
    //
    // Given a non-empty node ID string,
    // when `NodeId::new` is called,
    // then construction should succeed and `as_str` should return the same value.
    #[test]
    fn node_id_accepts_valid_value() {
        let id = NodeId::new("node--1").expect("valid node ID should be accepted");

        assert_eq!(id.as_str(), "node--1");
    }

    //
    // Verify that relationship IDs follow the same construction rules as node
    // IDs. Relationship IDs are stable graph record identifiers, so they need
    // the same validation contract.
    //
    // Given a non-empty relationship ID string,
    // when `RelationshipId::new` is called,
    // then construction should succeed and `as_str` should return the same value.
    #[test]
    fn relationship_id_accepts_valid_value() {
        let id = RelationshipId::new("relationship--1")
            .expect("valid relationship ID should be accepted");

        assert_eq!(id.as_str(), "relationship--1");
    }

    //
    // Verify that version IDs are represented as first-class typed IDs rather
    // than raw strings. Node version IDs are required by the full record
    // versioning model.
    //
    // Given a non-empty node version ID string,
    // when `NodeVersionId::new` is called,
    // then construction should succeed and `as_str` should return the same value.
    #[test]
    fn node_version_id_accepts_valid_value() {
        let id = NodeVersionId::new("node-version--1")
            .expect("valid node version ID should be accepted");

        assert_eq!(id.as_str(), "node-version--1");
    }

    //
    // Verify that relationship version IDs follow the same validation contract
    // as node version IDs. These IDs will later identify immutable relationship
    // versions.
    //
    // Given a non-empty relationship version ID string,
    // when `RelationshipVersionId::new` is called,
    // then construction should succeed and `as_str` should return the same value.
    #[test]
    fn relationship_version_id_accepts_valid_value() {
        let id = RelationshipVersionId::new("relationship-version--1")
            .expect("valid relationship version ID should be accepted");

        assert_eq!(id.as_str(), "relationship-version--1");
    }

    //
    // Verify that all metadata identifiers created by the shared ID macro expose
    // the same public behavior. These identifiers will be used by evidence,
    // transaction, workspace, actor, session, request, and extraction metadata.
    //
    // Given one valid value per metadata ID type,
    // when each typed ID is constructed,
    // then each one should preserve and return its original string value.
    #[test]
    fn metadata_ids_accept_valid_values() {
        assert_eq!(
            EvidenceId::new("evidence--1").unwrap().as_str(),
            "evidence--1"
        );
        assert_eq!(
            ValidationErrorId::new("validation-error--1")
                .unwrap()
                .as_str(),
            "validation-error--1"
        );
        assert_eq!(
            SnapshotId::new("snapshot--1").unwrap().as_str(),
            "snapshot--1"
        );
        assert_eq!(
            TransactionId::new("transaction--1").unwrap().as_str(),
            "transaction--1"
        );
        assert_eq!(
            WorkspaceId::new("workspace--1").unwrap().as_str(),
            "workspace--1"
        );
        assert_eq!(ActorId::new("actor--1").unwrap().as_str(), "actor--1");
        assert_eq!(SessionId::new("session--1").unwrap().as_str(), "session--1");
        assert_eq!(RequestId::new("request--1").unwrap().as_str(), "request--1");
        assert_eq!(
            ExtractionRunId::new("extraction-run--1").unwrap().as_str(),
            "extraction-run--1"
        );
    }

    //
    // Verify that empty identifiers are rejected at construction time. The graph
    // core should never accept a stable ID that cannot identify a record.
    //
    // Given an empty string,
    // when `NodeId::new` is called,
    // then construction should fail with `GraphError::InvalidIdentifier("NodeId")`.
    #[test]
    fn node_id_rejects_empty_value() {
        let error = NodeId::new("").expect_err("empty node ID should be rejected");

        assert!(matches!(error, GraphError::InvalidIdentifier(kind) if kind == "NodeId"));
    }

    //
    // Verify that whitespace-only identifiers are rejected. This prevents values
    // that are technically non-empty strings but still meaningless as IDs.
    //
    // Given a whitespace-only string,
    // when `NodeId::new` is called,
    // then construction should fail with `GraphError::InvalidIdentifier("NodeId")`.
    #[test]
    fn node_id_rejects_whitespace_only_value() {
        let error = NodeId::new(" ").expect_err("whitespace-only node ID should be rejected");

        assert!(matches!(error, GraphError::InvalidIdentifier(kind) if kind == "NodeId"));
    }

    //
    // Verify that relationship IDs reject empty values with the correct typed
    // error payload. This confirms the macro reports the concrete ID type.
    //
    // Given an empty string,
    // when `RelationshipId::new` is called,
    // then construction should fail with `GraphError::InvalidIdentifier("RelationshipId")`.
    #[test]
    fn relationship_id_rejects_empty_value() {
        let error = RelationshipId::new("").expect_err("empty relationship ID should be rejected");

        assert!(matches!(error, GraphError::InvalidIdentifier(kind) if kind == "RelationshipId"));
    }

    //
    // Verify that relationship IDs reject tab and newline-only values, not just
    // plain spaces. This documents that validation uses `trim().is_empty()`.
    //
    // Given a relationship ID made only of tab and newline characters,
    // when `RelationshipId::new` is called,
    // then construction should fail with `GraphError::InvalidIdentifier("RelationshipId")`.
    #[test]
    fn relationship_id_rejects_whitespace_only_value() {
        let error = RelationshipId::new("\t\n")
            .expect_err("whitespace-only relationship ID should be rejected");

        assert!(matches!(error, GraphError::InvalidIdentifier(kind) if kind == "RelationshipId"));
    }

    //
    // Verify the current normalization policy. The ID layer validates that the
    // string is meaningful, but it does not trim or rewrite the stored value.
    //
    // Given an ID value with surrounding spaces but non-whitespace content,
    // when the ID is constructed,
    // then `as_str` should return the original value exactly as provided.
    #[test]
    fn as_str_returns_the_original_value_without_normalization() {
        let id =
            NodeId::new(" node--with-surrounding-space ").expect("non-empty ID should be accepted");

        assert_eq!(id.as_str(), " node--with-surrounding-space ");
    }

    //
    // Verify that typed IDs can be used as keys in hash-based indexes. The graph
    // implementation will rely on this for maps keyed by stable node IDs.
    //
    // Given a valid `NodeId`,
    // when it is inserted into a `HashMap`,
    // then the same ID should retrieve the stored value.
    #[test]
    fn node_id_can_be_used_as_hash_map_key() {
        let id = NodeId::new("node--1").expect("valid node ID should be accepted");
        let mut map = HashMap::new();

        map.insert(id.clone(), "stored node");

        assert_eq!(map.get(&id), Some(&"stored node"));
    }
}
