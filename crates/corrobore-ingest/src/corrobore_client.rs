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
use std::time::Duration;

use opencti_adapter::{
    GraphDigest, OpenCtiSyncBatch, SyncBatchResult, SyncCheckpoint, SyncValidationReport,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::IngestError;

// `reqwest` applies no timeout of its own. An import posts a whole bundle, so the
// bound is larger than a plain read but still finite: without it a stalled server
// would hold the connector open indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Builds the shared HTTP client with explicit transport bounds.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        // Only the timeouts are configured here, so construction fails solely
        // when the TLS backend cannot initialize, which is not recoverable.
        .expect("Corrobore import HTTP client with timeouts must build")
}

/// Import outcome reported by the Corrobore HTTP server.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ImportSummary {
    /// Objects processed by the import pipeline.
    pub processed_objects: usize,
    /// Mutations applied to the graph.
    pub applied_mutations: usize,
    /// Mutations rejected by validation.
    pub rejected_mutations: usize,
    /// First rejection message per rejected mutation.
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ImportResponse {
    result: ImportSummary,
}

/// Durable response returned by the OpenCTI synchronization endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct OpenCtiSyncSummary {
    /// Per-operation results for the submitted batch.
    pub batch: SyncBatchResult,
    /// Acknowledged durable source checkpoint.
    pub checkpoint: SyncCheckpoint,
    /// Optional divergence report used to gate shadow reads.
    pub validation: Option<SyncValidationReport>,
}

#[derive(Debug, Deserialize)]
struct OpenCtiSyncResponse {
    result: OpenCtiSyncSummary,
}
/// Client for the Corrobore STIX import endpoint.
#[derive(Debug)]
pub struct CorroboreImportClient {
    base_url: String,
    auth_token: String,
    workspace_id: String,
    http: reqwest::Client,
}

impl CorroboreImportClient {
    /// Creates a client for one Corrobore server.
    pub fn new(
        base_url: impl Into<String>,
        auth_token: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            auth_token: auth_token.into(),
            workspace_id: workspace_id.into(),
            http: build_http_client(),
        }
    }

    /// Wraps `objects` into a STIX bundle and imports it.
    pub async fn import_objects(&self, objects: Vec<Value>) -> Result<ImportSummary, IngestError> {
        let body = json!({
            "bundle": {
                "type": "bundle",
                "objects": objects,
            },
            "workspace_id": self.workspace_id,
        });

        let url = format!("{}/v1/import/stix", self.base_url.trim_end_matches('/'));

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.auth_token)
            .json(&body)
            .send()
            .await
            .map_err(|error| IngestError::Transport(error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(IngestError::CorroboreImport {
                status: status.as_u16(),
                message,
            });
        }

        let parsed: ImportResponse = response
            .json()
            .await
            .map_err(|error| IngestError::Transport(error.to_string()))?;

        Ok(parsed.result)
    }

    /// Submit one bounded snapshot/catch-up batch to the canonical WAL-backed
    /// synchronization endpoint.
    pub async fn synchronize_opencti(
        &self,
        batch: OpenCtiSyncBatch,
        expected: Option<GraphDigest>,
    ) -> Result<OpenCtiSyncSummary, IngestError> {
        let url = format!(
            "{}/v1/opencti/sync/batches",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.auth_token)
            .json(&json!({"batch": batch, "expected": expected}))
            .send()
            .await
            .map_err(|error| IngestError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(IngestError::CorroboreImport {
                status: status.as_u16(),
                message,
            });
        }
        response
            .json::<OpenCtiSyncResponse>()
            .await
            .map(|response| response.result)
            .map_err(|error| IngestError::Transport(error.to_string()))
    }
}
