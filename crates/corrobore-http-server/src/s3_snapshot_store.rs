// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Minimal path-style AWS Signature V4 snapshot provider for S3 and MinIO.

use chrono::Utc;
use graph_storage::SnapshotArtifactStore;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Method, Url, blocking::Client};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Configuration for one S3-compatible snapshot bucket.
#[derive(Clone)]
pub struct S3SnapshotStoreConfig {
    /// HTTP(S) S3 or MinIO endpoint, without a bucket suffix.
    pub endpoint: String,
    /// Destination bucket.
    pub bucket: String,
    /// AWS signing region (`us-east-1` is conventional for MinIO).
    pub region: String,
    /// Access-key identifier.
    pub access_key: String,
    /// Secret signing key. This value is never logged or serialized.
    pub secret_key: String,
    /// Optional temporary-session token.
    pub session_token: Option<String>,
}

/// Synchronous S3/MinIO implementation used by offline snapshot export commands.
pub struct S3SnapshotArtifactStore {
    client: Client,
    config: S3SnapshotStoreConfig,
}

impl S3SnapshotArtifactStore {
    /// Build a provider after validating endpoint, bucket, region and credentials.
    pub fn new(config: S3SnapshotStoreConfig) -> Result<Self, String> {
        let endpoint = Url::parse(config.endpoint.trim_end_matches('/'))
            .map_err(|error| format!("invalid S3 endpoint: {error}"))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || config.bucket.trim().is_empty()
            || config.region.trim().is_empty()
            || config.access_key.trim().is_empty()
            || config.secret_key.is_empty()
        {
            return Err("S3 endpoint, bucket, region and credentials are required".to_owned());
        }
        Ok(Self {
            client: Client::builder()
                .build()
                .map_err(|error| format!("failed to initialize S3 client: {error}"))?,
            config,
        })
    }

    fn execute(
        &self,
        method: Method,
        key: &str,
        query: &str,
        body: &[u8],
    ) -> Result<Vec<u8>, String> {
        let endpoint = self.config.endpoint.trim_end_matches('/');
        let canonical_uri = format!(
            "/{}/{}",
            aws_encode(&self.config.bucket, false),
            aws_encode(key.trim_start_matches('/'), true)
        );
        let url = if query.is_empty() {
            format!("{endpoint}{canonical_uri}")
        } else {
            format!("{endpoint}{canonical_uri}?{query}")
        };
        let parsed =
            Url::parse(&url).map_err(|error| format!("invalid S3 request URL: {error}"))?;
        let host = match parsed.port() {
            Some(port) => format!("{}:{port}", parsed.host_str().unwrap_or_default()),
            None => parsed.host_str().unwrap_or_default().to_owned(),
        };
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let short_date = now.format("%Y%m%d").to_string();
        let payload_hash = sha256_hex(body);
        let mut canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let mut signed_headers = "host;x-amz-content-sha256;x-amz-date".to_owned();
        if let Some(token) = &self.config.session_token {
            canonical_headers.push_str(&format!("x-amz-security-token:{}\n", token.trim()));
            signed_headers.push_str(";x-amz-security-token");
        }
        let canonical_request = format!(
            "{}\n{canonical_uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            method.as_str()
        );
        let scope = format!("{short_date}/{}/s3/aws4_request", self.config.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signature =
            hex(
                &signing_key(&self.config.secret_key, &short_date, &self.config.region)
                    .and_then(|key| hmac(&key, string_to_sign.as_bytes()))?,
            );
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.config.access_key
        );
        let mut request = self
            .client
            .request(method, parsed)
            .header("host", host)
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", amz_date)
            .header("authorization", authorization);
        if let Some(token) = &self.config.session_token {
            request = request.header("x-amz-security-token", token.trim());
        }
        if !body.is_empty() {
            request = request.body(body.to_vec());
        }
        let response = request
            .send()
            .map_err(|error| format!("S3 request failed: {error}"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|error| format!("failed to read S3 response: {error}"))?;
        if !status.is_success() {
            return Err(format!("S3 request returned HTTP {status}"));
        }
        Ok(bytes.to_vec())
    }
}

impl SnapshotArtifactStore for S3SnapshotArtifactStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.execute(Method::PUT, key, "", bytes).map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        self.execute(Method::GET, key, "", &[])
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        let query = format!(
            "list-type=2&prefix={}",
            aws_encode(prefix.trim_start_matches('/'), false)
        );
        let body = self.execute(Method::GET, "", &query, &[])?;
        let body =
            String::from_utf8(body).map_err(|_| "S3 list response is not UTF-8".to_owned())?;
        let mut keys = Vec::new();
        let mut remaining = body.as_str();
        while let Some(start) = remaining.find("<Key>") {
            let after = &remaining[start + 5..];
            let Some(end) = after.find("</Key>") else {
                return Err("S3 list response contains an incomplete Key element".to_owned());
            };
            keys.push(xml_unescape(&after[..end]));
            remaining = &after[end + 6..];
        }
        Ok(keys)
    }
}

fn signing_key(secret: &str, date: &str, region: &str) -> Result<Vec<u8>, String> {
    let date_key = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let region_key = hmac(&date_key, region.as_bytes())?;
    let service_key = hmac(&region_key, b"s3")?;
    hmac(&service_key, b"aws4_request")
}

fn hmac(key: &[u8], value: &[u8]) -> Result<Vec<u8>, String> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|_| "failed to initialize S3 signing key".to_owned())?;
    mac.update(value);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

fn aws_encode(value: &str, preserve_slash: bool) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (preserve_slash && byte == b'/')
        {
            output.push(char::from(byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_encoding_and_signing_are_deterministic() {
        assert_eq!(aws_encode("folder/a b.json", true), "folder/a%20b.json");
        assert_eq!(
            hex(&signing_key("secret", "20260726", "us-east-1").expect("signing key")),
            "ceb37f7a04e12d2e2ec04ce54ee8aef8a63a245875c781aac8110770b841a574"
        );
    }
}
