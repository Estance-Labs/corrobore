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
//! Lazy hydration of content handles (ADR-0021).
//!
//! ```text
//! METADATA  →  PREVIEW  →  FULL CONTENT
//! ```
//!
//! A caller pays for what it reads, not for what it traverses. Describing a
//! handle transfers nothing; a ranged request transfers only that range.
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{ContentHandle, ContentStatus, ContentStore, ContentStoreError};

/// What a handle discloses without its bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDisclosure {
    content_id: Option<String>,
    sha256: String,
    byte_length: u64,
    media_type: Option<String>,
    inline: bool,
    erased: bool,
}

impl ContentDisclosure {
    /// Store-assigned identity, absent for inline content.
    #[must_use]
    pub fn content_id(&self) -> Option<&str> {
        self.content_id.as_deref()
    }

    /// Digest of the content.
    #[must_use]
    pub fn sha256(&self) -> String {
        self.sha256.clone()
    }

    /// Length of the content.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }

    /// Media type, when known.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    /// Whether the bytes travel with the record.
    #[must_use]
    pub fn is_inline(&self) -> bool {
        self.inline
    }

    /// Whether the content was retained and has since been erased.
    #[must_use]
    pub fn is_erased(&self) -> bool {
        self.erased
    }
}

/// Why content could not be hydrated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HydrationError {
    /// The store could not serve the content.
    #[error("{0}")]
    Unavailable(ContentStoreError),

    /// The requested window lies outside the content.
    #[error("range {start}..{end} lies outside content of {byte_length} bytes")]
    RangeOutOfBounds {
        /// First requested byte.
        start: u64,
        /// End of the requested window, exclusive.
        end: u64,
        /// Length of the content.
        byte_length: u64,
    },
}

/// Describe a handle without transferring its content.
///
/// # Errors
///
/// Returns [`HydrationError::Unavailable`] when the store never retained the
/// content, so an unknown reference is not described as if it were present.
pub fn describe_handle<S: ContentStore>(
    handle: &ContentHandle,
    store: &S,
) -> Result<ContentDisclosure, HydrationError> {
    match handle {
        ContentHandle::Inline(content) => Ok(ContentDisclosure {
            content_id: None,
            sha256: handle.sha256(),
            byte_length: handle.byte_length(),
            media_type: Some(content.media_type().to_owned()),
            inline: true,
            erased: false,
        }),
        ContentHandle::External(reference) => {
            let status = store
                .describe(reference)
                .map_err(HydrationError::Unavailable)?;
            // A tombstone still describes what was there, which is its point.
            let (byte_length, erased) = match status {
                ContentStatus::Present { byte_length } => (byte_length, false),
                ContentStatus::Erased { byte_length, .. } => (byte_length, true),
            };
            Ok(ContentDisclosure {
                content_id: Some(reference.content_id().to_owned()),
                sha256: reference.sha256().to_owned(),
                byte_length,
                media_type: reference.media_type().map(ToOwned::to_owned),
                inline: false,
                erased,
            })
        }
    }
}

/// Hydrate the whole content behind a handle.
///
/// # Errors
///
/// Returns [`HydrationError::Unavailable`] rather than empty content, because
/// empty content would read as an absence of evidence.
pub fn hydrate_handle<S: ContentStore>(
    handle: &ContentHandle,
    store: &S,
) -> Result<Vec<u8>, HydrationError> {
    match handle {
        // Already present, so no store is consulted.
        ContentHandle::Inline(content) => Ok(content.to_bytes()),
        ContentHandle::External(reference) => {
            store.load(reference).map_err(HydrationError::Unavailable)
        }
    }
}

/// Hydrate a window of the content behind a handle.
///
/// # Errors
///
/// Returns [`HydrationError::RangeOutOfBounds`] rather than clamping, which
/// would answer a different question convincingly.
pub fn hydrate_handle_range<S: ContentStore>(
    handle: &ContentHandle,
    store: &S,
    range: Range<u64>,
) -> Result<Vec<u8>, HydrationError> {
    match handle {
        ContentHandle::Inline(content) => {
            let bytes = content.to_bytes();
            let byte_length = bytes.len() as u64;
            if range.start > range.end || range.end > byte_length {
                return Err(HydrationError::RangeOutOfBounds {
                    start: range.start,
                    end: range.end,
                    byte_length,
                });
            }
            let start = usize::try_from(range.start).unwrap_or(usize::MAX);
            let end = usize::try_from(range.end).unwrap_or(usize::MAX);
            Ok(bytes[start..end].to_vec())
        }
        ContentHandle::External(reference) => store
            .load_range(reference, range)
            .map_err(HydrationError::Unavailable),
    }
}
