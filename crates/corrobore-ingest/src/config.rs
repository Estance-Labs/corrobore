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
use std::collections::HashMap;

use thiserror::Error;

const DEFAULT_WORKSPACE_ID: &str = "workspace--ingest-taxii";
const DEFAULT_POLL_INTERVAL_MS: u64 = 300_000;
const DEFAULT_PAGE_LIMIT: u32 = 100;
const DEFAULT_STATE_DIR: &str = ".corrobore-runtime/ingest";
/// Authentication used toward the TAXII server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaxiiAuth {
    /// Unauthenticated collection access.
    None,
    /// Bearer token.
    Bearer(String),
    /// HTTP Basic credentials.
    Basic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
}

/// Configuration loading errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IngestConfigError {
    /// A required environment variable is missing.
    #[error("missing required environment variable: {0}")]
    MissingEnv(&'static str),
    /// A variable is present but its value is invalid.
    #[error("invalid value for {name}: {reason}")]
    InvalidValue {
        /// Variable name.
        name: &'static str,
        /// Rejection reason.
        reason: String,
    },
}

/// Connector configuration, loadable from environment variables.
#[derive(Clone, Debug)]
pub struct IngestConfig {
    /// TAXII API root URL (without trailing collection path).
    pub taxii_root_url: String,
    /// TAXII collection identifier to poll.
    pub taxii_collection_id: String,
    /// Authentication toward the TAXII server.
    pub taxii_auth: TaxiiAuth,
    /// Corrobore HTTP server base URL.
    pub corrobore_base_url: String,
    /// Bearer token expected by the Corrobore HTTP server.
    pub corrobore_auth_token: String,
    /// Workspace identifier attached to imports.
    pub workspace_id: String,
    /// Poll interval in milliseconds for loop mode.
    pub poll_interval_ms: u64,
    /// Page size requested from the TAXII server.
    pub page_limit: u32,
    /// Directory holding the persisted cursor state.
    pub state_dir: String,
}

impl IngestConfig {
    /// Loads the configuration from process environment variables.
    pub fn from_env() -> Result<Self, IngestConfigError> {
        let vars: HashMap<String, String> = std::env::vars().collect();
        Self::from_map(&vars)
    }

    /// Loads the configuration from an explicit variable map.
    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, IngestConfigError> {
        let taxii_root_url = required(vars, "CORROBORE_INGEST_TAXII_ROOT_URL")?;
        let taxii_collection_id = required(vars, "CORROBORE_INGEST_TAXII_COLLECTION_ID")?;
        let corrobore_base_url = required(vars, "CORROBORE_INGEST_CORROBORE_BASE_URL")?;
        let corrobore_auth_token = required(vars, "CORROBORE_INGEST_CORROBORE_AUTH_TOKEN")?;
        let taxii_auth = parse_taxii_auth(vars)?;

        let workspace_id = optional(vars, "CORROBORE_INGEST_WORKSPACE_ID")
            .unwrap_or_else(|| DEFAULT_WORKSPACE_ID.to_owned());

        let poll_interval_ms = parse_number(
            vars,
            "CORROBORE_INGEST_POLL_INTERVAL_MS",
            DEFAULT_POLL_INTERVAL_MS,
        )?;

        let page_limit: u32 =
            parse_number(vars, "CORROBORE_INGEST_PAGE_LIMIT", DEFAULT_PAGE_LIMIT)?;
        if page_limit == 0 {
            return Err(IngestConfigError::InvalidValue {
                name: "CORROBORE_INGEST_PAGE_LIMIT",
                reason: "page limit must be greater than zero".to_owned(),
            });
        }

        let state_dir = optional(vars, "CORROBORE_INGEST_STATE_DIR")
            .unwrap_or_else(|| DEFAULT_STATE_DIR.to_owned());

        Ok(Self {
            taxii_root_url,
            taxii_collection_id,
            taxii_auth,
            corrobore_base_url,
            corrobore_auth_token,
            workspace_id,
            poll_interval_ms,
            page_limit,
            state_dir,
        })
    }
}

fn required(
    vars: &HashMap<String, String>,
    name: &'static str,
) -> Result<String, IngestConfigError> {
    let value = vars
        .get(name)
        .cloned()
        .ok_or(IngestConfigError::MissingEnv(name))?;

    if value.trim().is_empty() {
        return Err(IngestConfigError::InvalidValue {
            name,
            reason: "value cannot be blank".to_owned(),
        });
    }

    Ok(value)
}

fn optional(vars: &HashMap<String, String>, name: &str) -> Option<String> {
    vars.get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_number<T: std::str::FromStr>(
    vars: &HashMap<String, String>,
    name: &'static str,
    default: T,
) -> Result<T, IngestConfigError> {
    match optional(vars, name) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<T>()
            .map_err(|_| IngestConfigError::InvalidValue {
                name,
                reason: format!("expected a positive integer, got '{raw}'"),
            }),
    }
}

fn parse_taxii_auth(vars: &HashMap<String, String>) -> Result<TaxiiAuth, IngestConfigError> {
    let token = optional(vars, "CORROBORE_INGEST_TAXII_TOKEN");
    let username = optional(vars, "CORROBORE_INGEST_TAXII_USERNAME");
    let password = optional(vars, "CORROBORE_INGEST_TAXII_PASSWORD");

    if token.is_some() && (username.is_some() || password.is_some()) {
        return Err(IngestConfigError::InvalidValue {
            name: "CORROBORE_INGEST_TAXII_TOKEN",
            reason: "bearer token conflicts with basic credentials; configure only one".to_owned(),
        });
    }

    if let Some(token) = token {
        return Ok(TaxiiAuth::Bearer(token));
    }

    match (username, password) {
        (Some(username), Some(password)) => Ok(TaxiiAuth::Basic { username, password }),
        (None, None) => Ok(TaxiiAuth::None),
        _ => Err(IngestConfigError::InvalidValue {
            name: "CORROBORE_INGEST_TAXII_USERNAME",
            reason: "username and password must be provided together".to_owned(),
        }),
    }
}
