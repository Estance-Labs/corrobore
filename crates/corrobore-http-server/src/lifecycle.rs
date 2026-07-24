// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    fs,
    future::{Future, IntoFuture},
    net::SocketAddr,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::Router;
use thiserror::Error;
use tokio::{net::TcpListener, sync::oneshot};

use crate::{AppState, RuntimeStoreProvider};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LifecycleState {
    Initializing = 0,
    Ready = 1,
    Draining = 2,
    Stopped = 3,
    Failed = 4,
}

#[derive(Debug)]
pub struct ServerLifecycle {
    state: AtomicU8,
    active_requests: AtomicUsize,
    shutdown_started: AtomicU64,
    shutdown_failures: AtomicU64,
}

#[derive(Debug)]
pub struct RequestActivityGuard {
    lifecycle: Arc<ServerLifecycle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleRejection;

#[derive(Debug, Error)]
pub enum ServerLifecycleError {
    #[error("server listener failed: {reason}")]
    ListenerFailed { reason: String },
    #[error("shutdown timeout expired after {timeout_ms} ms")]
    ShutdownTimeout { timeout_ms: u64 },
    #[error("persistent storage flush failed at {path}: {reason}")]
    PersistentFlushFailed { path: String, reason: String },
    #[error("failed to install shutdown signal handler: {reason}")]
    SignalHandlerFailed { reason: String },
}

pub struct ShutdownSignal {
    future: Pin<Box<dyn Future<Output = ()> + Send>>,
}

pub fn install_shutdown_signal() -> Result<ShutdownSignal, ServerLifecycleError> {
    termination_signal()
        .map(|future| ShutdownSignal { future })
        .map_err(|error| ServerLifecycleError::SignalHandlerFailed {
            reason: error.to_string(),
        })
}

impl ServerLifecycle {
    pub fn initializing() -> Self {
        Self {
            state: AtomicU8::new(LifecycleState::Initializing as u8),
            active_requests: AtomicUsize::new(0),
            shutdown_started: AtomicU64::new(0),
            shutdown_failures: AtomicU64::new(0),
        }
    }

    pub fn state(&self) -> LifecycleState {
        match self.state.load(Ordering::Acquire) {
            0 => LifecycleState::Initializing,
            1 => LifecycleState::Ready,
            2 => LifecycleState::Draining,
            3 => LifecycleState::Stopped,
            _ => LifecycleState::Failed,
        }
    }

    pub fn mark_ready(&self) {
        let _ = self.state.compare_exchange(
            LifecycleState::Initializing as u8,
            LifecycleState::Ready as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub fn try_begin_request(self: &Arc<Self>) -> Result<RequestActivityGuard, LifecycleRejection> {
        if self.state() != LifecycleState::Ready {
            return Err(LifecycleRejection);
        }
        self.active_requests.fetch_add(1, Ordering::AcqRel);
        if self.state() != LifecycleState::Ready {
            self.active_requests.fetch_sub(1, Ordering::AcqRel);
            return Err(LifecycleRejection);
        }
        Ok(RequestActivityGuard {
            lifecycle: Arc::clone(self),
        })
    }

    pub fn active_requests(&self) -> usize {
        self.active_requests.load(Ordering::Acquire)
    }

    pub fn begin_draining(&self) {
        if self
            .state
            .compare_exchange(
                LifecycleState::Ready as u8,
                LifecycleState::Draining as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.shutdown_started.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub fn shutdown_started(&self) -> u64 {
        self.shutdown_started.load(Ordering::Acquire)
    }

    pub fn shutdown_failures(&self) -> u64 {
        self.shutdown_failures.load(Ordering::Acquire)
    }

    fn finish(&self, failed: bool) {
        if failed {
            self.shutdown_failures.fetch_add(1, Ordering::AcqRel);
        }
        self.state.store(
            if failed {
                LifecycleState::Failed as u8
            } else {
                LifecycleState::Stopped as u8
            },
            Ordering::Release,
        );
    }
}

impl LifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

impl Drop for RequestActivityGuard {
    fn drop(&mut self) {
        self.lifecycle
            .active_requests
            .fetch_sub(1, Ordering::AcqRel);
    }
}

pub async fn serve_with_lifecycle(
    listener: TcpListener,
    app: Router,
    state: AppState,
    signal: ShutdownSignal,
) -> Result<(), ServerLifecycleError> {
    let lifecycle = Arc::clone(&state.lifecycle);
    let timeout_ms = state.config.shutdown_timeout_ms;
    let (signal_started_tx, signal_started_rx) = oneshot::channel();
    let shutdown = async move {
        signal.future.await;
        lifecycle.begin_draining();
        let _ = signal_started_tx.send(());
    };
    let mut server = Box::pin(
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .into_future(),
    );

    tokio::select! {
        result = &mut server => {
            if state.lifecycle.state() == LifecycleState::Draining {
                flush_runtime_store(&state)?;
                let timed_out = timeout_ms == 0;
                state.lifecycle.finish(timed_out || result.is_err());
                if timed_out {
                    return Err(ServerLifecycleError::ShutdownTimeout { timeout_ms });
                }
                return result.map_err(|error| ServerLifecycleError::ListenerFailed {
                    reason: error.to_string(),
                });
            }
            state.lifecycle.finish(result.is_err());
            return result.map_err(|error| ServerLifecycleError::ListenerFailed {
                reason: error.to_string(),
            });
        }
        result = signal_started_rx => result.map_err(|error| {
            ServerLifecycleError::SignalHandlerFailed {
                reason: error.to_string(),
            }
        })?,
    };

    let timed_out = timeout_ms == 0
        || tokio::time::timeout(Duration::from_millis(timeout_ms), &mut server)
            .await
            .is_err();
    drop(server);
    flush_runtime_store(&state)?;
    state.lifecycle.finish(timed_out);
    if timed_out {
        return Err(ServerLifecycleError::ShutdownTimeout { timeout_ms });
    }
    Ok(())
}

/// Serve HTTPS while preserving the same initialization, draining, timeout,
/// flush, and terminal-state contract as the plaintext listener.
pub async fn serve_tls_with_lifecycle(
    addr: SocketAddr,
    app: Router,
    state: AppState,
    signal: ShutdownSignal,
    tls: axum_server::tls_rustls::RustlsConfig,
) -> Result<(), ServerLifecycleError> {
    let lifecycle = Arc::clone(&state.lifecycle);
    let timeout_ms = state.config.shutdown_timeout_ms;
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    let (signal_started_tx, signal_started_rx) = oneshot::channel();
    let shutdown = async move {
        signal.future.await;
        lifecycle.begin_draining();
        shutdown_handle.graceful_shutdown(Some(Duration::from_millis(timeout_ms)));
        let _ = signal_started_tx.send(());
    };
    tokio::pin!(shutdown);
    let mut server = Box::pin(
        axum_server::bind_rustls(addr, tls)
            .handle(handle)
            .serve(app.into_make_service()),
    );

    tokio::select! {
        result = &mut server => {
            state.lifecycle.finish(result.is_err());
            return result.map_err(|error| ServerLifecycleError::ListenerFailed {
                reason: error.to_string(),
            });
        }
        () = &mut shutdown => {}
        result = signal_started_rx => result.map_err(|error| {
            ServerLifecycleError::SignalHandlerFailed {
                reason: error.to_string(),
            }
        })?,
    };

    let timed_out = timeout_ms == 0
        || tokio::time::timeout(Duration::from_millis(timeout_ms), &mut server)
            .await
            .is_err();
    drop(server);
    flush_runtime_store(&state)?;
    state.lifecycle.finish(timed_out);
    if timed_out {
        return Err(ServerLifecycleError::ShutdownTimeout { timeout_ms });
    }
    Ok(())
}

fn flush_runtime_store(state: &AppState) -> Result<(), ServerLifecycleError> {
    let RuntimeStoreProvider::Persistent(runtime) = &state.runtime_store else {
        return Ok(());
    };
    sync_tree(&runtime.root_path).map_err(|error| ServerLifecycleError::PersistentFlushFailed {
        path: runtime.root_path.display().to_string(),
        reason: error.to_string(),
    })
}

fn sync_tree(path: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            sync_tree(&entry_path)?;
        } else if entry_path.is_file() {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&entry_path)?
                .sync_all()?;
        }
    }
    fs::File::open(path)?.sync_all()
}

#[cfg(unix)]
fn termination_signal() -> std::io::Result<Pin<Box<dyn Future<Output = ()> + Send>>> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    Ok(Box::pin(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    }))
}

#[cfg(not(unix))]
fn termination_signal() -> std::io::Result<Pin<Box<dyn Future<Output = ()> + Send>>> {
    Ok(Box::pin(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}
