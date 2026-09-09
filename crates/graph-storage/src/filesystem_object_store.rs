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
//! Durable object store backing the content plane (ADR-0021).
//!
//! `graph-core` states the storage-neutral contract and implements none of it.
//! Because the content-addressed semantics live once in `ObjectContentStore`,
//! this backend only has to move bytes — and never serve a partial write as
//! content.
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use graph_core::{ContentStoreError, ObjectStore};

/// Suffix of a write in progress, which is never a readable object.
const PENDING_SUFFIX: &str = ".next";

fn invalid(message: impl Into<String>) -> ContentStoreError {
    ContentStoreError::Invalid(message.into())
}

fn backend(operation: &str, error: &std::io::Error) -> ContentStoreError {
    ContentStoreError::Backend(format!("{operation}: {error}"))
}

/// Object store over a directory tree.
#[derive(Clone, Debug)]
pub struct FilesystemObjectStore {
    root: PathBuf,
}

impl FilesystemObjectStore {
    /// Open a store rooted at `root`, creating the directory if needed.
    ///
    /// # Errors
    ///
    /// Returns [`ContentStoreError::Backend`] when the root cannot be created.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ContentStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|error| backend("create_object_root", &error))?;
        Ok(Self { root })
    }

    /// Root this store writes under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a key inside the root.
    ///
    /// Keys are opaque, so anything that could leave the root is a malformed
    /// key rather than a path to follow. A pending write is not addressable,
    /// so a crashed write cannot be read back as content.
    fn resolve(&self, key: &str) -> Result<PathBuf, ContentStoreError> {
        if key.is_empty() || key.ends_with(PENDING_SUFFIX) {
            return Err(invalid(format!("object key is not addressable: {key}")));
        }
        let mut path = self.root.clone();
        for segment in key.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(invalid(format!("object key must not traverse: {key}")));
            }
            path.push(segment);
        }
        Ok(path)
    }

    fn length(&self, path: &Path, key: &str) -> Result<u64, ContentStoreError> {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
            Ok(_) => Err(invalid(format!("object key is not a file: {key}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(ContentStoreError::NotRetained(key.to_owned()))
            }
            Err(error) => Err(backend("stat_object", &error)),
        }
    }
}

impl ObjectStore for FilesystemObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), ContentStoreError> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| backend("create_object_parent", &error))?;
        }
        // Write aside and rename, so an interrupted write leaves nothing that
        // would later be served as content.
        let pending = path.with_extension(match path.extension() {
            Some(extension) => format!("{}{PENDING_SUFFIX}", extension.to_string_lossy()),
            None => PENDING_SUFFIX.trim_start_matches('.').to_owned(),
        });
        let mut file =
            File::create(&pending).map_err(|error| backend("create_pending_object", &error))?;
        file.write_all(bytes)
            .map_err(|error| backend("write_pending_object", &error))?;
        file.sync_all()
            .map_err(|error| backend("sync_pending_object", &error))?;
        drop(file);
        fs::rename(&pending, &path).map_err(|error| backend("publish_object", &error))
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ContentStoreError> {
        let path = self.resolve(key)?;
        self.length(&path, key)?;
        fs::read(&path).map_err(|error| backend("read_object", &error))
    }

    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ContentStoreError> {
        let path = self.resolve(key)?;
        let byte_length = self.length(&path, key)?;
        if range.start > range.end || range.end > byte_length {
            return Err(ContentStoreError::RangeOutOfBounds {
                start: range.start,
                end: range.end,
                byte_length,
            });
        }
        // Seek to the window rather than reading the object and slicing it,
        // which is the reason a ranged read exists.
        let mut file = File::open(&path).map_err(|error| backend("open_object", &error))?;
        file.seek(SeekFrom::Start(range.start))
            .map_err(|error| backend("seek_object", &error))?;
        let mut window = vec![0u8; (range.end - range.start) as usize];
        file.read_exact(&mut window)
            .map_err(|error| backend("read_object_range", &error))?;
        Ok(window)
    }

    fn head(&self, key: &str) -> Result<u64, ContentStoreError> {
        let path = self.resolve(key)?;
        self.length(&path, key)
    }

    fn remove(&mut self, key: &str) -> Result<(), ContentStoreError> {
        let path = self.resolve(key)?;
        self.length(&path, key)?;
        fs::remove_file(&path).map_err(|error| backend("remove_object", &error))
    }
}
