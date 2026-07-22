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
use tracing::info;

use crate::{
    CorroboreImportClient, CursorStore, ImportSummary, IngestConfig, IngestError, TaxiiClient,
};

/// Structured outcome of one poll cycle.
#[derive(Clone, Debug)]
pub struct PollOutcome {
    /// Objects fetched from the TAXII collection across all pages.
    pub fetched_objects: usize,
    /// Import summary when at least one object was fetched.
    pub import: Option<ImportSummary>,
    /// Cursor persisted at the end of the cycle, if the server provided one.
    pub cursor: Option<String>,
}

/// Runs one poll cycle: fetch new objects, import them, advance the cursor.
///
/// The cursor is persisted only after a successful import, so a failed cycle
/// is retried from the previous cursor (at-least-once delivery into the
/// import pipeline, which is idempotent through MERGE semantics).
pub async fn run_poll_cycle(
    config: &IngestConfig,
    store: &mut CursorStore,
) -> Result<PollOutcome, IngestError> {
    let previous_cursor = store.load_cursor(&config.taxii_collection_id)?;

    let taxii = TaxiiClient::new(
        &config.taxii_root_url,
        &config.taxii_collection_id,
        config.taxii_auth.clone(),
        config.page_limit,
    );

    let fetch = taxii.fetch_new_objects(previous_cursor.as_deref()).await?;
    let fetched_objects = fetch.objects.len();

    let import = if fetched_objects > 0 {
        let importer = CorroboreImportClient::new(
            &config.corrobore_base_url,
            &config.corrobore_auth_token,
            &config.workspace_id,
        );
        Some(importer.import_objects(fetch.objects).await?)
    } else {
        None
    };

    if let Some(cursor) = &fetch.date_added_last {
        store.save_cursor(&config.taxii_collection_id, cursor)?;
    }

    info!(
        collection_id = %config.taxii_collection_id,
        fetched_objects,
        applied = import.as_ref().map(|summary| summary.applied_mutations),
        rejected = import.as_ref().map(|summary| summary.rejected_mutations),
        cursor = ?fetch.date_added_last,
        "poll cycle completed"
    );

    Ok(PollOutcome {
        fetched_objects,
        import,
        cursor: fetch.date_added_last,
    })
}
