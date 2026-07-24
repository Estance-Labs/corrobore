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
use std::{collections::HashMap, env, fmt, fs, net::IpAddr};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::security::{AuthenticationMode, OperationalEndpointPolicy, SecretSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageMode {
    Ephemeral,
    Persistent,
}

impl StorageMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::Persistent => "persistent",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub auth_mode: AuthenticationMode,
    pub auth_token: Option<String>,
    pub auth_token_source: Option<SecretSource>,
    pub admin_auth_token: Option<String>,
    pub admin_auth_token_source: Option<SecretSource>,
    pub operational_endpoint_policy: OperationalEndpointPolicy,
    pub session_store_dir: String,
    pub log_dir: String,
    pub request_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub session_idle_ttl_ms: u64,
    /// Maximum request body size (bytes) for standard JSON routes (2.3).
    pub max_body_bytes: usize,
    /// Maximum request body size (bytes) for STIX import routes (2.3).
    pub import_max_body_bytes: usize,
    /// Sustained request rate per second for the global rate limiter (2.3).
    pub rate_limit_per_second: u64,
    /// Burst allowance for the global rate limiter (2.3).
    pub rate_limit_burst: u32,
    /// Optional directory containing the production explorer build.
    pub web_dir: Option<String>,
    /// Licensed enterprise modules enabled for this runtime instance.
    pub licensed_modules: Vec<String>,
    /// Optional validated client UUID extracted from a signed license PEM.
    pub license_client_uuid: Option<String>,
    /// Optional validated client email extracted from a signed license PEM.
    pub license_client_email: Option<String>,
    /// Optional validated RFC3339 expiration timestamp extracted from a signed license PEM.
    pub license_valid_until: Option<String>,
    /// Optional marker indicating the signed license is tagged as NFR.
    pub license_is_nfr: Option<bool>,
    /// Runtime graph storage mode.
    pub storage_mode: StorageMode,
    /// Graph storage directory configured for persistent mode.
    pub storage_dir: Option<String>,
    /// Durability control: require fsync for persistent graph mutation writes.
    pub storage_require_fsync: bool,
    /// Durability control: enforce strict recovery checks in persistent mode.
    pub storage_strict_recovery: bool,
    /// Trusted root containing enterprise domain provider libraries.
    pub domain_provider_dir: Option<String>,
    /// Deployment manifest describing required provider libraries and hashes.
    pub domain_provider_manifest_file: Option<String>,
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth_mode", &self.auth_mode.as_str())
            .field("auth_token", &"<redacted>")
            .field(
                "auth_token_source",
                &self.auth_token_source.map(SecretSource::as_str),
            )
            .field(
                "admin_auth_token",
                &self.admin_auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "admin_auth_token_source",
                &self.admin_auth_token_source.map(SecretSource::as_str),
            )
            .field(
                "operational_endpoint_policy",
                &self.operational_endpoint_policy.as_str(),
            )
            .field("session_store_dir", &self.session_store_dir)
            .field("log_dir", &self.log_dir)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("shutdown_timeout_ms", &self.shutdown_timeout_ms)
            .field("session_idle_ttl_ms", &self.session_idle_ttl_ms)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("import_max_body_bytes", &self.import_max_body_bytes)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field("web_dir", &self.web_dir)
            .field("licensed_modules", &self.licensed_modules)
            .field("license_client_uuid", &self.license_client_uuid)
            .field("license_client_email", &self.license_client_email)
            .field("license_valid_until", &self.license_valid_until)
            .field("license_is_nfr", &self.license_is_nfr)
            .field("storage_mode", &self.storage_mode)
            .field("storage_dir", &self.storage_dir)
            .field("storage_require_fsync", &self.storage_require_fsync)
            .field("storage_strict_recovery", &self.storage_strict_recovery)
            .field("domain_provider_dir", &self.domain_provider_dir)
            .field(
                "domain_provider_manifest_file",
                &self.domain_provider_manifest_file,
            )
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingEnv(&'static str),
    #[error("invalid environment variable {name}: {value}")]
    InvalidEnv { name: &'static str, value: String },
    #[error("{field}: inline and file secret sources are mutually exclusive")]
    SecretSourceConflict { field: &'static str },
    #[error("{field}: cannot read configured secret file")]
    SecretFileUnreadable { field: &'static str },
    #[error("{field}: configured secret must not be empty")]
    InvalidSecret { field: &'static str },
}

fn parse_auth_mode(vars: &HashMap<String, String>) -> Result<AuthenticationMode, ConfigError> {
    match vars
        .get("CORROBORE_HTTP_AUTH_MODE")
        .map_or("required", String::as_str)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "required" => Ok(AuthenticationMode::Required),
        "local-insecure" => Ok(AuthenticationMode::LocalInsecure),
        value => Err(ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_AUTH_MODE",
            value: value.to_owned(),
        }),
    }
}

fn parse_operational_endpoint_policy(
    vars: &HashMap<String, String>,
) -> Result<OperationalEndpointPolicy, ConfigError> {
    match vars
        .get("CORROBORE_OPERATIONAL_ENDPOINT_POLICY")
        .map_or("public", String::as_str)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "public" => Ok(OperationalEndpointPolicy::Public),
        "authenticated" => Ok(OperationalEndpointPolicy::Authenticated),
        value => Err(ConfigError::InvalidEnv {
            name: "CORROBORE_OPERATIONAL_ENDPOINT_POLICY",
            value: value.to_owned(),
        }),
    }
}

fn resolve_secret(
    vars: &HashMap<String, String>,
    inline_name: &'static str,
    file_name: &'static str,
    field: &'static str,
    file_field: &'static str,
) -> Result<(Option<String>, Option<SecretSource>), ConfigError> {
    let inline = vars.get(inline_name);
    let file = vars.get(file_name);
    match (inline, file) {
        (Some(_), Some(_)) => Err(ConfigError::SecretSourceConflict { field }),
        (Some(value), None) => {
            let secret = value.trim().to_owned();
            if secret.is_empty() {
                Err(ConfigError::InvalidSecret { field })
            } else {
                Ok((Some(secret), Some(SecretSource::Inline)))
            }
        }
        (None, Some(path)) => {
            let secret = fs::read_to_string(path)
                .map_err(|_| ConfigError::SecretFileUnreadable { field: file_field })?
                .trim()
                .to_owned();
            if secret.is_empty() {
                Err(ConfigError::InvalidSecret { field })
            } else {
                Ok((Some(secret), Some(SecretSource::File)))
            }
        }
        (None, None) => Ok((None, None)),
    }
}

impl ServerConfig {
    /// Validate the bind address against the resolved transport and endpoint
    /// policies before a listener is opened.
    pub fn validate_network_exposure(&self, tls_enabled: bool) -> Result<(), &'static str> {
        let Ok(host) = self.host.parse::<IpAddr>() else {
            return Err("server.host: expected an IP address");
        };
        if self.auth_mode == AuthenticationMode::LocalInsecure && !host.is_loopback() {
            return Err("server.host: local-insecure authentication mode is limited to loopback");
        }
        if !host.is_loopback() && !tls_enabled {
            return Err("tls.enabled: TLS is required for non-loopback exposure");
        }
        if !host.is_loopback()
            && self.operational_endpoint_policy == OperationalEndpointPolicy::Public
        {
            return Err(
                "operations.endpoint_policy: authenticated is required for non-loopback exposure",
            );
        }
        Ok(())
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let mut vars = HashMap::new();
        for (key, value) in env::vars() {
            vars.insert(key, value);
        }
        Self::from_map(&vars)
    }

    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let host = vars
            .get("CORROBORE_HTTP_HOST")
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".to_owned());

        let port = parse_u16(
            "CORROBORE_HTTP_PORT",
            vars.get("CORROBORE_HTTP_PORT")
                .map(String::as_str)
                .unwrap_or("8080"),
        )?;

        let auth_mode = parse_auth_mode(vars)?;
        let (auth_token, auth_token_source) = resolve_secret(
            vars,
            "CORROBORE_HTTP_AUTH_TOKEN",
            "CORROBORE_HTTP_AUTH_TOKEN_FILE",
            "server.auth_token",
            "server.auth_token_file",
        )?;
        if auth_mode == AuthenticationMode::Required && auth_token.is_none() {
            return Err(ConfigError::MissingEnv("CORROBORE_HTTP_AUTH_TOKEN"));
        }
        let (admin_auth_token, admin_auth_token_source) = resolve_secret(
            vars,
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE",
            "server.admin_auth_token",
            "server.admin_auth_token_file",
        )?;
        let operational_endpoint_policy = parse_operational_endpoint_policy(vars)?;

        let session_store_dir = vars
            .get("CORROBORE_HTTP_SESSION_STORE_DIR")
            .cloned()
            .unwrap_or_else(|| ".corrobore-runtime".to_owned());

        let log_dir = vars
            .get("CORROBORE_HTTP_LOG_DIR")
            .cloned()
            .unwrap_or_else(|| format!("{session_store_dir}/logs"));

        let request_timeout_ms = parse_u64(
            "CORROBORE_HTTP_REQUEST_TIMEOUT_MS",
            vars.get("CORROBORE_HTTP_REQUEST_TIMEOUT_MS")
                .map(String::as_str)
                .unwrap_or("30000"),
        )?;

        let shutdown_timeout_ms = parse_u64(
            "CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS",
            vars.get("CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS")
                .map(String::as_str)
                .unwrap_or("5000"),
        )?;

        let session_idle_ttl_ms = parse_u64(
            "CORROBORE_HTTP_SESSION_IDLE_TTL_MS",
            vars.get("CORROBORE_HTTP_SESSION_IDLE_TTL_MS")
                .map(String::as_str)
                .unwrap_or("0"),
        )?;

        let max_body_bytes = parse_usize(
            "CORROBORE_HTTP_MAX_BODY_BYTES",
            vars.get("CORROBORE_HTTP_MAX_BODY_BYTES")
                .map(String::as_str)
                .unwrap_or("2097152"),
        )?;

        let import_max_body_bytes = parse_usize(
            "CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES",
            vars.get("CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES")
                .map(String::as_str)
                .unwrap_or("33554432"),
        )?;

        let rate_limit_per_second = parse_u64(
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND",
            vars.get("CORROBORE_HTTP_RATE_LIMIT_PER_SECOND")
                .map(String::as_str)
                .unwrap_or("50"),
        )?;

        let rate_limit_burst = parse_u32(
            "CORROBORE_HTTP_RATE_LIMIT_BURST",
            vars.get("CORROBORE_HTTP_RATE_LIMIT_BURST")
                .map(String::as_str)
                .unwrap_or("200"),
        )?;

        let web_dir = vars
            .get("CORROBORE_HTTP_WEB_DIR")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let storage_mode = Self::parse_storage_mode(
            vars.get("CORROBORE_STORAGE_MODE")
                .map(String::as_str)
                .unwrap_or("ephemeral"),
        )?;
        let storage_dir = Self::parse_storage_dir(vars, storage_mode)?;
        let storage_require_fsync = Self::parse_storage_bool(
            vars,
            "CORROBORE_STORAGE_REQUIRE_FSYNC",
            matches!(storage_mode, StorageMode::Persistent),
        )?;
        let storage_strict_recovery = Self::parse_storage_bool(
            vars,
            "CORROBORE_STORAGE_STRICT_RECOVERY",
            matches!(storage_mode, StorageMode::Persistent),
        )?;
        let (domain_provider_dir, domain_provider_manifest_file) =
            Self::parse_domain_provider_config(vars)?;

        let license_bundle = resolve_license_bundle(vars)?;
        let (
            licensed_modules,
            license_client_uuid,
            license_client_email,
            license_valid_until,
            license_is_nfr,
        ) = if let Some(bundle) = license_bundle {
            let claims = validate_signed_license(&bundle.license_pem, &bundle.public_key_pem)?;
            (
                claims.modules,
                Some(claims.client_uuid),
                Some(claims.client_email),
                Some(claims.valid_until),
                Some(claims.is_nfr),
            )
        } else {
            if vars.contains_key("CORROBORE_HTTP_LICENSED_MODULES") {
                return Err(ConfigError::InvalidEnv {
                    name: "CORROBORE_HTTP_LICENSED_MODULES",
                    value:
                        "deprecated fallback disabled; provide signed license PEM and public key"
                            .to_owned(),
                });
            }

            (Vec::new(), None, None, None, None)
        };

        Ok(Self {
            host,
            port,
            auth_mode,
            auth_token,
            auth_token_source,
            admin_auth_token,
            admin_auth_token_source,
            operational_endpoint_policy,
            session_store_dir,
            log_dir,
            request_timeout_ms,
            shutdown_timeout_ms,
            session_idle_ttl_ms,
            max_body_bytes,
            import_max_body_bytes,
            rate_limit_per_second,
            rate_limit_burst,
            web_dir,
            licensed_modules,
            license_client_uuid,
            license_client_email,
            license_valid_until,
            license_is_nfr,
            storage_mode,
            storage_dir,
            storage_require_fsync,
            storage_strict_recovery,
            domain_provider_dir,
            domain_provider_manifest_file,
        })
    }

    fn parse_domain_provider_config(
        vars: &HashMap<String, String>,
    ) -> Result<(Option<String>, Option<String>), ConfigError> {
        // Validate that the trusted provider root and deployment manifest are
        // configured together. The runtime loader will later canonicalize both
        // paths, enforce containment, and verify each declared library hash.
        let provider_dir = vars
            .get("CORROBORE_DOMAIN_PROVIDER_DIR")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let manifest_file = vars
            .get("CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        match (provider_dir, manifest_file) {
            (Some(provider_dir), Some(manifest_file)) => {
                Ok((Some(provider_dir), Some(manifest_file)))
            }
            (Some(_), None) => Err(ConfigError::InvalidEnv {
                name: "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE",
                value: "required when CORROBORE_DOMAIN_PROVIDER_DIR is configured".to_owned(),
            }),
            (None, Some(_)) => Err(ConfigError::InvalidEnv {
                name: "CORROBORE_DOMAIN_PROVIDER_DIR",
                value: "required when CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE is configured"
                    .to_owned(),
            }),
            (None, None) => Ok((None, None)),
        }
    }

    #[must_use]
    pub fn is_module_licensed(&self, module: &str) -> bool {
        let module = module.trim().to_ascii_lowercase();
        self.licensed_modules.iter().any(|value| value == &module)
    }

    fn parse_storage_mode(value: &str) -> Result<StorageMode, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ephemeral" => Ok(StorageMode::Ephemeral),
            "persistent" => Ok(StorageMode::Persistent),
            _ => Err(ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_MODE",
                value: value.to_owned(),
            }),
        }
    }

    fn parse_storage_dir(
        vars: &HashMap<String, String>,
        storage_mode: StorageMode,
    ) -> Result<Option<String>, ConfigError> {
        let configured = vars
            .get("CORROBORE_STORAGE_DIR")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        match storage_mode {
            StorageMode::Ephemeral => Ok(None),
            StorageMode::Persistent => configured.map(Some).ok_or(ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_DIR",
                value: "CORROBORE_STORAGE_DIR is required when CORROBORE_STORAGE_MODE=persistent"
                    .to_owned(),
            }),
        }
    }

    fn parse_storage_bool(
        vars: &HashMap<String, String>,
        name: &'static str,
        default: bool,
    ) -> Result<bool, ConfigError> {
        let Some(value) = vars.get(name).map(|raw| raw.trim()) else {
            return Ok(default);
        };

        match value.to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ConfigError::InvalidEnv {
                name,
                value: value.to_owned(),
            }),
        }
    }
}

struct LicenseBundle {
    license_pem: String,
    public_key_pem: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LicenseClaims {
    client_uuid: String,
    client_email: String,
    modules: Vec<String>,
    valid_until: String,
    is_nfr: bool,
}

#[derive(Debug, Deserialize)]
struct SignedLicenseClaims {
    client_uuid: String,
    client_email: String,
    modules: Vec<String>,
    valid_until: String,
    #[serde(default)]
    tags: Vec<String>,
    signature: String,
}

#[derive(Debug, Serialize)]
struct UnsignedLicenseClaims<'a> {
    client_uuid: &'a str,
    client_email: &'a str,
    modules: &'a [String],
    valid_until: &'a str,
    tags: &'a [String],
}

fn resolve_license_bundle(
    vars: &HashMap<String, String>,
) -> Result<Option<LicenseBundle>, ConfigError> {
    let inline_license = vars
        .get("CORROBORE_HTTP_LICENSE_PEM")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let file_license = vars
        .get("CORROBORE_HTTP_LICENSE_PEM_FILE")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let license_pem = match (inline_license, file_license) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PEM",
                value:
                    "set only one of CORROBORE_HTTP_LICENSE_PEM or CORROBORE_HTTP_LICENSE_PEM_FILE"
                        .to_owned(),
            });
        }
        (Some(content), None) => content,
        (None, Some(path)) => {
            fs::read_to_string(&path).map_err(|error| ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PEM_FILE",
                value: format!("{path}: {error}"),
            })?
        }
    };

    let inline_public_key = vars
        .get("CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let file_public_key = vars
        .get("CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let public_key_pem = match (inline_public_key, file_public_key) {
        (None, None) => {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
                value: "public key PEM is required when CORROBORE_HTTP_LICENSE_PEM is set"
                    .to_owned(),
            });
        }
        (Some(_), Some(_)) => {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
                value: "set only one of CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM or CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE"
                    .to_owned(),
            });
        }
        (Some(content), None) => content,
        (None, Some(path)) => {
            fs::read_to_string(&path).map_err(|error| ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE",
                value: format!("{path}: {error}"),
            })?
        }
    };

    Ok(Some(LicenseBundle {
        license_pem,
        public_key_pem,
    }))
}

fn validate_signed_license(
    license_pem: &str,
    public_key_pem: &str,
) -> Result<LicenseClaims, ConfigError> {
    let license_bytes = decode_pem_block(license_pem, "CORROBORE LICENSE")?;
    let signed: SignedLicenseClaims =
        serde_json::from_slice(&license_bytes).map_err(|error| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("invalid license payload json: {error}"),
        })?;

    let client_uuid = signed.client_uuid.trim().to_owned();
    Uuid::parse_str(&client_uuid).map_err(|error| ConfigError::InvalidEnv {
        name: "CORROBORE_HTTP_LICENSE_PEM",
        value: format!("invalid client_uuid: {error}"),
    })?;

    let client_email = signed.client_email.trim().to_ascii_lowercase();
    if !is_email_like(&client_email) {
        return Err(ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: "invalid client_email".to_owned(),
        });
    }

    let modules = normalize_modules(signed.modules);
    let valid_until = parse_and_validate_license_expiry(&signed.valid_until)?;
    let tags = normalize_tags(signed.tags);
    let canonical = UnsignedLicenseClaims {
        client_uuid: &client_uuid,
        client_email: &client_email,
        modules: &modules,
        valid_until: &valid_until,
        tags: &tags,
    };
    let canonical_bytes =
        serde_json::to_vec(&canonical).map_err(|error| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("unable to serialize license payload: {error}"),
        })?;

    let signature_bytes =
        STANDARD
            .decode(signed.signature.trim())
            .map_err(|error| ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PEM",
                value: format!("invalid signature encoding: {error}"),
            })?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|error| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("invalid signature bytes: {error}"),
        })?;

    let public_key_der = decode_pem_block(public_key_pem, "PUBLIC KEY").map_err(|error| {
        ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
            value: match error {
                ConfigError::InvalidEnv { value, .. } => value,
                _ => "invalid public key pem".to_owned(),
            },
        }
    })?;

    let verifying_key =
        VerifyingKey::from_public_key_der(public_key_der.as_slice()).map_err(|error| {
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
                value: format!("invalid public key der: {error}"),
            }
        })?;

    verifying_key
        .verify(&canonical_bytes, &signature)
        .map_err(|_| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: "license signature verification failed".to_owned(),
        })?;

    Ok(LicenseClaims {
        client_uuid,
        client_email,
        modules,
        valid_until,
        is_nfr: tags.iter().any(|tag| tag == "nfr"),
    })
}

fn decode_pem_block(text: &str, label: &str) -> Result<Vec<u8>, ConfigError> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let trimmed = text.trim();
    let start = trimmed
        .find(&begin)
        .ok_or_else(|| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("missing {begin}"),
        })?;
    let tail = &trimmed[start + begin.len()..];
    let stop = tail.find(&end).ok_or_else(|| ConfigError::InvalidEnv {
        name: "CORROBORE_HTTP_LICENSE_PEM",
        value: format!("missing {end}"),
    })?;
    let body = tail[..stop].lines().map(str::trim).collect::<String>();

    STANDARD
        .decode(body)
        .map_err(|error| ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("invalid pem content: {error}"),
        })
}

fn is_email_like(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };

    !local.trim().is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

fn normalize_modules(modules: Vec<String>) -> Vec<String> {
    let mut modules = modules
        .into_iter()
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    modules.sort();
    modules.dedup();
    modules
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut tags = tags
        .into_iter()
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

fn parse_and_validate_license_expiry(value: &str) -> Result<String, ConfigError> {
    let raw = value.trim();
    let parsed = DateTime::parse_from_rfc3339(raw).map_err(|error| ConfigError::InvalidEnv {
        name: "CORROBORE_HTTP_LICENSE_PEM",
        value: format!("invalid valid_until: {error}"),
    })?;
    let valid_until = parsed.with_timezone(&Utc);
    if valid_until <= Utc::now() {
        return Err(ConfigError::InvalidEnv {
            name: "CORROBORE_HTTP_LICENSE_PEM",
            value: format!("license expired at {}", valid_until.to_rfc3339()),
        });
    }

    Ok(raw.to_owned())
}

fn parse_u16(name: &'static str, value: &str) -> Result<u16, ConfigError> {
    value.parse::<u16>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

fn parse_u64(name: &'static str, value: &str) -> Result<u64, ConfigError> {
    value.parse::<u64>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

fn parse_u32(name: &'static str, value: &str) -> Result<u32, ConfigError> {
    value.parse::<u32>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

fn parse_usize(name: &'static str, value: &str) -> Result<usize, ConfigError> {
    value.parse::<usize>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    use super::{ConfigError, ServerConfig, StorageMode};

    #[test]
    fn config_contract_loads_defaults_and_required_token() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("config should parse");

        // 2.4: default bind is the loopback interface, not 0.0.0.0.
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.session_store_dir, ".corrobore-runtime");
        assert_eq!(config.log_dir, ".corrobore-runtime/logs");
        assert_eq!(config.request_timeout_ms, 30_000);
        assert_eq!(config.shutdown_timeout_ms, 5_000);
        assert_eq!(config.session_idle_ttl_ms, 0);
        assert_eq!(config.auth_token.as_deref(), Some("token-123"));
        assert_eq!(config.admin_auth_token, None);

        // 2.3: explicit body-size posture with a larger allowance for imports.
        assert_eq!(config.max_body_bytes, 2 * 1024 * 1024);
        assert_eq!(config.import_max_body_bytes, 32 * 1024 * 1024);

        // 2.3: rate-limiting defaults are permissive but present.
        assert_eq!(config.rate_limit_per_second, 50);
        assert_eq!(config.rate_limit_burst, 200);
        assert_eq!(config.web_dir, None);
        assert!(config.licensed_modules.is_empty());
        assert_eq!(config.license_client_uuid, None);
        assert_eq!(config.license_client_email, None);
        assert_eq!(config.license_valid_until, None);
        assert_eq!(config.license_is_nfr, None);
        assert_eq!(config.storage_mode, StorageMode::Ephemeral);
        assert_eq!(config.storage_dir, None);
        assert_eq!(config.domain_provider_dir, None);
        assert_eq!(config.domain_provider_manifest_file, None);
    }

    #[test]
    fn config_contract_parses_domain_provider_manifest_pair() {
        let vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
                "/opt/corrobore/providers".to_owned(),
            ),
            (
                "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
                "/etc/corrobore/providers.json".to_owned(),
            ),
        ]);

        let config = ServerConfig::from_map(&vars).expect("provider configuration should parse");

        assert_eq!(
            config.domain_provider_dir.as_deref(),
            Some("/opt/corrobore/providers")
        );
        assert_eq!(
            config.domain_provider_manifest_file.as_deref(),
            Some("/etc/corrobore/providers.json")
        );
    }

    #[test]
    fn config_contract_rejects_partial_domain_provider_configuration() {
        let dir_only = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
                "/opt/corrobore/providers".to_owned(),
            ),
        ]);

        let error = ServerConfig::from_map(&dir_only)
            .expect_err("provider directory without manifest should fail");

        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE",
                value: "required when CORROBORE_DOMAIN_PROVIDER_DIR is configured".to_owned(),
            }
        );

        let manifest_only = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
                "/etc/corrobore/providers.json".to_owned(),
            ),
        ]);

        let error = ServerConfig::from_map(&manifest_only)
            .expect_err("provider manifest without directory should fail");

        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_DOMAIN_PROVIDER_DIR",
                value: "required when CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE is configured"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_allows_opt_in_public_bind() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_HOST".to_owned(), "0.0.0.0".to_owned());

        let config = ServerConfig::from_map(&vars).expect("config should parse");
        assert_eq!(config.host, "0.0.0.0");
    }

    #[test]
    fn config_contract_parses_optional_admin_token() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            " admin-token ".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("config should parse");
        assert_eq!(config.admin_auth_token.as_deref(), Some("admin-token"));
    }

    #[test]
    fn config_contract_overrides_body_and_rate_limits() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_MAX_BODY_BYTES".to_owned(), "10".to_owned());
        vars.insert(
            "CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES".to_owned(),
            "64".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND".to_owned(),
            "1".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_RATE_LIMIT_BURST".to_owned(), "1".to_owned());

        let config = ServerConfig::from_map(&vars).expect("config should parse");
        assert_eq!(config.max_body_bytes, 10);
        assert_eq!(config.import_max_body_bytes, 64);
        assert_eq!(config.rate_limit_per_second, 1);
        assert_eq!(config.rate_limit_burst, 1);
    }

    #[test]
    fn config_contract_enables_optional_web_delivery() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_WEB_DIR".to_owned(),
            "  web/dist  ".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("config should parse");

        assert_eq!(config.web_dir.as_deref(), Some("web/dist"));
    }

    #[test]
    fn config_contract_rejects_legacy_licensed_modules_fallback() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            " cti, crisis,cti ,fimi ".to_owned(),
        );

        let error = ServerConfig::from_map(&vars).expect_err("legacy fallback should be rejected");

        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSED_MODULES",
                value: "deprecated fallback disabled; provide signed license PEM and public key"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_validates_signed_license_pem_claims() {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let verifying_pem = public_key_pem(&signing.verifying_key());

        let canonical = signable_payload(
            "11111111-2222-4333-8444-555555555555",
            "security@example.com",
            vec!["fimi".to_owned(), "cti".to_owned(), "cti".to_owned()],
            "2099-01-01T00:00:00Z",
            vec!["NFR".to_owned()],
        );
        let signature = signing.sign(&canonical);

        let signed = json!({
            "client_uuid": "11111111-2222-4333-8444-555555555555",
            "client_email": "security@example.com",
            "modules": ["fimi", "cti", "cti"],
            "valid_until": "2099-01-01T00:00:00Z",
            "tags": ["NFR"],
            "signature": STANDARD.encode(signature.to_bytes())
        });

        let license_json = serde_json::to_vec(&signed).expect("license json should serialize");
        let license_pem = format!(
            "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
            STANDARD.encode(license_json)
        );

        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_LICENSE_PEM".to_owned(), license_pem);
        vars.insert(
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM".to_owned(),
            verifying_pem,
        );

        let config = ServerConfig::from_map(&vars).expect("signed license should parse");

        assert_eq!(config.licensed_modules, vec!["cti", "fimi"]);
        assert_eq!(
            config.license_client_uuid.as_deref(),
            Some("11111111-2222-4333-8444-555555555555")
        );
        assert_eq!(
            config.license_client_email.as_deref(),
            Some("security@example.com")
        );
        assert_eq!(
            config.license_valid_until.as_deref(),
            Some("2099-01-01T00:00:00Z")
        );
        assert_eq!(config.license_is_nfr, Some(true));
    }

    #[test]
    fn config_contract_rejects_signed_license_with_invalid_signature() {
        let signing = SigningKey::from_bytes(&[9_u8; 32]);
        let verifying_pem = public_key_pem(&signing.verifying_key());

        let signed = json!({
            "client_uuid": "11111111-2222-4333-8444-555555555555",
            "client_email": "security@example.com",
            "modules": ["cti"],
            "valid_until": "2099-01-01T00:00:00Z",
            "tags": [],
            "signature": STANDARD.encode([0_u8; 64])
        });

        let license_json = serde_json::to_vec(&signed).expect("license json should serialize");
        let license_pem = format!(
            "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
            STANDARD.encode(license_json)
        );

        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_LICENSE_PEM".to_owned(), license_pem);
        vars.insert(
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM".to_owned(),
            verifying_pem,
        );

        let error = ServerConfig::from_map(&vars).expect_err("invalid signature should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PEM",
                value: "license signature verification failed".to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_rejects_signed_license_without_public_key() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_LICENSE_PEM".to_owned(),
            "-----BEGIN CORROBORE LICENSE-----\nZm9v\n-----END CORROBORE LICENSE-----".to_owned(),
        );

        let error = ServerConfig::from_map(&vars).expect_err("public key is mandatory");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
                value: "public key PEM is required when CORROBORE_HTTP_LICENSE_PEM is set"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_loads_signed_license_from_files() {
        let signing = SigningKey::from_bytes(&[11_u8; 32]);
        let verifying_pem = public_key_pem(&signing.verifying_key());

        let canonical = signable_payload(
            "66666666-7777-4888-9999-aaaaaaaaaaaa",
            "ops@example.com",
            vec!["crisis".to_owned()],
            "2099-06-01T00:00:00Z",
            vec![],
        );
        let signature = signing.sign(&canonical);

        let signed = json!({
            "client_uuid": "66666666-7777-4888-9999-aaaaaaaaaaaa",
            "client_email": "ops@example.com",
            "modules": ["crisis"],
            "valid_until": "2099-06-01T00:00:00Z",
            "tags": [],
            "signature": STANDARD.encode(signature.to_bytes())
        });
        let license_json = serde_json::to_vec(&signed).expect("license json should serialize");
        let license_pem = format!(
            "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
            STANDARD.encode(license_json)
        );

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let tmp = std::env::temp_dir();
        let license_path = tmp.join(format!("corrobore-license-{nonce}.pem"));
        let public_key_path = tmp.join(format!("corrobore-license-public-key-{nonce}.pem"));
        fs::write(&license_path, license_pem).expect("license file should write");
        fs::write(&public_key_path, verifying_pem).expect("public key file should write");

        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_LICENSE_PEM_FILE".to_owned(),
            license_path.display().to_string(),
        );
        vars.insert(
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE".to_owned(),
            public_key_path.display().to_string(),
        );

        let config = ServerConfig::from_map(&vars).expect("signed license file should parse");
        assert_eq!(config.licensed_modules, vec!["crisis"]);
        assert_eq!(
            config.license_client_uuid.as_deref(),
            Some("66666666-7777-4888-9999-aaaaaaaaaaaa")
        );
        assert_eq!(
            config.license_client_email.as_deref(),
            Some("ops@example.com")
        );
        assert_eq!(
            config.license_valid_until.as_deref(),
            Some("2099-06-01T00:00:00Z")
        );
        assert_eq!(config.license_is_nfr, Some(false));

        let _ = fs::remove_file(license_path);
        let _ = fs::remove_file(public_key_path);
    }

    #[test]
    fn config_contract_rejects_expired_signed_license() {
        let signing = SigningKey::from_bytes(&[13_u8; 32]);
        let verifying_pem = public_key_pem(&signing.verifying_key());

        let canonical = signable_payload(
            "11111111-2222-4333-8444-555555555555",
            "security@example.com",
            vec!["cti".to_owned()],
            "2000-01-01T00:00:00Z",
            vec![],
        );
        let signature = signing.sign(&canonical);

        let signed = json!({
            "client_uuid": "11111111-2222-4333-8444-555555555555",
            "client_email": "security@example.com",
            "modules": ["cti"],
            "valid_until": "2000-01-01T00:00:00Z",
            "tags": [],
            "signature": STANDARD.encode(signature.to_bytes())
        });
        let license_json = serde_json::to_vec(&signed).expect("license json should serialize");
        let license_pem = format!(
            "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
            STANDARD.encode(license_json)
        );

        let vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            ("CORROBORE_HTTP_LICENSE_PEM".to_owned(), license_pem),
            (
                "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM".to_owned(),
                verifying_pem,
            ),
        ]);

        let error = ServerConfig::from_map(&vars).expect_err("expired license should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_LICENSE_PEM",
                value: "license expired at 2000-01-01T00:00:00+00:00".to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_rejects_missing_auth_token() {
        let vars = HashMap::new();
        let error = ServerConfig::from_map(&vars).expect_err("token must be required");
        assert_eq!(error, ConfigError::MissingEnv("CORROBORE_HTTP_AUTH_TOKEN"));
    }

    #[test]
    fn config_contract_rejects_invalid_numeric_values() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_PORT".to_owned(), "x".to_owned());

        let error = ServerConfig::from_map(&vars).expect_err("invalid port should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_HTTP_PORT",
                value: "x".to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_parses_persistent_storage_mode_with_storage_dir() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
        vars.insert(
            "CORROBORE_STORAGE_DIR".to_owned(),
            " .corrobore-runtime/graph ".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("persistent mode should parse");
        assert_eq!(config.storage_mode, StorageMode::Persistent);
        assert_eq!(
            config.storage_dir.as_deref(),
            Some(".corrobore-runtime/graph")
        );
        assert!(config.storage_require_fsync);
        assert!(config.storage_strict_recovery);
    }

    #[test]
    fn config_contract_rejects_unknown_storage_mode() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_STORAGE_MODE".to_owned(), "sqlite".to_owned());

        let error = ServerConfig::from_map(&vars).expect_err("unknown mode should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_MODE",
                value: "sqlite".to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_rejects_persistent_mode_without_storage_dir() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());

        let error = ServerConfig::from_map(&vars).expect_err("missing storage dir should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_DIR",
                value: "CORROBORE_STORAGE_DIR is required when CORROBORE_STORAGE_MODE=persistent"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_ephemeral_mode_defaults_disable_strict_durability_controls() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("ephemeral mode should parse");
        assert_eq!(config.storage_mode, StorageMode::Ephemeral);
        assert!(!config.storage_require_fsync);
        assert!(!config.storage_strict_recovery);
    }

    #[test]
    fn config_contract_persistent_mode_allows_explicit_durability_control_overrides() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
        vars.insert(
            "CORROBORE_STORAGE_DIR".to_owned(),
            ".corrobore-runtime/graph".to_owned(),
        );
        vars.insert(
            "CORROBORE_STORAGE_REQUIRE_FSYNC".to_owned(),
            "false".to_owned(),
        );
        vars.insert(
            "CORROBORE_STORAGE_STRICT_RECOVERY".to_owned(),
            "false".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("persistent overrides should parse");
        assert!(!config.storage_require_fsync);
        assert!(!config.storage_strict_recovery);
    }

    #[test]
    fn config_contract_rejects_invalid_durability_control_values() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_STORAGE_REQUIRE_FSYNC".to_owned(),
            "always".to_owned(),
        );

        let error =
            ServerConfig::from_map(&vars).expect_err("invalid durability control should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_REQUIRE_FSYNC",
                value: "always".to_owned(),
            }
        );
    }

    fn public_key_pem(verifying_key: &ed25519_dalek::VerifyingKey) -> String {
        let der = verifying_key
            .to_public_key_der()
            .expect("public key der should serialize")
            .as_bytes()
            .to_vec();
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
            STANDARD.encode(der)
        )
    }

    fn signable_payload(
        client_uuid: &str,
        client_email: &str,
        modules: Vec<String>,
        valid_until: &str,
        tags: Vec<String>,
    ) -> Vec<u8> {
        let modules = super::normalize_modules(modules);
        let tags = super::normalize_tags(tags);
        serde_json::to_vec(&super::UnsignedLicenseClaims {
            client_uuid,
            client_email,
            modules: &modules,
            valid_until,
            tags: &tags,
        })
        .expect("signable payload should serialize")
    }
}
