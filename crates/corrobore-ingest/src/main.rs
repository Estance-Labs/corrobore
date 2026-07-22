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
use std::{process::ExitCode, time::Duration};

use clap::Parser;
use corrobore_ingest::{CursorStore, IngestConfig, run_poll_cycle};
use tracing::{error, info};

/// TAXII 2.1 ingestion connector for Corrobore.
#[derive(Debug, Parser)]
#[command(name = "corrobore-ingest", version, about)]
struct Cli {
    /// Run a single poll cycle and exit instead of looping.
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = match IngestConfig::from_env() {
        Ok(config) => config,
        Err(config_error) => {
            error!(error = %config_error, "configuration loading failed");
            return ExitCode::FAILURE;
        }
    };

    let mut store = CursorStore::new(&config.state_dir);

    if cli.once {
        return match run_poll_cycle(&config, &mut store).await {
            Ok(_) => ExitCode::SUCCESS,
            Err(cycle_error) => {
                error!(error = %cycle_error, "poll cycle failed");
                ExitCode::FAILURE
            }
        };
    }

    info!(
        poll_interval_ms = config.poll_interval_ms,
        "starting ingestion loop; press Ctrl+C to stop"
    );

    loop {
        if let Err(cycle_error) = run_poll_cycle(&config, &mut store).await {
            // Keep looping on cycle failures: transient feed or Corrobore outages
            // must not kill the connector; the cursor was not advanced.
            error!(error = %cycle_error, "poll cycle failed; retrying next interval");
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(config.poll_interval_ms)) => {}
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received; stopping ingestion loop");
                return ExitCode::SUCCESS;
            }
        }
    }
}
