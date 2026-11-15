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

//! Object storage through the `object_store` crate: local files, S3 and
//! S3-compatible stores (MinIO, R2), or memory, behind Corrobore's content
//! plane and snapshot artifact interfaces.
//!
//! The interfaces are synchronous; each store owns a single-threaded tokio
//! runtime and blocks on it. Call it from a blocking context (a CLI, or
//! `tokio::task::spawn_blocking` in the server), never from an async task.

use std::ops::Range;
use std::sync::Arc;

use futures::TryStreamExt;
use graph_core::{ContentStoreError, ObjectStore as ContentObjectStore};
use graph_storage::SnapshotArtifactStore;
use object_store::{ObjectStore, ObjectStoreExt, path::Path};

use crate::InterchangeError;

/// An object store rooted at a URL prefix.
#[derive(Debug)]
pub struct UrlObjectStore {
    store: Arc<dyn ObjectStore>,
    prefix: Path,
    runtime: Option<tokio::runtime::Runtime>,
    description: String,
}

impl Drop for UrlObjectStore {
    /// A runtime dropped inside an async context panics, and hosts such as the
    /// HTTP server drop their stores there; shutting it down in the background
    /// is always allowed.
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl UrlObjectStore {
    /// Open `file:///…`, `s3://bucket/prefix` or `memory://` with
    /// provider options (for S3: `aws_endpoint`, `aws_region`,
    /// `aws_access_key_id`, `aws_secret_access_key`, `aws_session_token`,
    /// `aws_allow_http`).
    pub fn open<I, K, V>(url: &str, options: I) -> Result<Self, InterchangeError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let parsed = url::Url::parse(url).map_err(|error| InterchangeError::InvalidUrl {
            url: url.to_owned(),
            reason: error.to_string(),
        })?;
        let (store, prefix) = object_store::parse_url_opts(&parsed, options).map_err(|error| {
            InterchangeError::InvalidUrl {
                url: url.to_owned(),
                reason: error.to_string(),
            }
        })?;
        Self::from_store(Arc::from(store), prefix, url)
    }

    /// Wrap an existing store, for tests and embedding hosts.
    pub fn from_store(
        store: Arc<dyn ObjectStore>,
        prefix: Path,
        description: &str,
    ) -> Result<Self, InterchangeError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| InterchangeError::Backend(error.to_string()))?;
        Ok(Self {
            store,
            prefix,
            runtime: Some(runtime),
            description: description.to_owned(),
        })
    }

    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime
            .as_ref()
            .expect("the runtime lives until the store is dropped")
            .block_on(future)
    }

    /// Where the store points.
    pub fn description(&self) -> &str {
        &self.description
    }

    fn path(&self, key: &str) -> Path {
        key.split('/')
            .filter(|part| !part.is_empty())
            .fold(self.prefix.clone(), |path, part| path.clone().join(part))
    }

    fn relative(&self, location: &Path) -> String {
        location
            .prefix_match(&self.prefix)
            .map(|parts| {
                parts
                    .map(|part| part.as_ref().to_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_else(|| location.to_string())
    }

    fn put_bytes(&self, key: &str, bytes: &[u8]) -> Result<(), object_store::Error> {
        let path = self.path(key);
        self.block_on(self.store.put(&path, bytes.to_vec().into()))
            .map(|_| ())
    }

    fn get_bytes(&self, key: &str) -> Result<Vec<u8>, object_store::Error> {
        let path = self.path(key);
        self.block_on(async {
            let result = self.store.get(&path).await?;
            Ok(result.bytes().await?.to_vec())
        })
    }

    /// Keys starting with `prefix` (a plain string prefix, as S3 lists),
    /// relative to the store root, sorted.
    pub fn list_keys(&self, prefix: &str) -> Result<Vec<String>, InterchangeError> {
        let prefix = prefix.trim_start_matches('/');
        let directory = prefix
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory);
        let path = self.path(directory);
        let listed: Vec<object_store::ObjectMeta> = self
            .block_on(self.store.list(Some(&path)).try_collect())
            .map_err(backend)?;
        let mut keys: Vec<String> = listed
            .iter()
            .map(|meta| self.relative(&meta.location))
            .filter(|key| key.starts_with(prefix))
            .collect();
        keys.sort();
        Ok(keys)
    }
}

fn backend(error: object_store::Error) -> InterchangeError {
    InterchangeError::Backend(error.to_string())
}

fn content_error(key: &str, error: object_store::Error) -> ContentStoreError {
    match error {
        object_store::Error::NotFound { .. } => ContentStoreError::NotRetained(key.to_owned()),
        other => ContentStoreError::Backend(other.to_string()),
    }
}

impl ContentObjectStore for UrlObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        self.put_bytes(key, bytes)
            .map_err(|error| content_error(key, error))
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        self.get_bytes(key)
            .map_err(|error| content_error(key, error))
    }

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ContentStoreError> {
        let length = self.head(key)?;
        if range.start > range.end || range.end > length {
            return Err(ContentStoreError::RangeOutOfBounds {
                start: range.start,
                end: range.end,
                byte_length: length,
            });
        }
        let path = self.path(key);
        self.block_on(self.store.get_range(&path, range))
            .map(|bytes| bytes.to_vec())
            .map_err(|error| content_error(key, error))
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        let path = self.path(key);
        self.block_on(self.store.head(&path))
            .map(|meta| meta.size)
            .map_err(|error| content_error(key, error))
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        self.head(key)?;
        let path = self.path(key);
        self.block_on(self.store.delete(&path))
            .map_err(|error| content_error(key, error))
    }
}

impl SnapshotArtifactStore for UrlObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.put_bytes(key, bytes)
            .map_err(|error| error.to_string())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        self.get_bytes(key).map_err(|error| error.to_string())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        self.list_keys(prefix).map_err(|error| error.to_string())
    }
}
