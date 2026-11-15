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

//! HTTP side of deployment bundles and stored queries (issue #388).

use std::collections::HashMap;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::AppState,
    auth::RequestIdentity,
    cluster::{Bundle, BundleFiles, ClusterError, DeploymentRecord, PlannedChange, plan},
    error::ApiError,
    handlers::{
        cypher::{ExecuteCypherRequest, ExecuteCypherResponse},
        graph_history::{Envelope, ok},
    },
    policy::{PolicyAction, PolicyRequest},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentBody {
    #[serde(default)]
    pub deployment_id: Option<String>,
    pub files: BundleFiles,
}

#[derive(Debug, Serialize)]
pub struct DeploymentResult {
    /// False when this id was already applied with the same input.
    pub applied: bool,
    pub deployment: DeploymentRecord,
    pub changes: Vec<PlannedChange>,
}

fn cluster_error(error: ClusterError) -> ApiError {
    match error {
        ClusterError::Invalid(message) => {
            ApiError::unprocessable("CLUSTER_BUNDLE_INVALID", message)
        }
        ClusterError::DeploymentConflict(id) => ApiError {
            status: StatusCode::CONFLICT,
            code: "DEPLOYMENT_CONFLICT",
            message: format!("deployment {id} was already applied with different input"),
        },
        ClusterError::State { .. } => ApiError::internal("CLUSTER_STATE_FAILED", error.to_string()),
    }
}

/// `POST /v1/cluster/deployments`: validate, apply and activate a bundle.
/// The policy in force (checked by the route table) authorizes it; the
/// bundle's own policy only governs later requests.
pub async fn deploy(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Json(body): Json<DeploymentBody>,
) -> Result<Json<Envelope<DeploymentResult>>, ApiError> {
    let bundle = Bundle::from_files(&body.files).map_err(cluster_error)?;
    if let Some(root) = state.storage_root() {
        crate::cluster::check_storage_assertion(&bundle, &root).map_err(cluster_error)?;
    }
    let deployment_id = body
        .deployment_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
    let actor = identity.and_then(|Extension(identity)| identity.actor);
    let blocking = state.clone();
    let revision = bundle.revision.clone();
    tokio::task::spawn_blocking(move || {
        // One deployment at a time: the ledger and the active revision move together.
        let _deploying = blocking
            .deploy_lock
            .lock()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "deployment lock poisoned"))?;
        let previous = blocking.current_revision();
        let (record, applied) = blocking
            .cluster_state
            .apply(&deployment_id, &revision, actor)
            .map_err(cluster_error)?;
        let changes = if applied {
            plan(&previous, &revision)
        } else {
            Vec::new()
        };
        if applied {
            blocking
                .activate_revision(revision)
                .map_err(|reason| ApiError::internal("CLUSTER_ACTIVATION_FAILED", reason))?;
        }
        Ok(ok(DeploymentResult {
            applied,
            deployment: record,
            changes,
        }))
    })
    .await
    .map_err(|_| ApiError::internal("DEPLOYMENT_FAILED", "deployment task failed"))?
}

#[derive(Debug, Serialize)]
pub struct AppliedView {
    pub name: Option<String>,
    pub digest: String,
    pub policy: bool,
    pub queries: usize,
}

#[derive(Debug, Serialize)]
pub struct ClusterStatus {
    pub applied: AppliedView,
    /// Whether `CORROBORE_HTTP_POLICY_FILE` overrides the applied policy.
    pub policy_file_override: bool,
    pub deployments: Vec<DeploymentRecord>,
}

/// `GET /v1/cluster/status`.
pub async fn status(
    State(state): State<AppState>,
) -> Result<Json<Envelope<ClusterStatus>>, ApiError> {
    let revision = state.current_revision();
    let deployments = state.cluster_state.deployments().map_err(cluster_error)?;
    Ok(ok(ClusterStatus {
        applied: AppliedView {
            name: revision.name.clone(),
            digest: revision.digest.clone(),
            policy: revision.policy.is_some(),
            queries: revision.queries.len(),
        },
        policy_file_override: state.config.policy_file.is_some(),
        deployments,
    }))
}

#[derive(Debug, Serialize)]
pub struct StoredQueryView {
    pub name: String,
    pub description: Option<String>,
    pub writes: bool,
    pub parameters: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StoredQueryList {
    pub queries: Vec<StoredQueryView>,
}

/// `GET /v1/queries`: the applied stored queries.
pub async fn list_queries(State(state): State<AppState>) -> Json<Envelope<StoredQueryList>> {
    let revision = state.current_revision();
    ok(StoredQueryList {
        queries: revision
            .queries
            .iter()
            .map(|query| StoredQueryView {
                name: query.name.clone(),
                description: query.description.clone(),
                writes: query.writes,
                parameters: query.parameters.clone(),
            })
            .collect(),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeBody {
    #[serde(default)]
    pub params: HashMap<String, Value>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    /// When set, the call is refused unless the query's mode matches, so a
    /// client never runs a write it believed was a read.
    #[serde(default)]
    pub expect_mutation: Option<bool>,
}

/// `POST /v1/queries/{name}`: run a stored query. `invoke_query` is
/// checked here; `read` or `change` on the branch by the Cypher path.
pub async fn invoke(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Path(name): Path<String>,
    body: Option<Json<InvokeBody>>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let revision = state.current_revision();
    let Some(query) = revision.query(&name).cloned() else {
        return Err(ApiError::not_found(
            "QUERY_NOT_FOUND",
            format!("no stored query is named {name}"),
        ));
    };
    let identity = identity.map(|Extension(identity)| identity);
    crate::policy_gate::authorize(
        &state,
        identity.as_ref(),
        &PolicyRequest::graph_wide(PolicyAction::InvokeQuery),
    )?;
    if body
        .expect_mutation
        .is_some_and(|expected| expected != query.writes)
    {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "QUERY_MODE_MISMATCH",
            message: format!(
                "stored query {name} {}",
                if query.writes { "writes" } else { "only reads" }
            ),
        });
    }
    let payload = ExecuteCypherRequest {
        query: query.text,
        params: body.params,
        mode: Some(if query.writes { "write" } else { "read" }.to_owned()),
        branch: body.branch,
        commit: body.commit,
        ..ExecuteCypherRequest::default()
    };
    crate::handlers::cypher::execute_stored(state, payload, query.writes, identity).await
}
