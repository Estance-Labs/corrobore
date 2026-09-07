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
//! PostgreSQL wire protocol version 3 message codec.
//!
//! Module boundary: bytes in, bytes out. This module frames messages, builds
//! backend messages and reads the fields of frontend messages. It decides
//! nothing about SQL, authentication or state; the session does.

use std::io;

use sql_frontend::ColumnType;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest frontend message accepted.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

const PROTOCOL_3: i32 = 196_608;
const CANCEL_REQUEST: i32 = 80_877_102;
const SSL_REQUEST: i32 = 80_877_103;
const GSSENC_REQUEST: i32 = 80_877_104;

/// What a fresh connection sent first.
pub(super) enum Startup {
    /// `SSLRequest`.
    Ssl,
    /// `GSSENCRequest`.
    GssEncryption,
    /// `CancelRequest`; carries nothing this server acts on.
    Cancel,
    /// `StartupMessage` with its parameters.
    Start(Vec<(String, String)>),
}

/// Read the untyped startup packet.
pub(super) async fn read_startup<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> io::Result<Option<Startup>> {
    let mut length = [0u8; 4];
    match stream.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = i32::from_be_bytes(length);
    if !(8..=10_000).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "startup packet length out of range",
        ));
    }
    let mut body = vec![0u8; length as usize - 4];
    stream.read_exact(&mut body).await?;
    let mut cursor = Cursor::new(&body);
    let code = cursor.i32()?;
    Ok(Some(match code {
        SSL_REQUEST => Startup::Ssl,
        GSSENC_REQUEST => Startup::GssEncryption,
        CANCEL_REQUEST => Startup::Cancel,
        PROTOCOL_3 => {
            let mut parameters = Vec::new();
            loop {
                let key = cursor.cstring()?;
                if key.is_empty() {
                    break;
                }
                let value = cursor.cstring()?;
                parameters.push((key, value));
            }
            Startup::Start(parameters)
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported protocol version {other:#x}"),
            ));
        }
    }))
}

/// Read one typed frontend message; `None` when the client closed cleanly.
pub(super) async fn read_message<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut tag = [0u8; 1];
    match stream.read_exact(&mut tag).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).await?;
    let length = i32::from_be_bytes(length);
    if length < 4 || length as usize - 4 > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message length out of range",
        ));
    }
    let mut body = vec![0u8; length as usize - 4];
    stream.read_exact(&mut body).await?;
    Ok(Some((tag[0], body)))
}

/// A backend message ready to frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Backend {
    pub tag: u8,
    pub body: Vec<u8>,
}

impl Backend {
    fn new(tag: u8, body: Vec<u8>) -> Self {
        Self { tag, body }
    }

    pub fn authentication_cleartext() -> Self {
        Self::new(b'R', 3i32.to_be_bytes().to_vec())
    }

    pub fn authentication_ok() -> Self {
        Self::new(b'R', 0i32.to_be_bytes().to_vec())
    }

    pub fn parameter_status(key: &str, value: &str) -> Self {
        let mut body = Vec::new();
        push_cstring(&mut body, key);
        push_cstring(&mut body, value);
        Self::new(b'S', body)
    }

    pub fn backend_key_data(process_id: i32, secret_key: i32) -> Self {
        let mut body = process_id.to_be_bytes().to_vec();
        body.extend_from_slice(&secret_key.to_be_bytes());
        Self::new(b'K', body)
    }

    pub fn ready_for_query(status: u8) -> Self {
        Self::new(b'Z', vec![status])
    }

    pub fn row_description(columns: &[(String, ColumnType)]) -> Self {
        let mut body = (columns.len() as i16).to_be_bytes().to_vec();
        for (name, column_type) in columns {
            push_cstring(&mut body, name);
            body.extend_from_slice(&0i32.to_be_bytes()); // table oid
            body.extend_from_slice(&0i16.to_be_bytes()); // attribute number
            body.extend_from_slice(&column_type.oid().to_be_bytes());
            body.extend_from_slice(&column_type.length().to_be_bytes());
            body.extend_from_slice(&(-1i32).to_be_bytes()); // type modifier
            body.extend_from_slice(&0i16.to_be_bytes()); // text format
        }
        Self::new(b'T', body)
    }

    pub fn data_row(cells: &[Option<String>]) -> Self {
        let mut body = (cells.len() as i16).to_be_bytes().to_vec();
        for cell in cells {
            match cell {
                None => body.extend_from_slice(&(-1i32).to_be_bytes()),
                Some(text) => {
                    body.extend_from_slice(&(text.len() as i32).to_be_bytes());
                    body.extend_from_slice(text.as_bytes());
                }
            }
        }
        Self::new(b'D', body)
    }

    pub fn command_complete(tag: &str) -> Self {
        let mut body = Vec::new();
        push_cstring(&mut body, tag);
        Self::new(b'C', body)
    }

    pub fn empty_query() -> Self {
        Self::new(b'I', vec![])
    }

    pub fn parse_complete() -> Self {
        Self::new(b'1', vec![])
    }

    pub fn bind_complete() -> Self {
        Self::new(b'2', vec![])
    }

    pub fn close_complete() -> Self {
        Self::new(b'3', vec![])
    }

    pub fn no_data() -> Self {
        Self::new(b'n', vec![])
    }

    pub fn portal_suspended() -> Self {
        Self::new(b's', vec![])
    }

    pub fn parameter_description(oids: &[i32]) -> Self {
        let mut body = (oids.len() as i16).to_be_bytes().to_vec();
        for oid in oids {
            body.extend_from_slice(&oid.to_be_bytes());
        }
        Self::new(b't', body)
    }

    pub fn error(severity: &str, sqlstate: &str, message: &str) -> Self {
        Self::new(b'E', diagnostic_body(severity, sqlstate, message))
    }

    pub fn notice(severity: &str, sqlstate: &str, message: &str) -> Self {
        Self::new(b'N', diagnostic_body(severity, sqlstate, message))
    }
}

fn diagnostic_body(severity: &str, sqlstate: &str, message: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(b'S');
    push_cstring(&mut body, severity);
    body.push(b'V');
    push_cstring(&mut body, severity);
    body.push(b'C');
    push_cstring(&mut body, sqlstate);
    body.push(b'M');
    push_cstring(&mut body, message);
    body.push(0);
    body
}

fn push_cstring(body: &mut Vec<u8>, text: &str) {
    body.extend_from_slice(text.as_bytes());
    body.push(0);
}

/// Frame and write messages, then flush.
pub(super) async fn write<S: AsyncWrite + Unpin>(
    stream: &mut S,
    messages: &[Backend],
) -> io::Result<()> {
    let mut frame = Vec::new();
    for message in messages {
        frame.push(message.tag);
        frame.extend_from_slice(&((message.body.len() + 4) as i32).to_be_bytes());
        frame.extend_from_slice(&message.body);
    }
    stream.write_all(&frame).await?;
    stream.flush().await
}

/// A bounds-checked reader over a message body.
pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn truncated() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "message body is truncated")
    }

    pub fn byte(&mut self) -> io::Result<u8> {
        let byte = *self.bytes.get(self.position).ok_or_else(Self::truncated)?;
        self.position += 1;
        Ok(byte)
    }

    pub fn i16(&mut self) -> io::Result<i16> {
        let slice = self.take(2)?;
        Ok(i16::from_be_bytes([slice[0], slice[1]]))
    }

    pub fn i32(&mut self) -> io::Result<i32> {
        let slice = self.take(4)?;
        Ok(i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(Self::truncated)?;
        if end > self.bytes.len() {
            return Err(Self::truncated());
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    pub fn cstring(&mut self) -> io::Result<String> {
        let rest = &self.bytes[self.position..];
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(Self::truncated)?;
        let text = String::from_utf8(rest[..end].to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message text is not UTF-8"))?;
        self.position += end + 1;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn row_description_and_data_row_follow_the_wire_layout() {
        let description = Backend::row_description(&[("k".to_owned(), ColumnType::Int8)]);
        assert_eq!(description.tag, b'T');
        assert_eq!(&description.body[..2], &1i16.to_be_bytes());
        assert_eq!(&description.body[2..4], b"k\0");
        assert_eq!(&description.body[10..14], &20i32.to_be_bytes());

        let row = Backend::data_row(&[Some("42".to_owned()), None]);
        assert_eq!(
            row.body,
            [0, 2, 0, 0, 0, 2, b'4', b'2', 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn diagnostics_carry_severity_sqlstate_and_message() {
        let error = Backend::error("ERROR", "42601", "bad");
        let text = String::from_utf8_lossy(&error.body).to_string();
        assert!(text.contains("SERROR\0"));
        assert!(text.contains("C42601\0"));
        assert!(text.contains("Mbad\0"));
        assert_eq!(*error.body.last().unwrap(), 0);
    }

    #[test]
    fn cursor_reads_fields_and_rejects_truncation() {
        let body = [0, 7, b'a', b'b', 0, 0, 0, 0, 1];
        let mut cursor = Cursor::new(&body);
        assert_eq!(cursor.i16().unwrap(), 7);
        assert_eq!(cursor.cstring().unwrap(), "ab");
        assert_eq!(cursor.i32().unwrap(), 1);
        assert!(cursor.byte().is_err());
    }
}
