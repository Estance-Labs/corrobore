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

//! Enforcement of the graph policy on HTTP routes.
//!
//! Every protected route is listed in [`ROUTES`] with the action it performs
//! and where its branch comes from. The middleware decides routes whose
//! branch is in the path, the query or the JSON body; Cypher (whose action
//! depends on the statement) and branch creation (whose source may be a
//! commit) decide in their handlers with [`authorize`]. A route missing from
//! the table is denied whenever a policy is installed, so a new route cannot
//! slip past the policy unnoticed.

use axum::{
    Extension,
    body::Body,
    extract::{MatchedPath, Query, State},
    http::{Method, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use graph_storage::MAIN_BRANCH;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    app::AppState,
    auth::RequestIdentity,
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
    policy::{AccessReport, PolicyAction, PolicyRequest},
};

/// Where a route's branch comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchSource {
    /// The route acts on the whole graph, no branch.
    GraphWide,
    /// The route acts on `main`.
    Main,
    /// `?branch=`; `main` when absent, graph-wide when only `?commit=` is set.
    QueryBranch,
    /// `?from=&to=` or `?a=&b=`: each named branch is checked; commit ids
    /// are judged graph-wide.
    QueryPair(&'static str, &'static str),
    /// JSON body `branch`; `main` when absent, graph-wide with `commit` only.
    BodyBranch,
    /// JSON body `source` merged into `target` (default `main`).
    BodyMerge,
    /// The branch name in the path is the target.
    PathTarget,
    /// The handler decides with [`authorize`].
    Handler,
    /// No graph action: identity routes.
    Unchecked,
}

pub struct RouteRule {
    pub method: Method,
    pub path: &'static str,
    pub action: PolicyAction,
    pub branch: BranchSource,
}

macro_rules! route {
    ($method:ident $path:literal => $action:ident, $branch:expr) => {
        RouteRule {
            method: Method::$method,
            path: $path,
            action: PolicyAction::$action,
            branch: $branch,
        }
    };
}

/// The action and branch of every protected route.
pub fn routes() -> Vec<RouteRule> {
    use BranchSource::*;
    vec![
        route!(GET "/v1/whoami" => Read, Unchecked),
        route!(GET "/v1/graph/access" => Read, Unchecked),
        // Cypher.
        route!(POST "/v1/cypher/execute" => Read, Handler),
        route!(POST "/v1/cypher/read" => Read, Handler),
        route!(POST "/v1/cypher/write" => Change, Handler),
        route!(POST "/v1/cypher/validate" => Read, GraphWide),
        // History, branches and records.
        route!(GET "/v1/graph/commits" => Read, QueryBranch),
        route!(GET "/v1/graph/commits/{commit_id}" => Read, GraphWide),
        route!(GET "/v1/graph/branches" => Read, GraphWide),
        route!(POST "/v1/graph/branches" => BranchCreate, Handler),
        route!(DELETE "/v1/graph/branches/{*name}" => BranchDelete, PathTarget),
        route!(GET "/v1/graph/diff" => Read, QueryPair("from", "to")),
        route!(GET "/v1/graph/merge-base" => Read, QueryPair("a", "b")),
        route!(GET "/v1/graph/changes" => Read, QueryBranch),
        route!(GET "/v1/graph/records" => Read, QueryBranch),
        route!(GET "/v1/graph/nodes/{node_id}" => Read, QueryBranch),
        route!(GET "/v1/graph/schema" => Read, QueryBranch),
        route!(GET "/v1/graph/reviews" => Read, QueryBranch),
        route!(POST "/v1/graph/reviews" => Review, BodyBranch),
        route!(POST "/v1/graph/merge" => BranchMerge, BodyMerge),
        route!(POST "/v1/graph/edits" => Change, BodyBranch),
        route!(POST "/v1/graph/revert" => Change, BodyBranch),
        route!(POST "/v1/graph/search/vector" => Read, BodyBranch),
        route!(POST "/v1/graph/search/hybrid" => Read, BodyBranch),
        // Retrieval and analysis on main.
        route!(POST "/v1/seed/search" => Read, Main),
        route!(POST "/v1/nlq/compile" => Read, GraphWide),
        route!(POST "/v1/embeddings" => Read, GraphWide),
        route!(POST "/v1/domains/{domain}/validate" => Read, GraphWide),
        route!(POST "/v1/stix/validate" => Read, GraphWide),
        route!(GET "/v1/export/stix" => Export, Main),
        route!(POST "/v1/memory/operations" => Change, Main),
        // Runtime sessions and the explorer.
        route!(POST "/v1/sessions/start" => Change, Main),
        route!(POST "/v1/sessions/{session_id}/stop" => Change, Main),
        route!(GET "/v1/sessions/{session_id}/health" => Read, GraphWide),
        route!(GET "/v1/sessions/{session_id}/logs" => Read, GraphWide),
        route!(GET "/v1/explorer/sessions" => Read, GraphWide),
        route!(GET "/v1/explorer/sessions/{session_id}/timeline" => Read, GraphWide),
        route!(GET "/v1/explorer/sessions/{session_id}/graph" => Read, Main),
        // Claims and pipeline metrics.
        route!(POST "/v1/claims/{id}/decisions" => Change, Main),
        route!(GET "/v1/claims/{id}/audit" => Read, Main),
        route!(GET "/v1/epistemic/projection" => Read, Main),
        route!(GET "/v1/metrics/stages/{run_id}" => Read, GraphWide),
        route!(POST "/v1/metrics/stages/{run_id}" => Change, Main),
        // Imports and reconciliation write main.
        route!(POST "/v1/reconciliations" => Change, Main),
        route!(GET "/v1/reconciliations/{id}" => Read, Main),
        route!(POST "/v1/reconciliations/{id}/merge" => Change, Main),
        route!(POST "/v1/reconciliations/{id}/undo" => Change, Main),
        route!(POST "/v1/import/candidates" => Change, Main),
        route!(GET "/v1/import/candidates/{id}" => Read, Main),
        route!(POST "/v1/import/candidates/{id}/repairs" => Change, Main),
        route!(POST "/v1/import/candidates/{id}/promote" => Change, Main),
        route!(POST "/v1/import/stix" => Change, Main),
        route!(POST "/v1/import/stix/file" => Change, Main),
        // Content plane.
        route!(POST "/v1/content" => Change, GraphWide),
        route!(GET "/v1/content/{content_id}" => Read, GraphWide),
        // OpenCTI provider traffic.
        route!(POST "/v1/opencti/sync/batches" => Change, Main),
        route!(GET "/v1/opencti/sync/status" => Read, GraphWide),
        route!(POST "/v1/opencti/shadow/reads" => Read, Main),
        route!(GET "/v1/opencti/shadow/reports" => Read, GraphWide),
        route!(POST "/v1/opencti/reads" => Read, Main),
        route!(POST "/v1/opencti/writes" => Change, Main),
        route!(POST "/v1/opencti/files" => Change, Main),
        route!(GET "/v1/opencti/writes/status" => Read, GraphWide),
        route!(POST "/v1/opencti/reconciliation" => Change, Main),
        route!(GET "/v1/opencti/reconciliation/status" => Read, GraphWide),
        route!(GET "/v1/opencti/routing/decisions" => Read, GraphWide),
        route!(POST "/v1/opencti/routing/rollback" => Admin, GraphWide),
        // Deployment bundles and stored queries.
        route!(POST "/v1/cluster/deployments" => ConfigManage, GraphWide),
        route!(GET "/v1/cluster/status" => ConfigManage, GraphWide),
        route!(GET "/v1/queries" => Read, GraphWide),
        route!(POST "/v1/queries/{name}" => InvokeQuery, Handler),
    ]
}

/// The actor the policy judges: the credential's actor, or the actor the
/// shared token stands for when one is configured.
pub(crate) fn policy_actor(state: &AppState, identity: Option<&RequestIdentity>) -> Option<String> {
    let identity = identity?;
    identity.actor.clone().or_else(|| {
        (identity.method == crate::auth::AuthMethod::Static)
            .then(|| state.config.policy_shared_token_actor.clone())
            .flatten()
    })
}

/// Decide `request` for the caller; Ok when no policy is installed.
pub(crate) fn authorize(
    state: &AppState,
    identity: Option<&RequestIdentity>,
    request: &PolicyRequest,
) -> Result<(), ApiError> {
    let Some(engine) = state.current_policy() else {
        return Ok(());
    };
    let Some(actor) = policy_actor(state, identity) else {
        return Err(denied(None, request));
    };
    if engine.authorize(&actor, request).allowed {
        Ok(())
    } else {
        Err(denied(Some(&actor), request))
    }
}

fn denied(actor: Option<&str>, request: &PolicyRequest) -> ApiError {
    let who = actor.unwrap_or("a credential without an actor");
    let on = match (&request.branch, &request.target_branch) {
        (Some(branch), Some(target)) => format!(" from {branch} into {target}"),
        (Some(branch), None) => format!(" on {branch}"),
        (None, Some(target)) => format!(" on {target}"),
        (None, None) => String::new(),
    };
    ApiError::forbidden(
        "POLICY_DENIED",
        format!("{who} may not {}{on}", request.action.as_str()),
    )
}

/// A branch name, or `None` when `value` is a commit id (judged graph-wide).
fn branch_or_commit(value: &str) -> Option<String> {
    if graph_storage::GraphCommitId::new(value).is_ok() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn requests_for(
    rule: &RouteRule,
    query: &std::collections::HashMap<String, String>,
    path_target: Option<String>,
    body: Option<&Value>,
) -> Vec<PolicyRequest> {
    let action = rule.action;
    let main = || Some(MAIN_BRANCH.to_owned());
    let pick = |branch: Option<&str>, commit: bool| match branch {
        Some(branch) if !branch.trim().is_empty() => Some(branch.to_owned()),
        _ if commit => None,
        _ => main(),
    };
    match rule.branch {
        BranchSource::GraphWide => vec![PolicyRequest::graph_wide(action)],
        BranchSource::Main => vec![PolicyRequest {
            action,
            branch: main(),
            target_branch: None,
        }],
        BranchSource::QueryBranch => vec![PolicyRequest {
            action,
            branch: pick(
                query.get("branch").map(String::as_str),
                query.contains_key("commit"),
            ),
            target_branch: None,
        }],
        BranchSource::QueryPair(first, second) => [first, second]
            .into_iter()
            .map(|name| PolicyRequest {
                action,
                branch: query.get(name).and_then(|value| branch_or_commit(value)),
                target_branch: None,
            })
            .collect(),
        BranchSource::BodyBranch => {
            let field = |name: &str| body.and_then(|body| body.get(name)).and_then(Value::as_str);
            vec![PolicyRequest {
                action,
                branch: pick(field("branch"), field("commit").is_some()),
                target_branch: None,
            }]
        }
        BranchSource::BodyMerge => {
            let field = |name: &str| body.and_then(|body| body.get(name)).and_then(Value::as_str);
            vec![PolicyRequest {
                action,
                branch: field("source").map(str::to_owned),
                target_branch: pick(field("target"), false),
            }]
        }
        BranchSource::PathTarget => vec![PolicyRequest {
            action,
            branch: None,
            target_branch: path_target,
        }],
        BranchSource::Handler | BranchSource::Unchecked => Vec::new(),
    }
}

/// Route-layer middleware: decide each protected request before its handler.
pub async fn enforce_policy(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if state.current_policy().is_none() {
        return next.run(request).await;
    }
    let method = request.method().clone();
    let Some(path) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
    else {
        return ApiError::forbidden("POLICY_DENIED", "this route is not covered by the policy")
            .into_response();
    };
    let routes = routes();
    let Some(rule) = routes
        .iter()
        .find(|rule| rule.method == method && rule.path == path)
    else {
        return ApiError::forbidden("POLICY_DENIED", "this route is not covered by the policy")
            .into_response();
    };
    if matches!(rule.branch, BranchSource::Handler | BranchSource::Unchecked) {
        return next.run(request).await;
    }
    let identity = request.extensions().get::<RequestIdentity>().cloned();
    let query: std::collections::HashMap<String, String> = request
        .uri()
        .query()
        .and_then(|query| serde_urlencoded::from_str(query).ok())
        .unwrap_or_default();
    let path_target = matches!(rule.branch, BranchSource::PathTarget).then(|| {
        let prefix = path.trim_end_matches("{*name}");
        let raw = request
            .uri()
            .path()
            .strip_prefix(prefix)
            .unwrap_or_default();
        percent_decode(raw)
    });
    // Body-scoped routes are JSON: buffer the body (the route's own limit
    // still applies when the handler extracts it), read the branch fields,
    // and hand the same bytes on.
    let (request, body) = if matches!(
        rule.branch,
        BranchSource::BodyBranch | BranchSource::BodyMerge
    ) {
        let (parts, body) = request.into_parts();
        let limit = state.config.max_body_bytes;
        let bytes = match axum::body::to_bytes(body, limit).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return ApiError {
                    status: axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                    code: "PAYLOAD_TOO_LARGE",
                    message: "request body exceeds the configured limit".to_owned(),
                }
                .into_response();
            }
        };
        let value = serde_json::from_slice::<Value>(&bytes).ok();
        (Request::from_parts(parts, Body::from(bytes)), value)
    } else {
        (request, None)
    };
    for decision in requests_for(rule, &query, path_target, body.as_ref()) {
        if let Err(error) = authorize(&state, identity.as_ref(), &decision) {
            return error.into_response();
        }
    }
    next.run(request).await
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
}

#[derive(Debug, Default, Deserialize)]
pub struct AccessQuery {
    pub branch: Option<String>,
    pub target_branch: Option<String>,
}

/// `GET /v1/graph/access`: what the caller's credential may do. Without a
/// policy every action is allowed.
pub async fn access(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Query(query): Query<AccessQuery>,
) -> axum::Json<Envelope<AccessReport>> {
    let actor = policy_actor(&state, identity.as_deref());
    let report = match (state.current_policy(), actor) {
        (Some(engine), Some(actor)) => engine.access(
            &actor,
            query.branch.as_deref(),
            query.target_branch.as_deref(),
        ),
        (Some(_), None) => AccessReport {
            actor_id: None,
            graph_wide: Vec::new(),
            branch: query.branch,
            target_branch: query.target_branch,
            actions: Vec::new(),
        },
        (None, actor) => AccessReport {
            actor_id: actor,
            graph_wide: PolicyAction::ALL.to_vec(),
            actions: if query.branch.is_some() {
                PolicyAction::ALL
                    .into_iter()
                    .filter(|action| action.takes_branch() || *action == PolicyAction::BranchDelete)
                    .filter(|action| {
                        query.target_branch.is_some()
                            || !matches!(
                                action,
                                PolicyAction::BranchCreate | PolicyAction::BranchMerge
                            )
                    })
                    .collect()
            } else {
                Vec::new()
            },
            branch: query.branch,
            target_branch: query.target_branch,
        },
    };
    ok(report)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{BranchSource, routes};

    #[test]
    fn every_protected_route_is_in_the_table() {
        // Protected routes are the `/v1/` routes outside `/v1/admin/`; admin
        // routes check their own action with `require_admin_or_policy`.
        let table: HashSet<&str> = routes().iter().map(|rule| rule.path).collect();
        let source = include_str!("app.rs");
        let mut missing = Vec::new();
        for path in source
            .split('"')
            .filter(|text| text.starts_with("/v1/") && !text.starts_with("/v1/admin/"))
        {
            if !table.contains(path) {
                missing.push(path);
            }
        }
        assert!(
            missing.is_empty(),
            "routes missing from the policy table: {missing:?}"
        );
    }

    #[test]
    fn each_route_appears_once_per_method() {
        let mut seen = HashSet::new();
        for rule in routes() {
            assert!(
                seen.insert((rule.method.clone(), rule.path)),
                "{} {} is listed twice",
                rule.method,
                rule.path
            );
            if rule.branch == BranchSource::PathTarget {
                assert!(rule.path.ends_with("{*name}"));
            }
        }
    }
}
