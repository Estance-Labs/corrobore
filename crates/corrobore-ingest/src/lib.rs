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
#![warn(missing_docs)]

//! TAXII 2.1 ingestion connector for the intelligence graph engine.
//!
//! This crate keeps a Corrobore graph alive without an agent pushing every fact:
//! it polls a TAXII 2.1 collection incrementally and routes fetched STIX
//! objects through the existing `POST /v1/import/stix` pipeline, where the
//! engine's mapping, validation, and audit rules apply.
//!
//! Design boundary:
//!
//! - the connector lives outside the core engine and talks to Corrobore only
//!   through the public HTTP API, so the connector matrix can grow without
//!   touching engine crates;
//! - incremental state (the TAXII `added_after` cursor) is persisted per
//!   collection so restarts never re-import or skip windows;
//! - one poll cycle is a pure, testable unit: fetch pages, import once,
//!   advance cursor, report a structured outcome.

mod config;
mod corrobore_client;
mod runner;
mod state;
mod taxii;

pub use config::{IngestConfig, IngestConfigError, TaxiiAuth};
pub use corrobore_client::{CorroboreImportClient, ImportSummary};
pub use runner::{PollOutcome, run_poll_cycle};
pub use state::CursorStore;
pub use taxii::{TaxiiClient, TaxiiFetch};

use thiserror::Error;

/// Error surface for ingestion operations.
#[derive(Debug, Error)]
pub enum IngestError {
    /// Configuration loading failed.
    #[error("configuration error: {0}")]
    Config(#[from] IngestConfigError),
    /// HTTP transport failed before a response was received.
    #[error("http transport error: {0}")]
    Transport(String),
    /// The TAXII server answered with a non-success status.
    #[error("taxii server error: status {status}: {message}")]
    Taxii {
        /// HTTP status code returned by the TAXII server.
        status: u16,
        /// Response body or reason.
        message: String,
    },
    /// The Corrobore import endpoint answered with a non-success status.
    #[error("corrobore import error: status {status}: {message}")]
    CorroboreImport {
        /// HTTP status code returned by Corrobore.
        status: u16,
        /// Response body or reason.
        message: String,
    },
    /// Cursor state persistence failed.
    #[error("state persistence error: {0}")]
    State(String),
}
