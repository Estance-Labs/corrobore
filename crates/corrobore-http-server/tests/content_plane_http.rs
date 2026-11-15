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

//! Issue #341: the content plane (ADR-0021) served over HTTP. Small content
//! stays inline in the returned handle; larger content is offloaded to the
//! configured object store and read back by content id, whole or by range.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{
    TOKEN, app_state_error, app_with, ephemeral_app, persistent_app, send_bytes, unique_path,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const SMALL_INLINE: &[(&str, &str)] = &[("CORROBORE_HTTP_CONTENT_INLINE_MAX_BYTES", "16")];

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn payload(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

async fn upload(app: &axum::Router, bytes: &[u8], media_type: &str) -> (StatusCode, Value) {
    let (status, _, body) = send_bytes(
        app,
        Some(TOKEN),
        Method::POST,
        "/v1/content",
        bytes.to_vec(),
        &[("content-type", media_type)],
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn read(
    app: &axum::Router,
    content_id: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    send_bytes(
        app,
        Some(TOKEN),
        Method::GET,
        &format!("/v1/content/{content_id}"),
        Vec::new(),
        headers,
    )
    .await
}

fn header<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("missing header {name}"))
        .to_str()
        .unwrap()
}

#[tokio::test]
async fn small_text_stays_inline_and_is_not_retained() {
    let app = ephemeral_app();
    let (status, body) = upload(&app, b"Aster operates the North Relay.", "text/plain").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["placement"], "inline");
    assert_eq!(body["content_id"], Value::Null);
    assert_eq!(body["sha256"], digest(b"Aster operates the North Relay."));
    assert_eq!(body["byte_length"], 31);
    assert!(
        body["policy_version"]
            .as_str()
            .unwrap()
            .starts_with("content-policy")
    );
    assert!(body["handle"].is_object());

    let id = format!("content--{}", digest(b"Aster operates the North Relay."));
    let (status, _, body) = read(&app, &id, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "CONTENT_NOT_RETAINED", "{body}");
}

#[tokio::test]
async fn large_content_is_offloaded_under_the_storage_root_and_read_back() {
    let storage = unique_path("content-offload");
    let app = app_with(Some(&storage), SMALL_INLINE);
    let bytes = payload(100);
    let (status, body) = upload(&app, &bytes, "application/octet-stream").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["placement"], "offloaded");
    let content_id = format!("content--{}", digest(&bytes));
    assert_eq!(body["content_id"], content_id);
    assert_eq!(body["byte_length"], 100);
    assert_eq!(body["backend"], "filesystem");
    let sha = digest(&bytes);
    assert!(
        storage
            .join("content")
            .join(&sha[0..2])
            .join(&sha[2..4])
            .join(&sha)
            .is_file(),
        "the blob lives under <storage>/content"
    );

    let (status, headers, served) = read(&app, &content_id, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, bytes);
    assert_eq!(header(&headers, "content-type"), "application/octet-stream");
    assert_eq!(header(&headers, "accept-ranges"), "bytes");
    assert_eq!(header(&headers, "content-length"), "100");
    assert_eq!(header(&headers, "etag"), format!("\"{sha}\""));
}

#[tokio::test]
async fn ranges_are_served_as_partial_content() {
    let app = app_with(None, SMALL_INLINE);
    let bytes = payload(100);
    let (_, body) = upload(&app, &bytes, "application/octet-stream").await;
    let id = body["content_id"].as_str().unwrap().to_owned();

    let (status, headers, served) = read(&app, &id, &[("range", "bytes=10-19")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(served, bytes[10..20]);
    assert_eq!(header(&headers, "content-range"), "bytes 10-19/100");

    let (status, headers, served) = read(&app, &id, &[("range", "bytes=90-")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(served, bytes[90..]);
    assert_eq!(header(&headers, "content-range"), "bytes 90-99/100");

    let (status, _, served) = read(&app, &id, &[("range", "bytes=-5")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(served, bytes[95..]);

    // An end past the content is clamped, as RFC 9110 requires.
    let (status, headers, served) = read(&app, &id, &[("range", "bytes=95-500")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(served, bytes[95..]);
    assert_eq!(header(&headers, "content-range"), "bytes 95-99/100");

    let (status, headers, _) = read(&app, &id, &[("range", "bytes=200-300")]).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&headers, "content-range"), "bytes */100");

    // Range forms this server does not serve fall back to the whole content.
    for unsupported in ["bytes=0-1,5-6", "items=0-1", "bytes=x-y"] {
        let (status, _, served) = read(&app, &id, &[("range", unsupported)]).await;
        assert_eq!(status, StatusCode::OK, "{unsupported}");
        assert_eq!(served, bytes, "{unsupported}");
    }
}

#[tokio::test]
async fn identical_bytes_share_one_content_id() {
    let app = app_with(None, SMALL_INLINE);
    let bytes = payload(64);
    let (first_status, first) = upload(&app, &bytes, "application/octet-stream").await;
    let (second_status, second) = upload(&app, &bytes, "application/octet-stream").await;
    assert_eq!(first_status, StatusCode::CREATED);
    assert_eq!(second_status, StatusCode::CREATED);
    assert_eq!(first["content_id"], second["content_id"]);
}

#[tokio::test]
async fn the_media_type_is_kept_with_the_content() {
    let app = app_with(None, SMALL_INLINE);
    let csv = b"id,name\n1,Aster\n2,North Relay\n3,Harbour\n".to_vec();
    let (status, body) = upload(&app, &csv, "text/csv").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["media_type"], "text/csv");
    let (status, headers, served) = read(&app, body["content_id"].as_str().unwrap(), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, csv);
    assert_eq!(header(&headers, "content-type"), "text/csv");
}

#[tokio::test]
async fn offloaded_content_survives_a_restart() {
    let storage = unique_path("content-restart");
    let bytes = payload(80);
    let id = {
        let app = app_with(Some(&storage), SMALL_INLINE);
        let (_, body) = upload(&app, &bytes, "application/octet-stream").await;
        body["content_id"].as_str().unwrap().to_owned()
    };
    let reopened = persistent_app(&storage);
    let (status, _, served) = read(&reopened, &id, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, bytes);
}

#[tokio::test]
async fn an_object_store_url_selects_where_content_lives() {
    let objects = unique_path("content-url-store");
    std::fs::create_dir_all(&objects).unwrap();
    let url = format!("file://{}", objects.display());
    let app = app_with(
        None,
        &[
            ("CORROBORE_HTTP_CONTENT_STORE_URL", url.as_str()),
            ("CORROBORE_HTTP_CONTENT_INLINE_MAX_BYTES", "16"),
        ],
    );
    let bytes = payload(50);
    let (status, body) = upload(&app, &bytes, "application/octet-stream").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["backend"], "file");
    let sha = digest(&bytes);
    assert!(
        objects
            .join(&sha[0..2])
            .join(&sha[2..4])
            .join(&sha)
            .is_file()
    );
    let (status, _, served) = read(&app, body["content_id"].as_str().unwrap(), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, bytes);
}

#[test]
fn an_unusable_object_store_url_fails_startup() {
    let error = app_state_error(
        None,
        &[("CORROBORE_HTTP_CONTENT_STORE_URL", "ftp://example.test/x")],
    )
    .expect("startup must fail");
    assert!(error.contains("content store"), "{error}");
}

#[tokio::test]
async fn invalid_requests_are_rejected() {
    let app = app_with(
        None,
        &[
            ("CORROBORE_HTTP_CONTENT_INLINE_MAX_BYTES", "16"),
            ("CORROBORE_HTTP_CONTENT_MAX_BYTES", "64"),
        ],
    );
    let (status, body) = upload(&app, b"", "text/plain").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "INVALID_CONTENT", "{body}");

    let (status, _) = upload(&app, &payload(100), "application/octet-stream").await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    for id in ["not-an-id", "content--XYZ", "content--abc"] {
        let (status, _, body) = read(&app, id, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "INVALID_CONTENT_ID", "{body}");
    }

    let (status, _, _) = send_bytes(
        &app,
        None,
        Method::POST,
        "/v1/content",
        b"anonymous".to_vec(),
        &[("content-type", "text/plain")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
