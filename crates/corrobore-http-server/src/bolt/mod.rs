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
//! The opt-in Bolt listener (epic #12, item #258).
//!
//! Module boundary: this module owns sockets, the version handshake and chunk
//! framing. Message meaning lives in [`session`], value encoding in
//! [`packstream`]. The listener is one more protocol adapter over the same
//! engine, budgets and bearer token the HTTP routes use; it grants nothing the
//! HTTP surface does not.
//!
//! Supported versions are Bolt 4.4 and 5.0 through 5.8. A client offering the
//! handshake-v2 manifest marker is answered from its other offers, which is the
//! fallback the specification prescribes for servers without manifest support.

pub mod packstream;
mod session;

use std::{fmt, io, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

use crate::{app::AppState, lifecycle::LifecycleState};
use packstream::{Value, decode, encode};
use session::Session;

/// Bolt handshake preamble.
pub const MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];
/// Versions this server speaks, highest first.
pub const SUPPORTED_VERSIONS: &[(u8, u8)] = &[
    (5, 8),
    (5, 7),
    (5, 6),
    (5, 5),
    (5, 4),
    (5, 3),
    (5, 2),
    (5, 1),
    (5, 0),
    (4, 4),
];
/// Largest message accepted from a client, after dechunking.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// How long a fresh connection has to send its handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the accept loop re-checks the lifecycle while idle.
const LIFECYCLE_POLL: Duration = Duration::from_millis(100);
/// The handshake-v2 manifest marker a newer driver may put in its first slot.
const MANIFEST_MARKER: u32 = 0x0000_01FF;

/// Why the listener stopped.
#[derive(Debug)]
pub enum BoltServerError {
    /// The socket could not be accepted from.
    Accept(io::Error),
}

impl fmt::Display for BoltServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept(error) => write!(formatter, "bolt listener accept failed: {error}"),
        }
    }
}

impl std::error::Error for BoltServerError {}

/// Serve Bolt on `listener` until the server lifecycle starts draining.
///
/// `tls` wraps every connection when the deployment enables TLS; the material
/// is the same the HTTPS listener uses. Connections beyond
/// `bolt_max_connections` wait for a slot rather than being refused, so a
/// driver pool at its limit sees latency, not errors.
///
/// # Errors
/// [`BoltServerError::Accept`] when the socket itself fails.
pub async fn serve_bolt(
    listener: TcpListener,
    state: AppState,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> Result<(), BoltServerError> {
    let state = Arc::new(state);
    let permits = Arc::new(Semaphore::new(state.config.bolt_max_connections));
    let acceptor = tls.map(TlsAcceptor::from);
    info!(
        max_connections = state.config.bolt_max_connections,
        tls = acceptor.is_some(),
        "bolt listener accepting connections"
    );
    loop {
        if draining(&state) {
            info!("bolt listener stopped accepting: server is draining");
            return Ok(());
        }
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted.map_err(BoltServerError::Accept)?;
                let permit = match Arc::clone(&permits).acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => return Ok(()),
                };
                let state = Arc::clone(&state);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    debug!(%peer, "bolt connection accepted");
                    let result = match acceptor {
                        Some(acceptor) => match acceptor.accept(stream).await {
                            Ok(tls_stream) => connection(tls_stream, state).await,
                            Err(error) => {
                                warn!(%peer, error = %error, "bolt tls handshake failed");
                                Ok(())
                            }
                        },
                        None => connection(stream, state).await,
                    };
                    if let Err(error) = result {
                        debug!(%peer, error = %error, "bolt connection ended with an error");
                    }
                });
            }
            () = tokio::time::sleep(LIFECYCLE_POLL) => {}
        }
    }
}

fn draining(state: &AppState) -> bool {
    !matches!(
        state.lifecycle.state(),
        LifecycleState::Initializing | LifecycleState::Ready
    )
}

/// Negotiate a version and then run the message loop until the client leaves,
/// a protocol violation closes the connection, or the socket fails.
async fn connection<S>(mut stream: S, state: Arc<AppState>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut handshake = [0u8; 20];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut handshake)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return Err(error),
        Err(_) => return Ok(()),
    }
    if handshake[..4] != MAGIC {
        // Not a Bolt client: no answer, so a port scanner learns nothing.
        return Ok(());
    }
    let offers = [
        u32::from_be_bytes([handshake[4], handshake[5], handshake[6], handshake[7]]),
        u32::from_be_bytes([handshake[8], handshake[9], handshake[10], handshake[11]]),
        u32::from_be_bytes([handshake[12], handshake[13], handshake[14], handshake[15]]),
        u32::from_be_bytes([handshake[16], handshake[17], handshake[18], handshake[19]]),
    ];
    let Some((major, minor)) = negotiate(&offers) else {
        stream.write_all(&[0, 0, 0, 0]).await?;
        stream.shutdown().await?;
        return Ok(());
    };
    stream.write_all(&[0, 0, minor, major]).await?;

    let mut session = Session::new(state, major, minor);
    loop {
        let Some(payload) = read_message(&mut stream).await? else {
            return Ok(());
        };
        let message = match decode(&payload) {
            Ok((value, consumed)) if consumed == payload.len() => value,
            Ok(_) | Err(_) => {
                // Malformed PackStream is a protocol violation: answer once,
                // then close.
                write_message(
                    &mut stream,
                    &Value::Structure {
                        tag: session::FAILURE,
                        fields: vec![Value::Dictionary(vec![
                            (
                                "code".to_owned(),
                                Value::String(session::code::REQUEST_INVALID.to_owned()),
                            ),
                            (
                                "message".to_owned(),
                                Value::String("the message is not valid PackStream".to_owned()),
                            ),
                        ])],
                    },
                )
                .await?;
                return stream.shutdown().await;
            }
        };
        let outcome = session.handle(message).await;
        for response in outcome.responses {
            write_message(
                &mut stream,
                &Value::Structure {
                    tag: response.tag,
                    fields: response.fields,
                },
            )
            .await?;
        }
        stream.flush().await?;
        if outcome.close {
            return stream.shutdown().await;
        }
    }
}

/// Pick the highest version this server speaks among the client's offers,
/// honouring the client's preference order between offers.
#[must_use]
pub fn negotiate(offers: &[u32; 4]) -> Option<(u8, u8)> {
    for offer in offers {
        if *offer == 0 || *offer == MANIFEST_MARKER {
            continue;
        }
        let major = (offer & 0xFF) as u8;
        let minor = ((offer >> 8) & 0xFF) as u8;
        let range = ((offer >> 16) & 0xFF) as u8;
        let lowest = minor.saturating_sub(range);
        if let Some(found) = SUPPORTED_VERSIONS
            .iter()
            .find(|(candidate_major, candidate_minor)| {
                *candidate_major == major && (lowest..=minor).contains(candidate_minor)
            })
        {
            return Some(*found);
        }
    }
    None
}

/// Read one chunked message. `None` when the client closed the connection
/// cleanly between messages.
async fn read_message<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Option<Vec<u8>>> {
    let mut payload = Vec::new();
    loop {
        let mut header = [0u8; 2];
        match stream.read_exact(&mut header).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof && payload.is_empty() => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
        let length = usize::from(u16::from_be_bytes(header));
        if length == 0 {
            if payload.is_empty() {
                // A no-op chunk keeps the connection alive between messages.
                continue;
            }
            return Ok(Some(payload));
        }
        if payload.len() + length > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bolt message exceeds the {MAX_MESSAGE_BYTES}-byte maximum"),
            ));
        }
        let start = payload.len();
        payload.resize(start + length, 0);
        stream.read_exact(&mut payload[start..]).await?;
    }
}

async fn write_message<S: AsyncWrite + Unpin>(stream: &mut S, message: &Value) -> io::Result<()> {
    let mut payload = Vec::new();
    encode(message, &mut payload);
    let mut framed = Vec::with_capacity(payload.len() + 4 + payload.len() / 0xFFFF * 2);
    for chunk in payload.chunks(0xFFFF) {
        framed.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        framed.extend_from_slice(chunk);
    }
    framed.extend_from_slice(&[0, 0]);
    stream.write_all(&framed).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn version(major: u8, minor: u8, range: u8) -> u32 {
        ((range as u32) << 16) | ((minor as u32) << 8) | (major as u32)
    }

    #[test]
    fn negotiation_prefers_the_client_order_then_the_highest_minor() {
        assert_eq!(negotiate(&[version(5, 8, 8), 0, 0, 0]), Some((5, 8)));
        assert_eq!(
            negotiate(&[version(5, 9, 1), version(4, 4, 0), 0, 0]),
            Some((5, 8))
        );
        assert_eq!(
            negotiate(&[version(4, 4, 0), version(5, 4, 4), 0, 0]),
            Some((4, 4))
        );
        assert_eq!(
            negotiate(&[MANIFEST_MARKER, version(5, 2, 2), 0, 0]),
            Some((5, 2))
        );
        assert_eq!(negotiate(&[version(4, 3, 3), version(3, 0, 0), 0, 0]), None);
        assert_eq!(negotiate(&[0, 0, 0, 0]), None);
    }
}
