// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Request-correlation boundary shared by HTTP responses, structured request
//! spans, and stable API error envelopes.

use axum::{
    body::Body,
    http::{HeaderValue, Request},
    middleware::Next,
    response::Response,
};
use tracing::Instrument;
use uuid::Uuid;

pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const CORRELATION_ID_HEADER: &str = "x-correlation-id";
const MAX_CLIENT_ID_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub struct RequestCorrelationId(pub String);

tokio::task_local! {
    static CURRENT_CORRELATION_ID: String;
}

pub fn current_correlation_id() -> Option<String> {
    CURRENT_CORRELATION_ID.try_with(Clone::clone).ok()
}

/// Preserve a bounded, log-safe client identifier or generate a UUID, then
/// scope downstream request handling so errors and logs use the same value.
pub async fn correlate_request(mut request: Request<Body>, next: Next) -> Response {
    let correlation_id =
        client_correlation_id(&request).unwrap_or_else(|| Uuid::new_v4().to_string());
    request
        .extensions_mut()
        .insert(RequestCorrelationId(correlation_id.clone()));
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "http_request",
        correlation_id = %correlation_id,
        %method,
        %path,
    );
    let scoped_id = correlation_id.clone();
    let mut response = CURRENT_CORRELATION_ID
        .scope(scoped_id, next.run(request).instrument(span))
        .await;
    let header = HeaderValue::from_str(&correlation_id)
        .expect("validated or generated correlation IDs are valid headers");
    response
        .headers_mut()
        .insert(REQUEST_ID_HEADER, header.clone());
    response.headers_mut().insert(CORRELATION_ID_HEADER, header);
    response
}

fn client_correlation_id(request: &Request<Body>) -> Option<String> {
    [REQUEST_ID_HEADER, CORRELATION_ID_HEADER]
        .into_iter()
        .find_map(|name| request.headers().get(name))
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_CLIENT_ID_BYTES
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
                })
        })
        .map(str::to_owned)
}
