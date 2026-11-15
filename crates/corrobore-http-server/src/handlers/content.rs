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

//! `POST /v1/content` and `GET /v1/content/{content_id}` (issue #341).

use std::ops::Range;

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use graph_core::{ContentHandle, ContentPlacement, ContentStoreError};
use serde::Serialize;
use serde_json::Value;

use crate::{app::AppState, content_plane::ContentReadError, error::ApiError};

/// What an upload became.
#[derive(Debug, Serialize)]
pub struct ContentUploadResponse {
    pub ok: bool,
    /// `inline` (kept in `handle`, not retained) or `offloaded`.
    pub placement: ContentPlacement,
    pub policy_version: String,
    /// Set for offloaded content: the id `GET /v1/content/{content_id}` reads.
    pub content_id: Option<String>,
    pub sha256: String,
    pub byte_length: u64,
    pub media_type: Option<String>,
    /// Backend holding offloaded content.
    pub backend: Option<String>,
    /// The content handle a record stores.
    pub handle: Value,
}

fn store_error(error: ContentStoreError) -> ApiError {
    match error {
        ContentStoreError::Invalid(message) => ApiError::bad_request("INVALID_CONTENT", message),
        ContentStoreError::NotRetained(content_id) => ApiError::not_found(
            "CONTENT_NOT_RETAINED",
            format!("content {content_id} is not retained by this server"),
        ),
        ContentStoreError::Erased { content_id, reason } => ApiError {
            status: StatusCode::GONE,
            code: "CONTENT_ERASED",
            message: format!("content {content_id} was erased: {reason}"),
        },
        ContentStoreError::DigestMismatch { content_id } => ApiError::internal(
            "CONTENT_DIGEST_MISMATCH",
            format!("content {content_id} no longer matches its digest"),
        ),
        other => ApiError::internal("CONTENT_STORE_FAILED", other.to_string()),
    }
}

fn read_error(error: ContentReadError) -> ApiError {
    match error {
        ContentReadError::InvalidId => {
            ApiError::bad_request("INVALID_CONTENT_ID", error.to_string())
        }
        ContentReadError::Store(error) => store_error(error),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    // Object store backends block on their own runtime, which must not run on
    // an async worker.
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ApiError::internal("CONTENT_TASK_FAILED", error.to_string()))?
}

/// Store content by policy.
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<ContentUploadResponse>), ApiError> {
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let plane = state.content.clone();
    let decision = blocking(move || {
        let mut plane = plane
            .write()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "content plane lock poisoned"))?;
        plane
            .ingest(&body, media_type.as_deref())
            .map_err(store_error)
    })
    .await?;
    let handle = decision.handle();
    let (content_id, backend) = match handle {
        ContentHandle::External(reference) => (
            Some(reference.content_id().to_owned()),
            Some(reference.backend().to_owned()),
        ),
        ContentHandle::Inline(_) => (None, None),
    };
    let response = ContentUploadResponse {
        ok: true,
        placement: decision.placement(),
        policy_version: decision.policy_version().to_owned(),
        content_id,
        sha256: handle.sha256(),
        byte_length: handle.byte_length(),
        media_type: handle.media_type().map(str::to_owned),
        backend,
        handle: serde_json::to_value(handle)
            .map_err(|error| ApiError::internal("CONTENT_ENCODING_FAILED", error.to_string()))?,
    };
    Ok((StatusCode::CREATED, Json(response)))
}

/// The single byte range a `Range` header asks for, when this server serves it.
#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    /// No range, or a form this server answers with the whole content.
    Whole,
    /// A satisfiable window.
    Window(Range<u64>),
    /// A range wholly outside the content.
    Unsatisfiable,
}

/// Parse one `bytes=` range against a content length (RFC 9110 §14.1.2).
fn parse_range(value: Option<&str>, length: u64) -> RangeRequest {
    let Some(spec) = value.and_then(|value| value.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Whole;
    };
    if spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((start, end)) = spec.trim().split_once('-') else {
        return RangeRequest::Whole;
    };
    let window = match (start.trim(), end.trim()) {
        ("", "") => return RangeRequest::Whole,
        ("", suffix) => match suffix.parse::<u64>() {
            Ok(0) => return RangeRequest::Unsatisfiable,
            Ok(suffix) => length.saturating_sub(suffix)..length,
            Err(_) => return RangeRequest::Whole,
        },
        (start, end) => {
            let Ok(start) = start.parse::<u64>() else {
                return RangeRequest::Whole;
            };
            let end = if end.is_empty() {
                length.saturating_sub(1)
            } else {
                match end.parse::<u64>() {
                    Ok(end) if end >= start => end.min(length.saturating_sub(1)),
                    _ => return RangeRequest::Whole,
                }
            };
            if start >= length {
                return RangeRequest::Unsatisfiable;
            }
            start..end + 1
        }
    };
    if window.is_empty() {
        RangeRequest::Unsatisfiable
    } else {
        RangeRequest::Window(window)
    }
}

/// Read offloaded content, honouring a single `Range`.
pub async fn read(
    State(state): State<AppState>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let range_header = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let plane = state.content.clone();
    let (reference, request, bytes) = blocking(move || {
        let plane = plane
            .read()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "content plane lock poisoned"))?;
        let reference = plane.reference(&content_id).map_err(read_error)?;
        let request = parse_range(range_header.as_deref(), reference.byte_length());
        let bytes = match &request {
            RangeRequest::Whole => Some(plane.read(&reference, None).map_err(store_error)?),
            RangeRequest::Window(window) => Some(
                plane
                    .read(&reference, Some(window.clone()))
                    .map_err(store_error)?,
            ),
            RangeRequest::Unsatisfiable => None,
        };
        Ok((reference, request, bytes))
    })
    .await?;

    let length = reference.byte_length();
    let mut response = match (request, bytes) {
        (RangeRequest::Window(window), Some(bytes)) => {
            let mut response = (StatusCode::PARTIAL_CONTENT, Body::from(bytes)).into_response();
            if let Ok(value) = HeaderValue::from_str(&format!(
                "bytes {}-{}/{length}",
                window.start,
                window.end - 1
            )) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            response
        }
        (_, Some(bytes)) => (StatusCode::OK, Body::from(bytes)).into_response(),
        (_, None) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{length}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(response);
        }
    };
    let response_headers = response.headers_mut();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let media_type = reference.media_type().unwrap_or("application/octet-stream");
    if let Ok(value) = HeaderValue::from_str(media_type) {
        response_headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", reference.sha256())) {
        response_headers.insert(header::ETAG, value);
    }
    // Content is immutable under its digest.
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{RangeRequest, parse_range};

    #[test]
    fn ranges_follow_rfc_9110() {
        assert_eq!(parse_range(None, 100), RangeRequest::Whole);
        assert_eq!(
            parse_range(Some("bytes=0-9"), 100),
            RangeRequest::Window(0..10)
        );
        assert_eq!(
            parse_range(Some("bytes=90-"), 100),
            RangeRequest::Window(90..100)
        );
        assert_eq!(
            parse_range(Some("bytes=-5"), 100),
            RangeRequest::Window(95..100)
        );
        assert_eq!(
            parse_range(Some("bytes=-500"), 100),
            RangeRequest::Window(0..100)
        );
        assert_eq!(
            parse_range(Some("bytes=95-500"), 100),
            RangeRequest::Window(95..100)
        );
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=-0"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(parse_range(Some("bytes=5-1"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=0-1,4-5"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("items=0-1"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=a-b"), 100), RangeRequest::Whole);
    }
}
