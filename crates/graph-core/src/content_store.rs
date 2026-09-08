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
//! The content store and its object-store backends (ADR-0021).
//!
//! Content is addressed by its digest, so storing the same bytes twice stores
//! them once and no identity can name different bytes. Retrieval verifies what
//! it read, because content served as evidence must be the content the
//! reference named.
//!
//! `ContentStore` has no `delete`. Erasure is a recorded operation leaving a
//! tombstone, so erased content is retrievable *as erased* — a different answer
//! from content that was never retained. An evidence store that could forget
//! silently would undermine the audit position the graph is built on.
use std::collections::BTreeMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ContentRef;

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Why a content operation could not be served.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContentStoreError {
    /// The content was never retained by this store.
    #[error("content {0} was never retained")]
    NotRetained(String),

    /// The content was retained and has since been erased.
    #[error("content {content_id} was erased: {reason}")]
    Erased {
        /// Identity of the erased content.
        content_id: String,
        /// Why the erasure was performed.
        reason: String,
    },

    /// The retrieved bytes do not hash to the digest that named them.
    #[error("content {content_id} does not match its digest")]
    DigestMismatch {
        /// Identity of the content whose bytes disagreed.
        content_id: String,
    },

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

    /// The reference names a different store.
    #[error("content {content_id} belongs to backend {reference_backend}, not {store_backend}")]
    ForeignBackend {
        /// Identity of the referenced content.
        content_id: String,
        /// Backend the reference names.
        reference_backend: String,
        /// Backend asked to serve it.
        store_backend: String,
    },

    /// The operation was malformed.
    #[error("{0}")]
    Invalid(String),

    /// The backend could not serve the operation.
    #[error("content backend failure: {0}")]
    Backend(String),
}

/// Byte-level object storage, shared by the content plane and by snapshots.
///
/// This is the physical layer. It has no opinion about what the bytes mean, so
/// `remove` is a byte operation; the recorded erasure semantics live in
/// [`ContentStore`].
pub trait ObjectStore {
    /// Write bytes at a key.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::Backend`] when the write cannot complete.
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError>;

    /// Read all bytes at a key.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the key is absent.
    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError>;

    /// Read a window of the bytes at a key.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the key is absent, and
    /// [`ContentStoreError::RangeOutOfBounds`] when the window is not wholly
    /// inside the object.
    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ContentStoreError>;

    /// Length of the object at a key, without transferring it.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the key is absent.
    fn head(&self, key: &str) -> Result<u64, ContentStoreError>;

    /// Remove the bytes at a key.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the key is absent.
    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError>;
}

/// In-process object store.
///
/// Durable backends live outside `graph-core`, which states storage-neutral
/// contracts without implementing storage.
#[derive(Clone, Debug, Default)]
pub struct MemoryObjectStore {
    objects: BTreeMap<String, Vec<u8>>,
}

impl MemoryObjectStore {
    /// Number of objects currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the store holds no objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Replace the bytes at a key, bypassing content addressing.
    ///
    /// Exists so tests can simulate a corrupted or substituted object; the
    /// content contract offers no way to make an identity name other bytes.
    #[doc(hidden)]
    pub fn substitute(&mut self, key: &str, bytes: &[u8]) {
        self.objects.insert(key.to_owned(), bytes.to_vec());
    }
}

fn window(bytes: &[u8], range: Range<u64>) -> Result<Vec<u8>, ContentStoreError> {
    let byte_length = bytes.len() as u64;
    if range.start > range.end || range.end > byte_length {
        return Err(ContentStoreError::RangeOutOfBounds {
            start: range.start,
            end: range.end,
            byte_length,
        });
    }
    let start = usize::try_from(range.start).unwrap_or(usize::MAX);
    let end = usize::try_from(range.end).unwrap_or(usize::MAX);
    Ok(bytes[start..end].to_vec())
}

impl ObjectStore for MemoryObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        self.objects.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        self.objects
            .get(key)
            .cloned()
            .ok_or_else(|| ContentStoreError::NotRetained(key.to_owned()))
    }

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ContentStoreError> {
        let bytes = self
            .objects
            .get(key)
            .ok_or_else(|| ContentStoreError::NotRetained(key.to_owned()))?;
        window(bytes, range)
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        self.objects
            .get(key)
            .map(|bytes| bytes.len() as u64)
            .ok_or_else(|| ContentStoreError::NotRetained(key.to_owned()))
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        self.objects
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| ContentStoreError::NotRetained(key.to_owned()))
    }
}

/// Whether a store currently holds content, and what it says if it does not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentStatus {
    /// The bytes are present.
    Present {
        /// Length of the content.
        byte_length: u64,
    },
    /// The bytes were retained and have been erased.
    Erased {
        /// Length the content had.
        byte_length: u64,
        /// Why the erasure was performed.
        reason: String,
    },
}

/// Content-addressed storage of evidence content.
pub trait ContentStore {
    /// Retain content, returning the reference that names it.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::Invalid`] for empty content, and a backend
    /// error when the write cannot complete.
    fn store(
        &mut self,
        bytes: &[u8],
        media_type: Option<&str>,
    ) -> Result<ContentRef, ContentStoreError>;

    /// Retrieve content, verified against the reference that named it.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`], [`ContentStoreError::Erased`]
    /// or [`ContentStoreError::DigestMismatch`] rather than serving content the
    /// reference does not vouch for.
    fn load(&self, reference: &ContentRef) -> Result<Vec<u8>, ContentStoreError>;

    /// Retrieve a window of content.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::RangeOutOfBounds`] when the window is not
    /// wholly inside the content, and the same absence errors as [`Self::load`].
    fn load_range(
        &self,
        reference: &ContentRef,
        range: Range<u64>,
    ) -> Result<Vec<u8>, ContentStoreError>;

    /// Describe content without transferring it.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the store never held it.
    fn describe(&self, reference: &ContentRef) -> Result<ContentStatus, ContentStoreError>;

    /// Erase content, recording why.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::NotRetained`] when the store never held the
    /// content, and [`ContentStoreError::Invalid`] when no reason is stated.
    fn erase(&mut self, reference: &ContentRef, reason: &str) -> Result<(), ContentStoreError>;
}

/// Content store over any [`ObjectStore`].
///
/// The content-addressed semantics live here once, so a backend only has to
/// move bytes.
#[derive(Clone, Debug)]
pub struct ObjectContentStore<S: ObjectStore> {
    backend: String,
    objects: S,
    tombstones: BTreeMap<String, ContentStatus>,
}

impl<S: ObjectStore> ObjectContentStore<S> {
    /// Build a content store over an object store.
    pub fn new(backend: impl Into<String>, objects: S) -> Self {
        Self {
            backend: backend.into(),
            objects,
            tombstones: BTreeMap::new(),
        }
    }

    /// Backend this store writes to.
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// The underlying object store.
    #[must_use]
    pub fn objects(&self) -> &S {
        &self.objects
    }

    /// Sharded, digest-derived key, so an identity cannot name other bytes.
    fn key_for(digest: &str) -> String {
        format!("{}/{}/{}", &digest[0..2], &digest[2..4], digest)
    }

    fn content_id(digest: &str) -> String {
        format!("content--{digest}")
    }

    fn check_backend(&self, reference: &ContentRef) -> Result<(), ContentStoreError> {
        if reference.backend() == self.backend {
            return Ok(());
        }
        Err(ContentStoreError::ForeignBackend {
            content_id: reference.content_id().to_owned(),
            reference_backend: reference.backend().to_owned(),
            store_backend: self.backend.clone(),
        })
    }

    fn absence(&self, reference: &ContentRef) -> ContentStoreError {
        match self.tombstones.get(reference.content_id()) {
            Some(ContentStatus::Erased { reason, .. }) => ContentStoreError::Erased {
                content_id: reference.content_id().to_owned(),
                reason: reason.clone(),
            },
            _ => ContentStoreError::NotRetained(reference.content_id().to_owned()),
        }
    }
}

impl ObjectContentStore<MemoryObjectStore> {
    /// Number of objects held by the backing store.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Substitute the bytes behind a reference, bypassing content addressing.
    #[doc(hidden)]
    pub fn corrupt_for_test(&mut self, reference: &ContentRef, bytes: &[u8]) {
        self.objects.substitute(reference.key(), bytes);
    }
}

impl<S: ObjectStore> ContentStore for ObjectContentStore<S> {
    fn store(
        &mut self,
        bytes: &[u8],
        media_type: Option<&str>,
    ) -> Result<ContentRef, ContentStoreError> {
        if bytes.is_empty() {
            return Err(ContentStoreError::Invalid(
                "content must not be empty".to_owned(),
            ));
        }
        let digest = hex_digest(bytes);
        let content_id = Self::content_id(&digest);
        let key = Self::key_for(&digest);
        // Identical bytes are already present under the same key, so there is
        // nothing to write and nothing that could differ.
        if self.objects.head(&key).is_err() {
            self.objects.put(&key, bytes)?;
        }
        // Erasure removes bytes; it is not a ban on the same bytes arriving
        // again from a legitimate source.
        self.tombstones.remove(&content_id);
        ContentRef::new(
            content_id,
            digest,
            bytes.len() as u64,
            self.backend.clone(),
            key,
        )
        .map(|reference| reference.with_media_type_opt(media_type))
        .map_err(|error| ContentStoreError::Invalid(error.to_string()))
    }

    fn load(&self, reference: &ContentRef) -> Result<Vec<u8>, ContentStoreError> {
        self.check_backend(reference)?;
        let bytes = self
            .objects
            .get(reference.key())
            .map_err(|_| self.absence(reference))?;
        if hex_digest(&bytes) != reference.sha256() {
            return Err(ContentStoreError::DigestMismatch {
                content_id: reference.content_id().to_owned(),
            });
        }
        Ok(bytes)
    }

    fn load_range(
        &self,
        reference: &ContentRef,
        range: Range<u64>,
    ) -> Result<Vec<u8>, ContentStoreError> {
        self.check_backend(reference)?;
        // Presence is checked first so an absent object is reported as absent
        // rather than as a range failure.
        let byte_length = self
            .objects
            .head(reference.key())
            .map_err(|_| self.absence(reference))?;
        if range.start > range.end || range.end > byte_length {
            return Err(ContentStoreError::RangeOutOfBounds {
                start: range.start,
                end: range.end,
                byte_length,
            });
        }
        self.objects.get_range(reference.key(), range)
    }

    fn describe(&self, reference: &ContentRef) -> Result<ContentStatus, ContentStoreError> {
        self.check_backend(reference)?;
        match self.objects.head(reference.key()) {
            Ok(byte_length) => Ok(ContentStatus::Present { byte_length }),
            Err(_) => match self.tombstones.get(reference.content_id()) {
                Some(status) => Ok(status.clone()),
                None => Err(ContentStoreError::NotRetained(
                    reference.content_id().to_owned(),
                )),
            },
        }
    }

    fn erase(&mut self, reference: &ContentRef, reason: &str) -> Result<(), ContentStoreError> {
        self.check_backend(reference)?;
        if reason.trim().is_empty() {
            return Err(ContentStoreError::Invalid(
                "an erasure must state a reason".to_owned(),
            ));
        }
        // Recording an erasure that never happened would falsify the audit trail.
        let byte_length = self
            .objects
            .head(reference.key())
            .map_err(|_| self.absence(reference))?;
        self.objects.remove(reference.key())?;
        self.tombstones.insert(
            reference.content_id().to_owned(),
            ContentStatus::Erased {
                byte_length,
                reason: reason.to_owned(),
            },
        );
        Ok(())
    }
}
