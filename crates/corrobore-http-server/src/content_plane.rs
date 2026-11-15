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

//! The content plane served by the HTTP server (ADR-0021, issue #341).
//!
//! Uploads go through [`ingest_content`]: content the storage policy keeps
//! inline comes back in the handle and is never stored; anything else is
//! offloaded, content-addressed, to one object store chosen at startup:
//!
//! - `CORROBORE_HTTP_CONTENT_STORE_URL` (`s3://`, `file://`, `memory://`);
//! - otherwise `<storage>/content` in persistent mode;
//! - otherwise memory, in ephemeral mode.
//!
//! The object store only keeps bytes, so each offloaded content also gets a
//! small reference object, `refs/<content_id>.json`, holding the
//! [`ContentRef`] (length, backend, key, media type). Reads by content id
//! resolve that reference and let the content store verify the digest.

use std::{
    ops::Range,
    path::Path,
    sync::{Arc, Mutex},
};

use graph_core::{
    ContentPlacementDecision, ContentRef, ContentStoragePolicy, ContentStore, ContentStoreError,
    MemoryObjectStore, ObjectContentStore, ObjectStore, ingest_content,
};
use graph_storage::FilesystemObjectStore;
use storage_interchange::UrlObjectStore;

use crate::config::ContentStoreConfig;

/// Version of the storage policy recorded with every placement decision.
pub const CONTENT_POLICY_VERSION: &str = "content-policy-v1";

/// One object store shared by the content store and the reference objects.
#[derive(Clone)]
struct SharedObjects(Arc<Mutex<Box<dyn ObjectStore + Send>>>);

impl std::fmt::Debug for SharedObjects {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedObjects")
    }
}

impl SharedObjects {
    fn with<T>(
        &self,
        operation: impl FnOnce(&mut Box<dyn ObjectStore + Send>) -> Result<T, ContentStoreError>,
    ) -> Result<T, ContentStoreError> {
        let mut objects = self
            .0
            .lock()
            .map_err(|_| ContentStoreError::Backend("object store lock poisoned".to_owned()))?;
        operation(&mut objects)
    }
}

impl ObjectStore for SharedObjects {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        self.with(|objects| objects.put(key, bytes))
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        self.with(|objects| objects.get(key))
    }

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ContentStoreError> {
        self.with(|objects| objects.get_range(key, range))
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        self.with(|objects| objects.head(key))
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        self.with(|objects| objects.remove(key))
    }
}

/// Why a content id could not be served.
#[derive(Debug, thiserror::Error)]
pub enum ContentReadError {
    /// The id is not `content--` followed by a lowercase SHA-256.
    #[error("content id must be `content--` followed by 64 lowercase hexadecimal characters")]
    InvalidId,
    /// The content store failed or refused.
    #[error(transparent)]
    Store(#[from] ContentStoreError),
}

/// The server's content store, its storage policy and its reference objects.
#[derive(Debug)]
pub struct ContentPlane {
    store: ObjectContentStore<SharedObjects>,
    refs: SharedObjects,
    policy: ContentStoragePolicy,
}

impl ContentPlane {
    /// Open the content store the configuration selects.
    ///
    /// # Errors
    ///
    /// Returns a description of the failure when the URL cannot be opened, the
    /// directory cannot be created or the policy is invalid.
    pub fn open(config: &ContentStoreConfig, storage_root: Option<&Path>) -> Result<Self, String> {
        let (backend, objects): (String, Box<dyn ObjectStore + Send>) =
            match (&config.url, storage_root) {
                (Some(url), _) => {
                    let scheme = url.split_once("://").map_or("", |(scheme, _)| scheme);
                    let store = UrlObjectStore::open(url, config.object_store_options())
                        .map_err(|error| error.to_string())?;
                    (scheme.to_owned(), Box::new(store))
                }
                (None, Some(root)) => {
                    let store = FilesystemObjectStore::open(root.join("content"))
                        .map_err(|error| error.to_string())?;
                    ("filesystem".to_owned(), Box::new(store))
                }
                (None, None) => ("memory".to_owned(), Box::new(MemoryObjectStore::default())),
            };
        let policy = ContentStoragePolicy::new(CONTENT_POLICY_VERSION, config.inline_max_bytes)
            .map_err(|error| error.to_string())?;
        let objects = SharedObjects(Arc::new(Mutex::new(objects)));
        Ok(Self {
            store: ObjectContentStore::new(backend, objects.clone()),
            refs: objects,
            policy,
        })
    }

    /// The backend name recorded in references to offloaded content.
    #[must_use]
    pub fn backend(&self) -> &str {
        self.store.backend()
    }

    /// Place content by policy: inline content is returned, other content is
    /// retained and its reference recorded.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::Invalid`] for empty content or text that
    /// is not UTF-8, and a backend error when the store cannot write.
    pub fn ingest(
        &mut self,
        bytes: &[u8],
        media_type: Option<&str>,
    ) -> Result<ContentPlacementDecision, ContentStoreError> {
        let decision = ingest_content(bytes, media_type, &self.policy, &mut self.store)?;
        if let graph_core::ContentHandle::External(reference) = decision.handle() {
            let key = reference_key(reference.content_id());
            // The first reference wins: identical bytes uploaded again under
            // another media type keep the type they were first stored with.
            if self.refs.head(&key).is_err() {
                let encoded = serde_json::to_vec(reference)
                    .map_err(|error| ContentStoreError::Backend(error.to_string()))?;
                self.refs.put(&key, &encoded)?;
            }
        }
        Ok(decision)
    }

    /// The reference recorded for an offloaded content id.
    ///
    /// # Errors
    ///
    /// Returns [`ContentReadError::InvalidId`] for a malformed id and
    /// [`ContentStoreError::NotRetained`] when nothing was offloaded under it.
    pub fn reference(&self, content_id: &str) -> Result<ContentRef, ContentReadError> {
        let digest = content_id
            .strip_prefix("content--")
            .filter(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            .ok_or(ContentReadError::InvalidId)?;
        let encoded = self
            .refs
            .get(&reference_key(content_id))
            .map_err(|_| ContentStoreError::NotRetained(content_id.to_owned()))?;
        let reference: ContentRef = serde_json::from_slice(&encoded).map_err(|error| {
            ContentStoreError::Backend(format!("unreadable reference: {error}"))
        })?;
        if reference.sha256() != digest {
            return Err(ContentStoreError::DigestMismatch {
                content_id: content_id.to_owned(),
            }
            .into());
        }
        Ok(reference)
    }

    /// Read offloaded content, whole or one window of it.
    ///
    /// # Errors
    ///
    /// Propagates absence, erasure, digest and range errors of the store.
    pub fn read(
        &self,
        reference: &ContentRef,
        range: Option<Range<u64>>,
    ) -> Result<Vec<u8>, ContentStoreError> {
        match range {
            Some(range) => self.store.load_range(reference, range),
            None => self.store.load(reference),
        }
    }
}

fn reference_key(content_id: &str) -> String {
    format!("refs/{content_id}.json")
}
