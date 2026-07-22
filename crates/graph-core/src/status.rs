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
//! Record lifecycle status primitive for graph-core records.
//!
//! Module boundary:
//! this module owns the neutral lifecycle states that nodes and relationships can
//! carry. It must not own workflow policy, analyst queues, export routing, CTI
//! rules, FIMI rules, or crisis-specific escalation semantics.

use serde::{Deserialize, Serialize};

/// Lifecycle status attached to nodes and relationships.
///
/// Status values describe graph-core record state only. Higher-level workflows
/// may interpret these values, but policy-specific behavior belongs outside this
/// crate boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordStatus {
    /// Newly created or provisional record.
    Candidate,
    /// Record requires additional supporting evidence.
    NeedsEvidence,
    /// Record requires analyst review before it is treated as validated.
    NeedsReview,
    /// Record has been validated inside the graph-core lifecycle model.
    Validated,
    /// Record has been rejected inside the graph-core lifecycle model.
    Rejected,
    /// Record is ready to be exported by an outer layer.
    Exportable,
    /// Record has been exported by an outer layer.
    Exported,
    /// Record is logically deleted but remains available in version history.
    Tombstoned,
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    //
    // Verify that candidate is available as the default early lifecycle state for
    // graph records created from analyst or extraction input.
    //
    // Given `RecordStatus::Candidate`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_candidate() {
        assert_eq!(RecordStatus::Candidate, RecordStatus::Candidate);
    }

    //
    // Verify that records can explicitly represent the need for more evidence.
    //
    // Given `RecordStatus::NeedsEvidence`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_needs_evidence() {
        assert_eq!(RecordStatus::NeedsEvidence, RecordStatus::NeedsEvidence);
    }

    //
    // Verify that records can explicitly represent the need for analyst review.
    //
    // Given `RecordStatus::NeedsReview`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_needs_review() {
        assert_eq!(RecordStatus::NeedsReview, RecordStatus::NeedsReview);
    }

    //
    // Verify that records can explicitly represent validated intelligence.
    //
    // Given `RecordStatus::Validated`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_validated() {
        assert_eq!(RecordStatus::Validated, RecordStatus::Validated);
    }

    //
    // Verify that records can explicitly represent rejected intelligence.
    //
    // Given `RecordStatus::Rejected`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_rejected() {
        assert_eq!(RecordStatus::Rejected, RecordStatus::Rejected);
    }

    //
    // Verify that records can explicitly represent intelligence ready for export.
    //
    // Given `RecordStatus::Exportable`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_exportable() {
        assert_eq!(RecordStatus::Exportable, RecordStatus::Exportable);
    }

    //
    // Verify that records can explicitly represent intelligence already exported.
    //
    // Given `RecordStatus::Exported`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_exported() {
        assert_eq!(RecordStatus::Exported, RecordStatus::Exported);
    }

    //
    // Verify that records can explicitly represent tombstoned lifecycle state.
    //
    // Given `RecordStatus::Tombstoned`,
    // when it is compared with the same variant,
    // then equality should hold.
    #[test]
    fn record_status_supports_tombstoned() {
        assert_eq!(RecordStatus::Tombstoned, RecordStatus::Tombstoned);
    }

    //
    // Verify that record statuses are cheap value objects. Status is copied from
    // inputs into graph records and patches, so it should not require ownership
    // choreography.
    //
    // Given a `RecordStatus`,
    // when it is assigned to another variable,
    // then both values should remain usable and equal.
    #[test]
    fn record_status_is_copyable() {
        let status = RecordStatus::Validated;
        let copied = status;

        assert_eq!(status, copied);
    }

    //
    // Verify that lifecycle variants remain semantically distinct. This protects
    // tests and callers from accidentally treating unrelated lifecycle states as
    // equivalent.
    //
    // Given different `RecordStatus` variants,
    // when they are compared,
    // then unrelated lifecycle states should not be equal.
    #[test]
    fn record_status_variants_are_distinct() {
        assert_ne!(RecordStatus::Candidate, RecordStatus::Validated);
        assert_ne!(RecordStatus::Rejected, RecordStatus::Exportable);
        assert_ne!(RecordStatus::Exported, RecordStatus::Tombstoned);
    }

    //
    // Verify that record lifecycle statuses satisfy serde contracts at the unit
    // level. Graph records carry `RecordStatus`, so persistence and API layers
    // need every status to remain serializable and deserializable.
    //
    // Given the `RecordStatus` enum,
    // when serde trait bounds are required,
    // then `RecordStatus` should satisfy both `Serialize` and `Deserialize`.
    #[test]
    fn record_status_is_serializable() {
        fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

        assert_serializable::<RecordStatus>();
    }
}
