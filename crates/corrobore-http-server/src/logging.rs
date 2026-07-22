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
use std::{
    fs,
    path::{Path, PathBuf},
};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub const SESSION_LOG_FILE_NAME: &str = "http-server.session.log.jsonl";

pub struct LoggingRuntime {
    pub guard: WorkerGuard,
    pub filter: String,
    pub session_log_path: PathBuf,
}

/// Resolves the effective `tracing` filter string for the HTTP server process.
///
/// Intended behavior:
/// - Respect an explicit `RUST_LOG` value when provided.
/// - Otherwise map CLI verbosity (`-v`, `-vv`) to progressively detailed targets.
/// - Keep a stable `info` default for backward compatibility.
pub fn resolve_log_filter(verbose: u8, rust_log_override: Option<&str>) -> String {
    if let Some(filter) = rust_log_override {
        let trimmed = filter.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    match verbose {
        0 => "info".to_owned(),
        1 => [
            "info",
            "corrobore_http_server=debug",
            "cypher_executor=debug",
            "graph_storage=debug",
            "shared_runtime=debug",
        ]
        .join(","),
        _ => [
            "info",
            "corrobore_http_server=trace",
            "cypher_executor=trace",
            "graph_storage=trace",
            "shared_runtime=trace",
            "tower_http=trace",
        ]
        .join(","),
    }
}

pub fn resolve_log_directory(session_store_dir: &str, log_dir_override: Option<&str>) -> PathBuf {
    if let Some(override_value) = log_dir_override {
        let trimmed = override_value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    Path::new(session_store_dir).join("logs")
}

pub fn init_logging(
    verbose: u8,
    rust_log_override: Option<&str>,
    log_dir: &str,
) -> Result<LoggingRuntime, Box<dyn std::error::Error>> {
    let filter = resolve_log_filter(verbose, rust_log_override);
    let log_dir = PathBuf::from(log_dir);
    fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::never(&log_dir, SESSION_LOG_FILE_NAME);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let stdout_layer = tracing_subscriber::fmt::layer().with_target(true);
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_ansi(false)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(filter.clone()))
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(LoggingRuntime {
        guard,
        filter,
        session_log_path: log_dir.join(SESSION_LOG_FILE_NAME),
    })
}
