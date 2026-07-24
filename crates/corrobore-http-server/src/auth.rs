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
use axum::{
    extract::State,
    http::{HeaderMap, Request, header},
    middleware::Next,
    response::Response,
};

use crate::{app::AppState, error::ApiError, security::AuthenticationMode};

pub async fn require_bearer_auth(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if state.config.auth_mode == AuthenticationMode::LocalInsecure {
        return Ok(next.run(request).await);
    }
    let Some(auth_token) = state.config.auth_token.as_deref() else {
        return Err(ApiError::unauthorized(
            "AUTH_NOT_CONFIGURED",
            "bearer authentication is not configured",
        ));
    };
    let expected = format!("Bearer {auth_token}");

    let value = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header_value| header_value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("AUTH_REQUIRED", "missing Authorization header"))?;

    if !token_matches(value, &expected) {
        return Err(ApiError::unauthorized(
            "AUTH_INVALID",
            "invalid bearer token",
        ));
    }

    Ok(next.run(request).await)
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
mod tests {
    use super::token_matches;

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
}
