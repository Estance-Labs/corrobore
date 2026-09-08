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
//! Derived content lineage (ADR-0021).
//!
//! A span addresses an extractor's output, not the original artifact. Retaining
//! the artifact proves what was ingested; retaining the derived content proves
//! what was *read*. Without the second, re-running a different extractor
//! version leaves the same offsets silently addressing different text.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ContentRef, ContentStore, ContentStoreError, EvidenceLocator, GraphError};

/// What an extraction produced from a source artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedContentKind {
    /// Text recovered from a document.
    ExtractedText,
    /// Structured records normalized from a document or feed.
    NormalizedRecords,
    /// A transcript recovered from audio or video.
    Transcript,
}

impl DerivedContentKind {
    /// Stable token used in identities and projections.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExtractedText => "extracted_text",
            Self::NormalizedRecords => "normalized_records",
            Self::Transcript => "transcript",
        }
    }
}

/// Qualified identity of the extractor that produced derived content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractorIdentity {
    id: String,
    version: String,
}

impl ExtractorIdentity {
    /// Name an extractor and the version that ran.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidPropertyValue`] when either part is empty:
    /// an unidentified extractor makes its own output unreproducible.
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Result<Self, GraphError> {
        let id = id.into();
        let version = version.into();
        if id.trim().is_empty() || version.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "extractor identity requires both an id and a version".to_owned(),
            ));
        }
        Ok(Self { id, version })
    }

    /// Extractor name.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Extractor version that produced the content.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Content produced from a source artifact by a named extractor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedContent {
    id: String,
    source_artifact: ContentRef,
    content: ContentRef,
    extractor: ExtractorIdentity,
    kind: DerivedContentKind,
}

impl DerivedContent {
    /// Record a derivation.
    ///
    /// The identity is computed from the provenance, so the same artifact
    /// extracted by the same extractor yields the same identity, and a
    /// different extractor yields a different one even when the bytes match.
    #[must_use]
    pub fn new(
        source_artifact: ContentRef,
        content: ContentRef,
        extractor: ExtractorIdentity,
        kind: DerivedContentKind,
    ) -> Self {
        let identity = format!(
            "{}|{}|{}|{}|{}",
            source_artifact.sha256(),
            extractor.id(),
            extractor.version(),
            kind.as_str(),
            content.sha256()
        );
        let digest: String = Sha256::digest(identity.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            id: format!("derived--{digest}"),
            source_artifact,
            content,
            extractor,
            kind,
        }
    }

    /// Rebind an identity to other content, bypassing provenance.
    ///
    /// Exists so tests can forge a contradicting record; the constructor offers
    /// no way to make an identity describe a different derivation.
    #[doc(hidden)]
    #[must_use]
    pub fn rebind_for_test(id: String, content: ContentRef) -> Self {
        Self {
            id,
            source_artifact: content.clone(),
            content,
            extractor: ExtractorIdentity {
                id: "forged".to_owned(),
                version: "0".to_owned(),
            },
            kind: DerivedContentKind::ExtractedText,
        }
    }

    /// Identity of this derivation.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Artifact this content was derived from.
    #[must_use]
    pub fn source_artifact(&self) -> &ContentRef {
        &self.source_artifact
    }

    /// The derived content itself.
    #[must_use]
    pub fn content(&self) -> &ContentRef {
        &self.content
    }

    /// Extractor that produced it.
    #[must_use]
    pub fn extractor(&self) -> &ExtractorIdentity {
        &self.extractor
    }

    /// What the extraction produced.
    #[must_use]
    pub fn kind(&self) -> DerivedContentKind {
        self.kind
    }
}

/// Append-only record of what each extraction produced.
#[derive(Clone, Debug, Default)]
pub struct DerivedContentStore {
    records: BTreeMap<String, DerivedContent>,
}

impl DerivedContentStore {
    /// Record a derivation.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::ImmutableRecordConflict`] when the identity is
    /// already recorded against a different derivation: the identity comes from
    /// the provenance, so a contradicting record would rewrite history.
    pub fn record(&mut self, derived: DerivedContent) -> Result<(), GraphError> {
        if let Some(existing) = self.records.get(derived.id())
            && existing != &derived
        {
            return Err(GraphError::ImmutableRecordConflict {
                kind: crate::ImmutableRecordKind::DerivedContent,
                id: derived.id().to_owned(),
            });
        }
        self.records.insert(derived.id().to_owned(), derived);
        Ok(())
    }

    /// Look up a derivation without transferring any content.
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&DerivedContent> {
        self.records.get(id)
    }

    /// Number of recorded derivations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Why a span could not be resolved against its derived content.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpanResolutionError {
    /// The selector does not address an offset into the derived bytes.
    #[error("selector {selector} does not address an offset in derived content")]
    NotAddressable {
        /// Rendered selector that could not be resolved.
        selector: String,
    },

    /// The span lies outside the derived content.
    #[error("span {start}..{end} lies outside derived content")]
    OutOfBounds {
        /// First requested position.
        start: u64,
        /// End of the requested span, exclusive.
        end: u64,
    },

    /// The derived content could not be retrieved.
    #[error("derived content unavailable: {0}")]
    Unavailable(ContentStoreError),

    /// The derived content is not valid text for a character span.
    #[error("derived content is not text, so a character span cannot address it")]
    NotText,
}

/// Resolve a span against the derived content it was created from.
///
/// A byte range is served through a ranged read. A character span is not a byte
/// offset, so it requires the content: treating character offsets as bytes
/// would split a character and silently return different text.
///
/// # Errors
///
/// Returns [`SpanResolutionError`] rather than a plausible-looking answer for a
/// selector that does not address bytes, a span outside the content, content
/// that cannot be retrieved, or a character span over non-text.
pub fn resolve_derived_span<S: ContentStore>(
    derived: &DerivedContent,
    selector: &EvidenceLocator,
    store: &S,
) -> Result<Vec<u8>, SpanResolutionError> {
    match selector {
        EvidenceLocator::ByteRange { start, end } => {
            if start > end || *end > derived.content().byte_length() {
                return Err(SpanResolutionError::OutOfBounds {
                    start: *start,
                    end: *end,
                });
            }
            store
                .load_range(derived.content(), *start..*end)
                .map_err(SpanResolutionError::Unavailable)
        }
        EvidenceLocator::CharacterSpan { start, end } => {
            let bytes = store
                .load(derived.content())
                .map_err(SpanResolutionError::Unavailable)?;
            let text = String::from_utf8(bytes).map_err(|_| SpanResolutionError::NotText)?;
            let start = usize::try_from(*start).unwrap_or(usize::MAX);
            let end = usize::try_from(*end).unwrap_or(usize::MAX);
            if start > end || end > text.chars().count() {
                return Err(SpanResolutionError::OutOfBounds {
                    start: start as u64,
                    end: end as u64,
                });
            }
            Ok(text
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
                .into_bytes())
        }
        other => Err(SpanResolutionError::NotAddressable {
            selector: other.render(),
        }),
    }
}
