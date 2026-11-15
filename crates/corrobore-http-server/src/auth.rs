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

//! Request authentication.
//!
//! A protected request presents one bearer credential: the unnamed trusted
//! token, a named static token (`CORROBORE_HTTP_BEARER_TOKENS_JSON`), or an
//! OAuth access token from the configured issuer (issue #382). The middleware
//! resolves it to a [`RequestIdentity`] carried in the request extensions;
//! handlers read it to record who wrote a commit, and refuse a request body
//! that claims another actor.

use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tracing::debug;

use crate::{
    app::AppState,
    config::{NamedToken, ServerConfig},
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
    oauth::OAuthVerifier,
    security::AuthenticationMode,
};

/// How a request authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// `local-insecure` mode: no credential is checked.
    Anonymous,
    /// The unnamed token or a named static token.
    Static,
    /// A verified OAuth access token.
    Oauth,
}

/// Who made a request, as proven by its credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestIdentity {
    /// The actor a named credential proves; `None` for the unnamed trusted
    /// token and for anonymous requests, whose body may name an actor.
    pub actor: Option<String>,
    /// OAuth client the access token was issued to.
    pub client_id: Option<String>,
    pub method: AuthMethod,
    /// Scopes the access token carries.
    pub scopes: Vec<String>,
}

impl RequestIdentity {
    pub fn anonymous() -> Self {
        Self {
            actor: None,
            client_id: None,
            method: AuthMethod::Anonymous,
            scopes: Vec::new(),
        }
    }

    fn unnamed() -> Self {
        Self {
            method: AuthMethod::Static,
            ..Self::anonymous()
        }
    }
}

/// The actor a write records. A named identity owns its writes: a body actor
/// is accepted only when it repeats that identity, and refused with 403
/// `ACTOR_MISMATCH` otherwise. Without a named identity the body actor, when
/// present and not blank, is used as before.
pub(crate) fn resolve_write_actor(
    identity: Option<&RequestIdentity>,
    requested: Option<String>,
) -> Result<Option<String>, ApiError> {
    let requested = requested.filter(|actor| !actor.trim().is_empty());
    match identity.and_then(|identity| identity.actor.as_deref()) {
        Some(actor) => match requested {
            Some(requested) if requested != actor => Err(ApiError::forbidden(
                "ACTOR_MISMATCH",
                format!("the credential authenticates {actor}; it cannot write as {requested}"),
            )),
            _ => Ok(Some(actor.to_owned())),
        },
        None => Ok(requested),
    }
}

/// Why a credential was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthFailure {
    /// No credential can be accepted in this configuration.
    NotConfigured,
    /// Not a known static token, and not an access token.
    Invalid,
    /// An access token that failed verification; the reason is for logs.
    InvalidToken(String),
}

/// Checks presented credentials; shared by HTTP, Bolt and SQL.
#[derive(Debug)]
pub struct Authenticator {
    unnamed: Option<String>,
    named: Vec<NamedToken>,
    oauth: Option<OAuthVerifier>,
}

impl Authenticator {
    pub fn new(config: &ServerConfig) -> Self {
        Self {
            unnamed: config.auth_token.clone(),
            named: config.identity.named_tokens.clone(),
            oauth: config.identity.oauth.clone().map(OAuthVerifier::new),
        }
    }

    pub fn oauth(&self) -> Option<&OAuthVerifier> {
        self.oauth.as_ref()
    }

    /// Resolve a presented credential (the token itself, without `Bearer`).
    pub async fn authenticate(&self, presented: &str) -> Result<RequestIdentity, AuthFailure> {
        // Every static token is compared, in constant time, so the time taken
        // does not tell which one nearly matched.
        let mut matched: Option<RequestIdentity> = None;
        if let Some(unnamed) = self.unnamed.as_deref()
            && secret_matches(presented, unnamed)
        {
            matched = Some(RequestIdentity::unnamed());
        }
        for named in &self.named {
            if secret_matches(presented, &named.token) {
                matched = Some(RequestIdentity {
                    actor: Some(named.actor.clone()),
                    ..RequestIdentity::unnamed()
                });
            }
        }
        if let Some(identity) = matched {
            return Ok(identity);
        }
        match &self.oauth {
            Some(verifier) if OAuthVerifier::looks_like_jwt(presented) => {
                let verified = verifier
                    .verify(presented)
                    .await
                    .map_err(AuthFailure::InvalidToken)?;
                Ok(RequestIdentity {
                    actor: Some(verified.actor),
                    client_id: verified.client_id,
                    method: AuthMethod::Oauth,
                    scopes: verified.scopes,
                })
            }
            Some(_) => Err(AuthFailure::InvalidToken("not a known token".to_owned())),
            None if self.unnamed.is_none() && self.named.is_empty() => {
                Err(AuthFailure::NotConfigured)
            }
            None => Err(AuthFailure::Invalid),
        }
    }
}

/// The credential of an `Authorization: Bearer <token>` header value.
fn bearer_credential(value: &str) -> Option<&str> {
    let (scheme, credential) = value.trim().split_once(' ')?;
    let credential = credential.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !credential.is_empty()).then_some(credential)
}

pub async fn require_bearer_auth(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if state.config.auth_mode == AuthenticationMode::LocalInsecure {
        request
            .extensions_mut()
            .insert(RequestIdentity::anonymous());
        return next.run(request).await;
    }
    let Some(value) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return unauthorized(
            &state,
            ApiError::unauthorized("AUTH_REQUIRED", "missing Authorization header"),
            false,
        );
    };
    let Some(credential) = bearer_credential(value) else {
        return unauthorized(
            &state,
            ApiError::unauthorized("AUTH_INVALID", "invalid bearer token"),
            false,
        );
    };
    match state.authenticator.authenticate(credential).await {
        Ok(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(AuthFailure::NotConfigured) => unauthorized(
            &state,
            ApiError::unauthorized(
                "AUTH_NOT_CONFIGURED",
                "bearer authentication is not configured",
            ),
            false,
        ),
        Err(AuthFailure::Invalid) => unauthorized(
            &state,
            ApiError::unauthorized("AUTH_INVALID", "invalid bearer token"),
            false,
        ),
        Err(AuthFailure::InvalidToken(reason)) => {
            debug!(reason = %reason, "access token refused");
            unauthorized(
                &state,
                ApiError::unauthorized("AUTH_INVALID", "invalid bearer token"),
                true,
            )
        }
    }
}

/// A 401 that, with OAuth enabled, tells the client where the resource
/// metadata is (RFC 9728 section 5.1) and, for a refused token, that the
/// token is invalid (RFC 6750 section 3.1).
fn unauthorized(state: &AppState, error: ApiError, invalid_token: bool) -> Response {
    let mut response = error.into_response();
    if let Some(oauth) = state.authenticator.oauth() {
        let metadata = oauth.config().resource_metadata_url();
        let challenge = if invalid_token {
            format!("Bearer error=\"invalid_token\", resource_metadata=\"{metadata}\"")
        } else {
            format!("Bearer resource_metadata=\"{metadata}\"")
        };
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
    }
    response
}

/// `GET /v1/whoami`: what the caller's credential proves.
pub async fn whoami(
    identity: Option<Extension<RequestIdentity>>,
) -> Json<Envelope<RequestIdentity>> {
    ok(identity.map_or_else(RequestIdentity::anonymous, |Extension(identity)| identity))
}

/// RFC 9728 protected-resource metadata.
#[derive(Debug, Serialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub bearer_methods_supported: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes_supported: Vec<String>,
    pub resource_name: &'static str,
}

/// `GET /.well-known/oauth-protected-resource`: public; 404 without OAuth.
pub async fn protected_resource_metadata(State(state): State<AppState>) -> Response {
    let Some(oauth) = state.authenticator.oauth() else {
        return ApiError::not_found(
            "OAUTH_NOT_CONFIGURED",
            "this server accepts no OAuth tokens",
        )
        .into_response();
    };
    let config = oauth.config();
    (
        StatusCode::OK,
        Json(ProtectedResourceMetadata {
            resource: config.audience.clone(),
            authorization_servers: vec![config.issuer.clone()],
            bearer_methods_supported: vec!["header"],
            scopes_supported: config.scopes.clone(),
            resource_name: "Corrobore",
        }),
    )
        .into_response()
}

pub(crate) fn require_admin_auth(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let admin_token = state.config.admin_auth_token.as_deref().ok_or_else(|| {
        ApiError::forbidden(
            "ADMIN_AUTH_NOT_CONFIGURED",
            "admin endpoint requires CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        )
    })?;
    let expected = format!("Bearer {admin_token}");
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("AUTH_REQUIRED", "missing Authorization header"))?;
    if !token_matches(provided, &expected) {
        return Err(ApiError::unauthorized(
            "AUTH_INVALID",
            "invalid admin bearer token",
        ));
    }
    Ok(())
}

/// Whether a presented secret equals a configured one, in constant time.
pub(crate) fn secret_matches(provided: &str, expected: &str) -> bool {
    token_matches(provided, expected)
}

/// Compares the provided `Authorization` header value against the expected
/// `Bearer <token>` string in constant time.
///
/// 2.2: `!=` on `&str` short-circuits at the first differing byte, leaking
/// timing information about how many leading bytes matched. This helper uses
/// `subtle::ConstantTimeEq` so comparison time does not depend on the content
/// of the provided value. A length mismatch still returns `false` (and may
/// leak the expected length), but the secret bytes are never compared in a
/// short-circuiting manner.
pub(crate) fn token_matches(provided: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;

    let provided = provided.as_bytes();
    let expected = expected.as_bytes();

    provided.len() == expected.len() && provided.ct_eq(expected).into()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{RequestIdentity, bearer_credential, resolve_write_actor, token_matches};

    #[test]
    fn token_matches_accepts_exact_value() {
        assert!(token_matches("Bearer token-123", "Bearer token-123"));
    }

    #[test]
    fn token_matches_rejects_same_length_difference() {
        assert!(!token_matches("Bearer token-124", "Bearer token-123"));
    }

    #[test]
    fn token_matches_rejects_length_difference() {
        assert!(!token_matches("Bearer token-123-extra", "Bearer token-123"));
        assert!(!token_matches("Bearer tok", "Bearer token-123"));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_credential("Bearer abc"), Some("abc"));
        assert_eq!(bearer_credential("bearer  abc "), Some("abc"));
        assert_eq!(bearer_credential("Basic abc"), None);
        assert_eq!(bearer_credential("Bearer "), None);
    }

    #[test]
    fn a_named_identity_owns_its_writes() {
        let named = RequestIdentity {
            actor: Some("act-alice".to_owned()),
            ..RequestIdentity::anonymous()
        };
        assert_eq!(
            resolve_write_actor(Some(&named), None).unwrap(),
            Some("act-alice".to_owned())
        );
        assert_eq!(
            resolve_write_actor(Some(&named), Some(" ".to_owned())).unwrap(),
            Some("act-alice".to_owned())
        );
        let error = resolve_write_actor(Some(&named), Some("act-bob".to_owned())).unwrap_err();
        assert_eq!(error.code, "ACTOR_MISMATCH");
        let unnamed = RequestIdentity::anonymous();
        assert_eq!(
            resolve_write_actor(Some(&unnamed), Some("importer".to_owned())).unwrap(),
            Some("importer".to_owned())
        );
        assert_eq!(resolve_write_actor(None, None).unwrap(), None);
    }
}
