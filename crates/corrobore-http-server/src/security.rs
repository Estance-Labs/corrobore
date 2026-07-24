// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Standalone-server network security contracts.
//!
//! Authentication policy belongs to the transport adapter rather than the
//! engine. TLS material is loaded once during startup so process restart is the
//! explicit, auditable rotation boundary.

use std::{fs, path::PathBuf, sync::Arc};

use axum_server::tls_rustls::RustlsConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use thiserror::Error;
use x509_parser::parse_x509_certificate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticationMode {
    Required,
    LocalInsecure,
}

impl AuthenticationMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::LocalInsecure => "local-insecure",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationalEndpointPolicy {
    Public,
    Authenticated,
}

impl OperationalEndpointPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Authenticated => "authenticated",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretSource {
    Inline,
    File,
}

impl SecretSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::File => "file",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsMaterialPaths {
    pub certificate_file: PathBuf,
    pub private_key_file: PathBuf,
}

#[derive(Debug, Error)]
pub enum TlsMaterialError {
    #[error("tls.certificate_file: cannot read or parse configured certificate")]
    InvalidCertificate,
    #[error("tls.private_key_file: cannot read or parse configured private key")]
    InvalidPrivateKey,
    #[error("tls.private_key_file: private key does not match the configured certificate")]
    KeyMismatch,
    #[error("tls.certificate_file: certificate is expired or not yet valid")]
    CertificateExpired,
}

/// Load, validate, and assemble the TLS listener configuration without ever
/// including certificate or private-key contents in diagnostics.
pub async fn load_tls_material(paths: &TlsMaterialPaths) -> Result<RustlsConfig, TlsMaterialError> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certificate_pem =
        fs::read(&paths.certificate_file).map_err(|_| TlsMaterialError::InvalidCertificate)?;
    let certificates = CertificateDer::pem_slice_iter(&certificate_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsMaterialError::InvalidCertificate)?;
    let Some(leaf) = certificates.first() else {
        return Err(TlsMaterialError::InvalidCertificate);
    };
    let (_, certificate) =
        parse_x509_certificate(leaf.as_ref()).map_err(|_| TlsMaterialError::InvalidCertificate)?;
    if !certificate.validity().is_valid() {
        return Err(TlsMaterialError::CertificateExpired);
    }

    let private_key_pem =
        fs::read(&paths.private_key_file).map_err(|_| TlsMaterialError::InvalidPrivateKey)?;
    let private_key = PrivateKeyDer::from_pem_slice(&private_key_pem)
        .map_err(|_| TlsMaterialError::InvalidPrivateKey)?;
    let server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|_| TlsMaterialError::KeyMismatch)?;
    Ok(RustlsConfig::from_config(Arc::new(server)))
}
