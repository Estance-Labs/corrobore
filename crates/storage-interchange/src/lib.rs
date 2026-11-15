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

//! Open interchange for Corrobore graphs (issue #327): object-storage
//! backends through the `object_store` crate, and Apache Parquet export of a
//! commit.
#![warn(missing_docs)]

mod object_storage;
mod parquet_export;

pub use object_storage::UrlObjectStore;
pub use parquet_export::{ParquetExportReport, ParquetFile, export_commit_parquet};

/// Interchange failures.
#[derive(Debug, thiserror::Error)]
pub enum InterchangeError {
    /// The store URL is not usable.
    #[error("object store URL {url} is not usable: {reason}")]
    InvalidUrl {
        /// URL given.
        url: String,
        /// Parser or builder diagnostic.
        reason: String,
    },
    /// The object store backend failed.
    #[error("object store failure: {0}")]
    Backend(String),
    /// Reading the graph failed.
    #[error("graph read failed: {0}")]
    Graph(#[from] graph_storage::GraphStorageError),
    /// Writing the columnar export failed.
    #[error("columnar export failed: {0}")]
    Export(String),
}
