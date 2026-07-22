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
//! Temporal metadata shape for graph-core records.
//!
//! Module boundary:
//! this module owns neutral timestamp slots carried by graph records. It does
//! not own timestamp parsing, clock policy, retention rules, chronology diffing,
//! or domain-specific temporal interpretation.

use serde::{Deserialize, Serialize};

use crate::GraphError;

/// Typed RFC3339 timestamp wrapper for temporal metadata fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalTimestamp(String);

impl TemporalTimestamp {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();

        if !is_valid_rfc3339_utc(&value) {
            return Err(GraphError::InvalidPropertyValue(format!(
                "invalid RFC3339 timestamp: {}",
                value
            )));
        }

        Ok(Self(value))
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Optional temporal fields attached to nodes and relationships.
///
/// The fields are stored as strings at this layer so graph-core can preserve
/// caller-provided temporal metadata without committing to a parsing,
/// timezone, or normalization policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalMetadata {
    /// Time when the record version was created by the producing layer.
    pub created_at: Option<String>,
    /// Time when the record version was last updated by the producing layer.
    pub updated_at: Option<String>,
    /// Time when this metadata was recorded.
    pub recorded_at: Option<String>,
    /// Time when this version was superseded by a later version.
    pub superseded_at: Option<String>,
    /// Time when the described fact or observation occurred.
    pub observed_at: Option<String>,
    /// First known sighting time for the described entity or relationship.
    pub first_seen: Option<String>,
    /// Last known sighting time for the described entity or relationship.
    pub last_seen: Option<String>,
    /// Start of the validity interval for the described fact.
    pub valid_from: Option<String>,
    /// End of the validity interval for the described fact.
    pub valid_until: Option<String>,
}

impl TemporalMetadata {
    /// Sets the recorded at.
    pub fn with_recorded_at(mut self, timestamp: TemporalTimestamp) -> Self {
        self.recorded_at = Some(timestamp.0);
        self
    }

    /// Sets the observed at.
    pub fn with_observed_at(mut self, timestamp: TemporalTimestamp) -> Self {
        self.observed_at = Some(timestamp.0);
        self
    }

    /// Sets the first seen.
    pub fn with_first_seen(mut self, timestamp: TemporalTimestamp) -> Self {
        self.first_seen = Some(timestamp.0);
        self
    }

    /// Sets the last seen.
    pub fn with_last_seen(mut self, timestamp: TemporalTimestamp) -> Self {
        self.last_seen = Some(timestamp.0);
        self
    }

    /// Sets the valid from.
    pub fn with_valid_from(mut self, timestamp: TemporalTimestamp) -> Self {
        self.valid_from = Some(timestamp.0);
        self
    }

    /// Sets the valid until.
    pub fn with_valid_until(mut self, timestamp: TemporalTimestamp) -> Self {
        self.valid_until = Some(timestamp.0);
        self
    }

    /// Sets the superseded at.
    pub fn with_superseded_at(mut self, timestamp: TemporalTimestamp) -> Self {
        self.superseded_at = Some(timestamp.0);
        self
    }

    /// Validates the semantics.
    pub fn validate_semantics(&self) -> Result<(), GraphError> {
        if let (Some(first_seen), Some(last_seen)) = (&self.first_seen, &self.last_seen)
            && first_seen > last_seen
        {
            return Err(GraphError::InvalidPropertyValue(
                "invalid temporal ordering: first_seen must be <= last_seen".to_owned(),
            ));
        }

        if let (Some(valid_from), Some(valid_until)) = (&self.valid_from, &self.valid_until)
            && valid_from > valid_until
        {
            return Err(GraphError::InvalidPropertyValue(
                "invalid temporal ordering: valid_from must be <= valid_until".to_owned(),
            ));
        }

        Ok(())
    }
}

fn is_valid_rfc3339_utc(value: &str) -> bool {
    if value.len() < 20 {
        return false;
    }

    let bytes = value.as_bytes();

    let date_separators_ok = bytes.get(4) == Some(&b'-') && bytes.get(7) == Some(&b'-');
    let time_separator_ok = bytes.get(10) == Some(&b'T');
    let clock_separators_ok = bytes.get(13) == Some(&b':') && bytes.get(16) == Some(&b':');

    if !(date_separators_ok && time_separator_ok && clock_separators_ok) {
        return false;
    }

    let base_digits_ok = bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| ![4, 7, 10, 13, 16].contains(index))
        .take(14)
        .all(|(_, byte)| byte.is_ascii_digit());

    if !base_digits_ok {
        return false;
    }

    let remainder = &value[19..];
    remainder == "Z"
        || remainder.starts_with('Z')
        || remainder.starts_with('+')
        || remainder.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    //
    // Verify typed timestamp wrapper accepts canonical RFC3339 UTC input.
    #[test]
    fn temporal_timestamp_accepts_rfc3339_utc() {
        let timestamp = TemporalTimestamp::new("2026-07-06T14:33:45Z")
            .expect("valid RFC3339 timestamp should be accepted");

        assert_eq!(timestamp.as_str(), "2026-07-06T14:33:45Z");
    }

    //
    // Verify typed timestamp wrapper rejects malformed timestamp values.
    #[test]
    fn temporal_timestamp_rejects_malformed_value() {
        let error = TemporalTimestamp::new("2026-07-06 14:33:45")
            .expect_err("invalid timestamp should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message == "invalid RFC3339 timestamp: 2026-07-06 14:33:45"
        ));
    }

    //
    // Verify first_seen and last_seen ordering is deterministic and enforced.
    #[test]
    fn temporal_metadata_rejects_first_seen_after_last_seen() {
        let metadata = TemporalMetadata::default()
            .with_first_seen(
                TemporalTimestamp::new("2026-07-07T00:00:00Z").expect("timestamp should be valid"),
            )
            .with_last_seen(
                TemporalTimestamp::new("2026-07-06T00:00:00Z").expect("timestamp should be valid"),
            );

        let error = metadata
            .validate_semantics()
            .expect_err("invalid ordering should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message == "invalid temporal ordering: first_seen must be <= last_seen"
        ));
    }
}
