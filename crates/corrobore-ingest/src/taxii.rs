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
use reqwest::header::ACCEPT;
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::{IngestError, TaxiiAuth};

const TAXII_MEDIA_TYPE: &str = "application/taxii+json;version=2.1";
const DATE_ADDED_LAST_HEADER: &str = "X-TAXII-Date-Added-Last";
// Upper bound on envelope pages per cycle so a misbehaving server that always
// answers `more: true` cannot pin the connector in an endless drain loop.
const MAX_PAGES_PER_CYCLE: usize = 1_000;

#[derive(Debug, Deserialize)]
struct TaxiiEnvelope {
    #[serde(default)]
    objects: Vec<Value>,
    #[serde(default)]
    more: bool,
    #[serde(default)]
    next: Option<String>,
}
/// Result of draining every available TAXII envelope page.
#[derive(Clone, Debug)]
pub struct TaxiiFetch {
    /// STIX objects across all pages, in server order.
    pub objects: Vec<Value>,
    /// Last `X-TAXII-Date-Added-Last` header observed, used as next cursor.
    pub date_added_last: Option<String>,
}

/// Minimal TAXII 2.1 collection poll client.
#[derive(Debug)]
pub struct TaxiiClient {
    root_url: String,
    collection_id: String,
    auth: TaxiiAuth,
    page_limit: u32,
    http: reqwest::Client,
}

impl TaxiiClient {
    /// Creates a client for one collection.
    pub fn new(
        root_url: impl Into<String>,
        collection_id: impl Into<String>,
        auth: TaxiiAuth,
        page_limit: u32,
    ) -> Self {
        Self {
            root_url: root_url.into(),
            collection_id: collection_id.into(),
            auth,
            page_limit,
            http: reqwest::Client::new(),
        }
    }

    /// Fetches every object added after `added_after`, following pagination.
    pub async fn fetch_new_objects(
        &self,
        added_after: Option<&str>,
    ) -> Result<TaxiiFetch, IngestError> {
        let url = format!(
            "{}/collections/{}/objects",
            self.root_url.trim_end_matches('/'),
            self.collection_id
        );

        let mut objects = Vec::new();
        let mut date_added_last: Option<String> = None;
        let mut next: Option<String> = None;

        for _page in 0..MAX_PAGES_PER_CYCLE {
            let mut request = self
                .http
                .get(&url)
                .header(ACCEPT, TAXII_MEDIA_TYPE)
                .query(&[("limit", self.page_limit.to_string())]);

            if let Some(cursor) = added_after {
                request = request.query(&[("added_after", cursor)]);
            }
            if let Some(next_value) = &next {
                request = request.query(&[("next", next_value.as_str())]);
            }

            request = match &self.auth {
                TaxiiAuth::None => request,
                TaxiiAuth::Bearer(token) => request.bearer_auth(token),
                TaxiiAuth::Basic { username, password } => {
                    request.basic_auth(username, Some(password))
                }
            };

            let response = request
                .send()
                .await
                .map_err(|error| IngestError::Transport(error.to_string()))?;

            let status = response.status();
            if !status.is_success() {
                let message = response.text().await.unwrap_or_default();
                return Err(IngestError::Taxii {
                    status: status.as_u16(),
                    message,
                });
            }

            if let Some(header_value) = response
                .headers()
                .get(DATE_ADDED_LAST_HEADER)
                .and_then(|value| value.to_str().ok())
            {
                date_added_last = Some(header_value.to_owned());
            }

            let envelope: TaxiiEnvelope = response
                .json()
                .await
                .map_err(|error| IngestError::Transport(error.to_string()))?;

            objects.extend(envelope.objects);

            if !envelope.more {
                return Ok(TaxiiFetch {
                    objects,
                    date_added_last,
                });
            }

            match envelope.next {
                Some(next_value) => next = Some(next_value),
                None => {
                    warn!("taxii envelope claimed more pages without a next cursor; stopping");
                    return Ok(TaxiiFetch {
                        objects,
                        date_added_last,
                    });
                }
            }
        }

        warn!(
            max_pages = MAX_PAGES_PER_CYCLE,
            "taxii page cap reached; remaining objects will be fetched next cycle"
        );
        Ok(TaxiiFetch {
            objects,
            date_added_last,
        })
    }
}
