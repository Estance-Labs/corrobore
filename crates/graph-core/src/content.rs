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
//! Content handles and the inline-or-offloaded storage policy (ADR-0021).
//!
//! A handle describes content whether the bytes sit inline or in a content
//! store, so a reader never needs the bytes to describe what it is looking at.
//! The digest of inline content is derived rather than declared, so a handle
//! cannot disagree with the bytes it carries.
//!
//! The placement decision belongs to the engine under a policy whose identity
//! is retained with the content: a configurable threshold that is not recorded
//! makes a store's layout irreproducible.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::GraphError;

/// Media types whose content may be held inline when it is small enough.
const INLINE_ELIGIBLE_PREFIXES: [&str; 2] = ["text/", "application/json"];

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Content small enough to travel with the record that owns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineContent {
    /// UTF-8 text, such as an evidence span.
    Text(String),
    /// Structured content, such as a normalized record.
    Json(serde_json::Value),
    /// Opaque bytes.
    Bytes(Vec<u8>),
}

impl InlineContent {
    /// Canonical bytes of this content, as digested and measured.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Text(text) => text.as_bytes().to_vec(),
            // Serialization is canonical for a given value, so the digest is stable.
            Self::Json(value) => serde_json::to_vec(value).unwrap_or_default(),
            Self::Bytes(bytes) => bytes.clone(),
        }
    }

    /// Media type this content is held as.
    #[must_use]
    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "text/plain",
            Self::Json(_) => "application/json",
            Self::Bytes(_) => "application/octet-stream",
        }
    }
}

/// Reference to content held in a content store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRef {
    content_id: String,
    sha256: String,
    byte_length: u64,
    backend: String,
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_type: Option<String>,
}

impl ContentRef {
    /// Build a reference to stored content.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidPropertyValue`] when the identity, digest,
    /// backend or key is missing or malformed.
    pub fn new(
        content_id: impl Into<String>,
        sha256: impl Into<String>,
        byte_length: u64,
        backend: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let content_id = content_id.into();
        let sha256 = sha256.into();
        let backend = backend.into();
        let key = key.into();
        if content_id.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "content reference identity must not be empty".to_owned(),
            ));
        }
        if !is_sha256(&sha256) {
            return Err(GraphError::InvalidPropertyValue(
                "content reference sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        if backend.trim().is_empty() || key.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "content reference backend and key must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            content_id,
            sha256,
            byte_length,
            backend,
            key,
            media_type: None,
        })
    }

    /// Declare the media type of the referenced content.
    #[must_use]
    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }

    /// Declare an optional media type, leaving it absent when unknown.
    #[must_use]
    pub fn with_media_type_opt(self, media_type: Option<&str>) -> Self {
        match media_type {
            Some(media_type) => self.with_media_type(media_type),
            None => self,
        }
    }

    /// Store-assigned identity of the content.
    #[must_use]
    pub fn content_id(&self) -> &str {
        &self.content_id
    }

    /// Digest of the referenced bytes.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Length of the referenced bytes.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }

    /// Backend holding the content.
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// Backend-scoped key of the content.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Media type of the referenced content, when known.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }
}

/// Content owned by a record, held inline or in a content store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentHandle {
    /// Bytes carried with the record.
    Inline(InlineContent),
    /// Bytes held in a content store.
    External(ContentRef),
}

impl ContentHandle {
    /// Hold content inline.
    #[must_use]
    pub fn inline(content: InlineContent) -> Self {
        Self::Inline(content)
    }

    /// Digest of the content, derived for inline content and declared for a
    /// reference whose bytes are not present to be measured.
    #[must_use]
    pub fn sha256(&self) -> String {
        match self {
            Self::Inline(content) => hex_digest(&content.to_bytes()),
            Self::External(reference) => reference.sha256().to_owned(),
        }
    }

    /// Length of the content in bytes.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        match self {
            Self::Inline(content) => content.to_bytes().len() as u64,
            Self::External(reference) => reference.byte_length(),
        }
    }

    /// Media type of the content, when known.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        match self {
            Self::Inline(content) => Some(content.media_type()),
            Self::External(reference) => reference.media_type(),
        }
    }

    /// Bytes of inline content, absent when the content is offloaded.
    #[must_use]
    pub fn inline_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Inline(content) => Some(content.to_bytes()),
            Self::External(_) => None,
        }
    }
}

/// Where a policy places content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentPlacement {
    /// Held with the record that owns it.
    Inline,
    /// Held in a content store.
    Offloaded,
}

/// Rule deciding whether content travels with its record or is offloaded.
///
/// The identity is retained with the content it placed, so two stores holding
/// identical content but laid out differently can explain why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentStoragePolicy {
    version: String,
    inline_max_bytes: u64,
}

impl ContentStoragePolicy {
    /// Build a storage policy.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidPropertyValue`] when the policy has no
    /// identity to retain, or admits no inline content at all.
    pub fn new(version: impl Into<String>, inline_max_bytes: u64) -> Result<Self, GraphError> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "content storage policy must declare a version".to_owned(),
            ));
        }
        if inline_max_bytes == 0 {
            return Err(GraphError::InvalidPropertyValue(
                "content storage policy inline maximum must be positive".to_owned(),
            ));
        }
        Ok(Self {
            version,
            inline_max_bytes,
        })
    }

    /// Identity retained with the content this policy placed.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Largest content held inline.
    #[must_use]
    pub fn inline_max_bytes(&self) -> u64 {
        self.inline_max_bytes
    }

    /// Decide where content of this type and size belongs.
    ///
    /// Unlabelled content is offloaded: guessing that unknown bytes are text
    /// would inline arbitrary binary.
    #[must_use]
    pub fn placement(&self, media_type: Option<&str>, byte_length: u64) -> ContentPlacement {
        let inline_eligible = media_type.is_some_and(|media_type| {
            INLINE_ELIGIBLE_PREFIXES
                .iter()
                .any(|prefix| media_type.starts_with(prefix))
        });
        if inline_eligible && byte_length <= self.inline_max_bytes {
            ContentPlacement::Inline
        } else {
            ContentPlacement::Offloaded
        }
    }
}

/// Where the engine placed content, and under which policy.
///
/// The policy identity travels with the outcome: a configurable threshold that
/// is not recorded makes a store's layout irreproducible, so a caller can
/// retain why this content sits where it does.
///
/// Deliberately not deserializable: a decision is what [`ingest_content`]
/// returned, not a shape a caller can assemble to claim a policy it never ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentPlacementDecision {
    handle: ContentHandle,
    placement: ContentPlacement,
    policy_version: String,
}

impl ContentPlacementDecision {
    /// Handle naming the content.
    #[must_use]
    pub fn handle(&self) -> &ContentHandle {
        &self.handle
    }

    /// Consume the decision, keeping only the handle.
    #[must_use]
    pub fn into_handle(self) -> ContentHandle {
        self.handle
    }

    /// Where the content was placed.
    #[must_use]
    pub fn placement(&self) -> ContentPlacement {
        self.placement
    }

    /// Identity of the policy that decided.
    #[must_use]
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
}

/// Ingest content, letting the policy decide where it belongs.
///
/// The caller supplies bytes and what they are, never where they should live.
/// Content the policy keeps inline never reaches the store, so a short span
/// costs no round trip.
///
/// # Errors
///
/// Returns [`ContentStoreError::Invalid`] for empty content, and propagates a
/// store failure rather than falling back to inline: falling back would place
/// content the policy said to offload, and the record would claim an inline
/// copy nobody decided to keep.
pub fn ingest_content<S: crate::ContentStore>(
    bytes: &[u8],
    media_type: Option<&str>,
    policy: &ContentStoragePolicy,
    store: &mut S,
) -> Result<ContentPlacementDecision, crate::ContentStoreError> {
    if bytes.is_empty() {
        return Err(crate::ContentStoreError::Invalid(
            "content must not be empty".to_owned(),
        ));
    }

    let placement = policy.placement(media_type, bytes.len() as u64);
    let handle = match placement {
        ContentPlacement::Inline => ContentHandle::inline(inline_content(bytes, media_type)?),
        ContentPlacement::Offloaded => ContentHandle::External(store.store(bytes, media_type)?),
    };
    Ok(ContentPlacementDecision {
        handle,
        placement,
        policy_version: policy.version().to_owned(),
    })
}

/// Interpret inline bytes as what their media type says they are.
fn inline_content(
    bytes: &[u8],
    media_type: Option<&str>,
) -> Result<InlineContent, crate::ContentStoreError> {
    let text = || {
        std::str::from_utf8(bytes).map_err(|_| {
            crate::ContentStoreError::Invalid(
                "content declared as text is not valid UTF-8".to_owned(),
            )
        })
    };
    // Matched by prefix like the policy does, so a parameterised
    // `application/json; charset=utf-8` is not silently demoted to text.
    if media_type.is_some_and(|media_type| media_type.starts_with("application/json")) {
        let value = serde_json::from_slice(bytes).map_err(|error| {
            crate::ContentStoreError::Invalid(format!(
                "content declared as JSON does not parse: {error}"
            ))
        })?;
        return Ok(InlineContent::Json(value));
    }
    Ok(InlineContent::Text(text()?.to_owned()))
}
