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
use std::net::SocketAddr;

use clap::{ArgAction, Parser};
use corrobore_http_server::{
    AppState, ServerConfig, build_router, install_shutdown_signal, logging::init_logging,
    serve_with_lifecycle,
};
use tokio::net::TcpListener;
use tracing::info;

#[derive(Debug, Parser)]
#[command(
    name = "corrobore-http-server",
    about = "Run the Corrobore HTTP server"
)]
struct CliArgs {
    /// Increases log verbosity. Use `-vv` for full trace.
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let args = CliArgs::parse();
    let rust_log = std::env::var("RUST_LOG").ok();
    let config = ServerConfig::from_env()?;
    let logging_runtime = init_logging(args.verbose, rust_log.as_deref(), &config.log_dir)?;
    let _logging_guard = logging_runtime.guard;

    info!(
        filter = %logging_runtime.filter,
        rust_log_override = rust_log.is_some(),
        session_log_path = %logging_runtime.session_log_path.display(),
        storage_mode = config.storage_mode.as_str(),
        storage_dir = config.storage_dir.as_deref().unwrap_or("disabled"),
        storage_require_fsync = config.storage_require_fsync,
        storage_strict_recovery = config.storage_strict_recovery,
        "logging initialized"
    );
    let state = AppState::new(config.clone())?;
    let app = build_router(state.clone());

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let shutdown_signal = install_shutdown_signal()?;
    let listener = TcpListener::bind(addr).await?;
    info!("corrobore-http-server listening on http://{}", addr);

    serve_with_lifecycle(listener, app, state, shutdown_signal).await?;

    Ok(())
}
