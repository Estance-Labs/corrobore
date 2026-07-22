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
use graph_core::{GraphError, TemporalMetadata, TemporalTimestamp};

//
// Verify typed temporal wrappers accept valid RFC3339 timestamps and preserve
// exact string values for storage-level compatibility.
#[test]
fn temporal_timestamp_accepts_valid_rfc3339_value() {
    let timestamp = TemporalTimestamp::new("2026-07-06T14:33:45Z")
        .expect("valid RFC3339 timestamp should be accepted");

    assert_eq!(timestamp.as_str(), "2026-07-06T14:33:45Z");
}

//
// Verify invalid timestamp strings are rejected with explicit typed errors.
#[test]
fn temporal_timestamp_rejects_invalid_rfc3339_value() {
    let error = TemporalTimestamp::new("2026-07-06 14:33:45")
        .expect_err("invalid RFC3339 timestamp should be rejected");

    assert!(matches!(
    error,
    GraphError::InvalidPropertyValue(message)
    if message == "invalid RFC3339 timestamp: 2026-07-06 14:33:45"
    ));
}

//
// Verify first_seen and last_seen ordering is deterministic and accepted when
// first_seen is before or equal to last_seen.
#[test]
fn temporal_metadata_validate_accepts_first_seen_before_last_seen() {
    let metadata = TemporalMetadata::default()
        .with_first_seen(
            TemporalTimestamp::new("2026-07-01T00:00:00Z").expect("timestamp should be valid"),
        )
        .with_last_seen(
            TemporalTimestamp::new("2026-07-06T00:00:00Z").expect("timestamp should be valid"),
        );

    assert_eq!(metadata.validate_semantics(), Ok(()));
}

//
// Verify deterministic temporal ordering rejects invalid first_seen > last_seen.
#[test]
fn temporal_metadata_validate_rejects_first_seen_after_last_seen() {
    let metadata = TemporalMetadata::default()
        .with_first_seen(
            TemporalTimestamp::new("2026-07-06T00:00:00Z").expect("timestamp should be valid"),
        )
        .with_last_seen(
            TemporalTimestamp::new("2026-07-01T00:00:00Z").expect("timestamp should be valid"),
        );

    let error = metadata
        .validate_semantics()
        .expect_err("first_seen after last_seen should be rejected");

    assert!(matches!(
    error,
    GraphError::InvalidPropertyValue(message)
    if message == "invalid temporal ordering: first_seen must be <= last_seen"
    ));
}

//
// Verify valid_from and valid_until ordering is accepted for monotonic ranges.
#[test]
fn temporal_metadata_validate_accepts_valid_from_before_valid_until() {
    let metadata = TemporalMetadata::default()
        .with_valid_from(
            TemporalTimestamp::new("2026-07-01T00:00:00Z").expect("timestamp should be valid"),
        )
        .with_valid_until(
            TemporalTimestamp::new("2026-07-31T00:00:00Z").expect("timestamp should be valid"),
        );

    assert_eq!(metadata.validate_semantics(), Ok(()));
}

//
// Verify valid_from and valid_until ordering is rejected when interval bounds are inverted.
#[test]
fn temporal_metadata_validate_rejects_valid_from_after_valid_until() {
    let metadata = TemporalMetadata::default()
        .with_valid_from(
            TemporalTimestamp::new("2026-07-31T00:00:00Z").expect("timestamp should be valid"),
        )
        .with_valid_until(
            TemporalTimestamp::new("2026-07-01T00:00:00Z").expect("timestamp should be valid"),
        );

    let error = metadata
        .validate_semantics()
        .expect_err("valid_from after valid_until should be rejected");

    assert!(matches!(
    error,
    GraphError::InvalidPropertyValue(message)
    if message == "invalid temporal ordering: valid_from must be <= valid_until"
    ));
}

//
// Verify timestamp parser accepts explicit timezone offsets in RFC3339-compatible shape.
#[test]
fn temporal_timestamp_accepts_rfc3339_with_timezone_offset() {
    let timestamp = TemporalTimestamp::new("2026-07-06T14:33:45+02:00")
        .expect("timestamp with timezone offset should be accepted");

    assert_eq!(timestamp.as_str(), "2026-07-06T14:33:45+02:00");
}

//
// Verify malformed short or separator-invalid timestamps are rejected deterministically.
#[test]
fn temporal_timestamp_rejects_short_and_separator_invalid_values() {
    let short_error =
        TemporalTimestamp::new("2026-07-06T14:33").expect_err("short timestamp should be rejected");
    assert!(matches!(
    short_error,
    GraphError::InvalidPropertyValue(message)
    if message == "invalid RFC3339 timestamp: 2026-07-06T14:33"
    ));

    let separator_error = TemporalTimestamp::new("2026/07/06T14:33:45Z")
        .expect_err("timestamp with invalid separators should be rejected");
    assert!(matches!(
    separator_error,
    GraphError::InvalidPropertyValue(message)
    if message == "invalid RFC3339 timestamp: 2026/07/06T14:33:45Z"
    ));
}

//
// Verify metadata builder methods preserve exact string values for storage-facing fields.
#[test]
fn temporal_metadata_builder_methods_store_expected_fields() {
    let recorded_at = TemporalTimestamp::new("2026-07-06T10:00:00Z").expect("valid timestamp");
    let observed_at = TemporalTimestamp::new("2026-07-06T11:00:00Z").expect("valid timestamp");
    let superseded_at = TemporalTimestamp::new("2026-07-06T12:00:00Z").expect("valid timestamp");

    let metadata = TemporalMetadata::default()
        .with_recorded_at(recorded_at)
        .with_observed_at(observed_at)
        .with_superseded_at(superseded_at);

    assert_eq!(
        metadata.recorded_at.as_deref(),
        Some("2026-07-06T10:00:00Z")
    );
    assert_eq!(
        metadata.observed_at.as_deref(),
        Some("2026-07-06T11:00:00Z")
    );
    assert_eq!(
        metadata.superseded_at.as_deref(),
        Some("2026-07-06T12:00:00Z")
    );
}
