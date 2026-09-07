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
//! The opt-in PostgreSQL wire-protocol listener (epic #90, item #259).
//!
//! Module boundary: this module owns sockets, the startup handshake (including
//! `SSLRequest`) and the connection loop. Message meaning lives in
//! [`session`], framing in [`wire`], and SQL compilation in the `sql-frontend`
//! crate. The listener is one more adapter over the same engine, budgets and
//! bearer token the HTTP routes use; a SQL client is authenticated with that
//! token as its password.

mod session;
mod wire;

use std::{fmt, io, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use crate::{app::AppState, lifecycle::LifecycleState};
use session::Session;
use wire::{Backend, Startup};

/// How long a fresh connection has to complete startup and authentication.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the accept loop re-checks the lifecycle while idle.
const LIFECYCLE_POLL: Duration = Duration::from_millis(100);

/// Why the listener stopped.
#[derive(Debug)]
pub enum SqlServerError {
    /// The socket could not be accepted from.
    Accept(io::Error),
}

impl fmt::Display for SqlServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept(error) => write!(formatter, "sql listener accept failed: {error}"),
        }
    }
}

impl std::error::Error for SqlServerError {}

/// Serve the PostgreSQL wire protocol on `listener` until the lifecycle drains.
///
/// With `tls`, an `SSLRequest` is answered with `S` and the connection is
/// upgraded before startup; without it the request is declined with `N` and a
/// client that insists on TLS disconnects on its own.
///
/// # Errors
/// [`SqlServerError::Accept`] when the socket itself fails.
pub async fn serve_sql(
    listener: TcpListener,
    state: AppState,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> Result<(), SqlServerError> {
    let state = Arc::new(state);
    let permits = Arc::new(Semaphore::new(state.config.sql_max_connections));
    let acceptor = tls.map(TlsAcceptor::from);
    info!(
        max_connections = state.config.sql_max_connections,
        tls = acceptor.is_some(),
        "sql listener accepting connections"
    );
    loop {
        if !matches!(
            state.lifecycle.state(),
            LifecycleState::Initializing | LifecycleState::Ready
        ) {
            info!("sql listener stopped accepting: server is draining");
            return Ok(());
        }
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted.map_err(SqlServerError::Accept)?;
                let permit = match Arc::clone(&permits).acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => return Ok(()),
                };
                let state = Arc::clone(&state);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    debug!(%peer, "sql connection accepted");
                    if let Err(error) = connection(stream, state, acceptor).await {
                        debug!(%peer, error = %error, "sql connection ended with an error");
                    }
                });
            }
            () = tokio::time::sleep(LIFECYCLE_POLL) => {}
        }
    }
}

/// Startup, optional TLS upgrade, authentication, then the message loop.
async fn connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<AppState>,
    acceptor: Option<TlsAcceptor>,
) -> io::Result<()> {
    let startup = tokio::time::timeout(STARTUP_TIMEOUT, wire::read_startup(&mut stream)).await;
    let mut startup = match startup {
        Ok(Ok(Some(startup))) => startup,
        Ok(Ok(None)) | Err(_) => return Ok(()),
        Ok(Err(error)) => return Err(error),
    };
    if let Startup::Ssl = startup {
        match acceptor {
            Some(acceptor) => {
                stream.write_all(b"S").await?;
                let tls_stream = match acceptor.accept(stream).await {
                    Ok(tls_stream) => tls_stream,
                    Err(error) => {
                        warn!(error = %error, "sql tls handshake failed");
                        return Ok(());
                    }
                };
                return after_encryption(tls_stream, state).await;
            }
            None => {
                stream.write_all(b"N").await?;
                startup =
                    match tokio::time::timeout(STARTUP_TIMEOUT, wire::read_startup(&mut stream))
                        .await
                    {
                        Ok(Ok(Some(startup))) => startup,
                        Ok(Ok(None)) | Err(_) => return Ok(()),
                        Ok(Err(error)) => return Err(error),
                    };
            }
        }
    }
    if let Startup::GssEncryption = startup {
        stream.write_all(b"N").await?;
        startup = match tokio::time::timeout(STARTUP_TIMEOUT, wire::read_startup(&mut stream)).await
        {
            Ok(Ok(Some(startup))) => startup,
            Ok(Ok(None)) | Err(_) => return Ok(()),
            Ok(Err(error)) => return Err(error),
        };
    }
    run(stream, state, startup).await
}

/// After a TLS upgrade the client sends its startup packet again, in clear
/// inside the tunnel.
async fn after_encryption<S>(mut stream: S, state: Arc<AppState>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let startup = match tokio::time::timeout(STARTUP_TIMEOUT, wire::read_startup(&mut stream)).await
    {
        Ok(Ok(Some(startup))) => startup,
        Ok(Ok(None)) | Err(_) => return Ok(()),
        Ok(Err(error)) => return Err(error),
    };
    run(stream, state, startup).await
}

async fn run<S>(mut stream: S, state: Arc<AppState>, startup: Startup) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let parameters = match startup {
        Startup::Start(parameters) => parameters,
        Startup::Cancel => return Ok(()),
        Startup::Ssl | Startup::GssEncryption => {
            // A second encryption request is a protocol violation.
            return Ok(());
        }
    };
    let user = parameters
        .iter()
        .find(|(key, _)| key == "user")
        .map(|(_, value)| value.as_str())
        .unwrap_or("");
    debug!(user, "sql startup");

    let mut session = Session::new(state);
    if session.needs_password() {
        wire::write(&mut stream, &[Backend::authentication_cleartext()]).await?;
        let password =
            match tokio::time::timeout(STARTUP_TIMEOUT, wire::read_message(&mut stream)).await {
                Ok(Ok(Some((b'p', body)))) => wire::Cursor::new(&body).cstring()?,
                Ok(Ok(Some(_))) | Ok(Ok(None)) | Err(_) => return Ok(()),
                Ok(Err(error)) => return Err(error),
            };
        if !session.check_password(&password) {
            warn!(user, "sql authentication refused");
            wire::write(&mut stream, &[Session::invalid_password()]).await?;
            return stream.shutdown().await;
        }
    }
    let mut welcome = vec![Backend::authentication_ok()];
    welcome.extend(session.welcome());
    wire::write(&mut stream, &welcome).await?;

    loop {
        let Some((tag, body)) = wire::read_message(&mut stream).await? else {
            return Ok(());
        };
        let (messages, close) = session.handle(tag, body).await;
        if !messages.is_empty() {
            wire::write(&mut stream, &messages).await?;
        }
        if close {
            return stream.shutdown().await;
        }
    }
}
