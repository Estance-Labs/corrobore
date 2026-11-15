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

//! Issue #392: a build without the Lance cold tier refuses the setting
//! instead of silently dropping history.
#![cfg(not(feature = "lance-cold-tier"))]
#![allow(clippy::unwrap_used)]

use corrobore_http_server::ServerConfig;

#[test]
fn a_cold_tier_needs_the_feature() {
    let vars = std::collections::HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned()),
        (
            "CORROBORE_STORAGE_COLD_TIER_URL".to_owned(),
            "s3://archive/graph".to_owned(),
        ),
    ]);
    let error = ServerConfig::from_map(&vars).unwrap_err().to_string();
    assert!(error.contains("lance-cold-tier"), "{error}");
    let vars = std::collections::HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned()),
        (
            "CORROBORE_STORAGE_COLD_TIER_ENDPOINT".to_owned(),
            "http://localhost:9000".to_owned(),
        ),
    ]);
    assert!(ServerConfig::from_map(&vars).is_err());
}
