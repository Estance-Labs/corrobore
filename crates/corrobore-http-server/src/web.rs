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
use std::path::Path;

use axum::{Router, routing::any};
use tower_http::services::{ServeDir, ServeFile};

use crate::error::ApiError;

/// Attach optional production static assets and SPA fallback behavior.
///
/// Serves immutable assets from the configured directory, resolves browser-side
/// routes to index.html, and keeps unknown `/v1` routes as
/// API 404 responses. Returning the router unchanged keeps API-only mode intact.
pub(crate) fn attach_web_delivery(router: Router, web_dir: Option<&str>) -> Router {
    let Some(web_dir) = web_dir else {
        return router;
    };
    let index = Path::new(web_dir).join("index.html");
    router
        .route("/v1/{*path}", any(api_not_found))
        .fallback_service(ServeDir::new(web_dir).fallback(ServeFile::new(index)))
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("API_ROUTE_NOT_FOUND", "API route not found")
}
