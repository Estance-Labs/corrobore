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
use corrobore_http_server::logging::{resolve_log_directory, resolve_log_filter};

#[test]
fn default_filter_is_info_when_no_overrides() {
    let filter = resolve_log_filter(0, None);
    assert_eq!(filter, "info");
}

#[test]
fn verbose_once_maps_to_debug_targets() {
    let filter = resolve_log_filter(1, None);
    assert!(filter.contains("corrobore_http_server=debug"));
    assert!(filter.contains("cypher_executor=debug"));
    assert!(filter.contains("graph_storage=debug"));
    assert!(filter.contains("shared_runtime=debug"));
}

#[test]
fn verbose_twice_maps_to_trace_targets() {
    let filter = resolve_log_filter(2, None);
    assert!(filter.contains("corrobore_http_server=trace"));
    assert!(filter.contains("cypher_executor=trace"));
    assert!(filter.contains("graph_storage=trace"));
    assert!(filter.contains("shared_runtime=trace"));
}

#[test]
fn rust_log_override_wins_over_verbose_flag() {
    let filter = resolve_log_filter(2, Some("warn,axum=info"));
    assert_eq!(filter, "warn,axum=info");
}

#[test]
fn default_log_directory_is_under_session_store_dir() {
    let path = resolve_log_directory(".corrobore-runtime", None);
    assert_eq!(
        path,
        std::path::Path::new(".corrobore-runtime").join("logs")
    );
}

#[test]
fn explicit_log_directory_override_is_respected() {
    let path = resolve_log_directory(".corrobore-runtime", Some("./custom-http-logs"));
    assert_eq!(path, std::path::Path::new("./custom-http-logs"));
}
