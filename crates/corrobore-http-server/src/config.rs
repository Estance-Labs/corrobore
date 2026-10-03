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

use corrobore_engine::MemoryPermissions;
use thiserror::Error;

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
    /// Bolt listener port, used only when the `bolt` interface is enabled. The
    /// listener shares `host`, the bearer token and the TLS material.
    pub bolt_port: u16,
    /// Maximum concurrent Bolt connections; excess connections wait for a slot.
    pub bolt_max_connections: usize,
    /// PostgreSQL wire listener port, used only when the `sql` interface is
    /// enabled. It shares `host`, the bearer token and the TLS material.
    pub sql_port: u16,
    /// Maximum concurrent SQL connections; excess connections wait for a slot.
    pub sql_max_connections: usize,
    pub auth_mode: AuthenticationMode,
    pub auth_token: Option<String>,
    pub auth_token_source: Option<SecretSource>,
    pub admin_auth_token: Option<String>,
    pub admin_auth_token_source: Option<SecretSource>,
    pub operational_endpoint_policy: OperationalEndpointPolicy,
    /// Trusted workspace assigned to high-level memory API requests.
    pub memory_workspace_id: String,
    /// Trusted actor assigned after standalone bearer authentication.
    pub memory_actor_id: String,
    /// Optional trusted agent assigned by standalone deployment policy.
    pub memory_agent_id: Option<String>,
    /// Trusted session assigned to high-level memory API requests.
    pub memory_session_id: String,
    /// Independently configured high-level memory capabilities.
    pub memory_permissions: MemoryPermissions,
    pub session_store_dir: String,
    pub log_dir: String,
    pub request_timeout_ms: u64,
    pub shutdown_timeout_ms: u64,
    pub session_idle_ttl_ms: u64,
    /// Maximum request body size (bytes) for standard JSON routes (2.3).
    pub max_body_bytes: usize,
    /// Maximum request body size (bytes) for STIX import routes (2.3).
    pub import_max_body_bytes: usize,
    /// Maximum OpenCTI mutations accepted in one synchronization batch.
    pub opencti_sync_max_operations: usize,
    /// Maximum replay identities and dead-letter diagnostics retained.
    pub opencti_sync_max_replay_identities: usize,
    /// Final single-node mode with no reference Elasticsearch/OpenSearch provider.
    pub opencti_elastic_free: bool,
    /// OpenCTI asynchronous shadow-read policy and reference provider.
    pub opencti_shadow: OpenCtiShadowConfig,
    /// Sustained request rate per second for the global rate limiter (2.3).
    pub rate_limit_per_second: u64,
    /// Burst allowance for the global rate limiter (2.3).
    pub rate_limit_burst: u32,
    /// Sustained request rate reserved for authenticated OpenCTI provider traffic.
    pub opencti_rate_limit_per_second: u64,
    /// Burst allowance reserved for authenticated OpenCTI provider traffic.
    pub opencti_rate_limit_burst: u32,
    /// Optional directory containing the production explorer build.
    pub web_dir: Option<String>,
    /// Runtime graph storage mode.
    pub storage_mode: StorageMode,
    /// Graph storage directory configured for persistent mode.
    pub storage_dir: Option<String>,
    /// Durability control: require fsync for persistent graph mutation writes.
    pub storage_require_fsync: bool,
    /// Durability control: enforce strict recovery checks in persistent mode.
    pub storage_strict_recovery: bool,
    /// Maximum node payloads resident in a persistent request projection.
    pub storage_max_hot_nodes: u64,
    /// Maximum relationship payloads resident in a persistent request projection.
    pub storage_max_hot_relationships: u64,
    /// Maximum lightweight adjacency entries resident in a persistent projection.
    pub storage_max_warm_adjacency_entries: u64,
    /// Trusted root containing native domain provider libraries.
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
            .field("bolt_port", &self.bolt_port)
            .field("bolt_max_connections", &self.bolt_max_connections)
            .field("sql_port", &self.sql_port)
            .field("sql_max_connections", &self.sql_max_connections)
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
            .field("memory_workspace_id", &self.memory_workspace_id)
            .field("memory_actor_id", &self.memory_actor_id)
            .field("memory_agent_id", &self.memory_agent_id)
            .field("memory_session_id", &self.memory_session_id)
            .field("memory_permissions", &self.memory_permissions)
            .field("session_store_dir", &self.session_store_dir)
            .field("log_dir", &self.log_dir)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("shutdown_timeout_ms", &self.shutdown_timeout_ms)
            .field("session_idle_ttl_ms", &self.session_idle_ttl_ms)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("import_max_body_bytes", &self.import_max_body_bytes)
            .field(
                "opencti_sync_max_operations",
                &self.opencti_sync_max_operations,
            )
            .field(
                "opencti_sync_max_replay_identities",
                &self.opencti_sync_max_replay_identities,
            )
            .field("opencti_elastic_free", &self.opencti_elastic_free)
            .field("opencti_shadow", &self.opencti_shadow)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field(
                "opencti_rate_limit_per_second",
                &self.opencti_rate_limit_per_second,
            )
            .field("opencti_rate_limit_burst", &self.opencti_rate_limit_burst)
            .field("web_dir", &self.web_dir)
            .field("storage_mode", &self.storage_mode)
            .field("storage_dir", &self.storage_dir)
            .field("storage_require_fsync", &self.storage_require_fsync)
            .field("storage_strict_recovery", &self.storage_strict_recovery)
            .field("storage_max_hot_nodes", &self.storage_max_hot_nodes)
            .field(
                "storage_max_hot_relationships",
                &self.storage_max_hot_relationships,
            )
            .field(
                "storage_max_warm_adjacency_entries",
                &self.storage_max_warm_adjacency_entries,
            )
            .field("domain_provider_dir", &self.domain_provider_dir)
            .field(
                "domain_provider_manifest_file",
                &self.domain_provider_manifest_file,
            )
            .finish()
    }
}

/// OpenCTI shadow-read transport, sampling, deadline, and retention settings.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenCtiShadowConfig {
    /// Fixed reference Knowledge Data Engine endpoint.
    pub reference_endpoint: Option<String>,
    /// Optional bearer token for the reference endpoint.
    pub reference_auth_token: Option<String>,
    /// Origin of the reference bearer token.
    pub reference_auth_token_source: Option<SecretSource>,
    /// Operator-supplied Elasticsearch/OpenSearch version.
    pub reference_version: String,
    /// Bounded Corrobore release label used in reports and metrics.
    pub release: String,
    /// Fallback deterministic sampling percentage in basis points.
    pub sample_basis_points: u16,
    /// Maximum accepted shadow executions.
    pub max_concurrency: usize,
    /// Independent shadow deadline.
    pub timeout_ms: u64,
    /// Maximum durable reports retained.
    pub max_reports: usize,
    /// Optional JSON sampling policy file.
    pub sampling_policy_file: Option<String>,
    /// Optional JSON baseline file.
    pub baseline_file: Option<String>,
    /// Optional JSON progressive read-routing policy file.
    pub routing_policy_file: Option<String>,
    /// Maximum durable provider-decision audit events retained.
    pub routing_max_audits: usize,
}

impl fmt::Debug for OpenCtiShadowConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenCtiShadowConfig")
            .field("reference_endpoint", &self.reference_endpoint)
            .field(
                "reference_auth_token",
                &self.reference_auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "reference_auth_token_source",
                &self.reference_auth_token_source,
            )
            .field("reference_version", &self.reference_version)
            .field("release", &self.release)
            .field("sample_basis_points", &self.sample_basis_points)
            .field("max_concurrency", &self.max_concurrency)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_reports", &self.max_reports)
            .field("sampling_policy_file", &self.sampling_policy_file)
            .field("baseline_file", &self.baseline_file)
            .field("routing_policy_file", &self.routing_policy_file)
            .field("routing_max_audits", &self.routing_max_audits)
            .finish()
    }
}

impl Default for OpenCtiShadowConfig {
    fn default() -> Self {
        Self {
            reference_endpoint: None,
            reference_auth_token: None,
            reference_auth_token_source: None,
            reference_version: "unconfigured".to_owned(),
            release: env!("CARGO_PKG_VERSION").to_owned(),
            sample_basis_points: 0,
            max_concurrency: 4,
            timeout_ms: 2_000,
            max_reports: 10_000,
            sampling_policy_file: None,
            baseline_file: None,
            routing_policy_file: None,
            routing_max_audits: 10_000,
        }
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

        let bolt_port = parse_u16(
            "CORROBORE_BOLT_PORT",
            vars.get("CORROBORE_BOLT_PORT")
                .map(String::as_str)
                .unwrap_or("7687"),
        )?;
        let bolt_max_connections = parse_positive_usize(
            "CORROBORE_BOLT_MAX_CONNECTIONS",
            vars.get("CORROBORE_BOLT_MAX_CONNECTIONS")
                .map(String::as_str)
                .unwrap_or("64"),
        )?;

        let sql_port = parse_u16(
            "CORROBORE_SQL_PORT",
            vars.get("CORROBORE_SQL_PORT")
                .map(String::as_str)
                .unwrap_or("5432"),
        )?;
        let sql_max_connections = parse_positive_usize(
            "CORROBORE_SQL_MAX_CONNECTIONS",
            vars.get("CORROBORE_SQL_MAX_CONNECTIONS")
                .map(String::as_str)
                .unwrap_or("64"),
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
        let memory_workspace_id = trusted_memory_id(
            vars,
            "CORROBORE_MEMORY_WORKSPACE_ID",
            "workspace--standalone-default",
        )?;
        let memory_actor_id = trusted_memory_id(
            vars,
            "CORROBORE_MEMORY_ACTOR_ID",
            "actor--standalone-client",
        )?;
        let memory_agent_id = vars
            .get("CORROBORE_MEMORY_AGENT_ID")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let memory_session_id = trusted_memory_id(
            vars,
            "CORROBORE_MEMORY_SESSION_ID",
            "session--standalone-api",
        )?;
        let memory_permissions = parse_memory_permissions(vars)?;

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

        let opencti_sync_max_operations = parse_positive_usize(
            "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS",
            vars.get("CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS")
                .map(String::as_str)
                .unwrap_or("512"),
        )?;

        let opencti_sync_max_replay_identities = parse_positive_usize(
            "CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES",
            vars.get("CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES")
                .map(String::as_str)
                .unwrap_or("4096"),
        )?;

        let opencti_elastic_free =
            Self::parse_storage_bool(vars, "CORROBORE_OPENCTI_ELASTIC_FREE", false)?;

        let reference_endpoint = vars
            .get("CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let (reference_auth_token, reference_auth_token_source) = resolve_secret(
            vars,
            "CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN",
            "CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN_FILE",
            "opencti_shadow.reference_auth_token",
            "opencti_shadow.reference_auth_token_file",
        )?;
        let reference_version = vars
            .get("CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION")
            .map_or("unconfigured", String::as_str)
            .trim()
            .to_owned();
        let release = vars
            .get("CORROBORE_OPENCTI_SHADOW_RELEASE")
            .map_or(env!("CARGO_PKG_VERSION"), String::as_str)
            .trim()
            .to_owned();
        if reference_endpoint.is_some() && reference_version.is_empty() {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION",
                value: reference_version,
            });
        }
        if release.is_empty() {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_SHADOW_RELEASE",
                value: release,
            });
        }
        let sample_basis_points = parse_u16(
            "CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS",
            vars.get("CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS")
                .map(String::as_str)
                .unwrap_or("0"),
        )?;
        if sample_basis_points > 10_000 {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS",
                value: sample_basis_points.to_string(),
            });
        }
        let max_concurrency = parse_positive_usize(
            "CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY",
            vars.get("CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY")
                .map(String::as_str)
                .unwrap_or("4"),
        )?;
        let timeout_ms = parse_positive_u64(
            "CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS",
            vars.get("CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS")
                .map(String::as_str)
                .unwrap_or("2000"),
        )?;
        let max_reports = parse_positive_usize(
            "CORROBORE_OPENCTI_SHADOW_MAX_REPORTS",
            vars.get("CORROBORE_OPENCTI_SHADOW_MAX_REPORTS")
                .map(String::as_str)
                .unwrap_or("10000"),
        )?;
        let sampling_policy_file = vars
            .get("CORROBORE_OPENCTI_SHADOW_SAMPLING_POLICY_FILE")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let baseline_file = vars
            .get("CORROBORE_OPENCTI_SHADOW_BASELINE_FILE")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let routing_policy_file = vars
            .get("CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let routing_max_audits = parse_positive_usize(
            "CORROBORE_OPENCTI_READ_ROUTING_MAX_AUDITS",
            vars.get("CORROBORE_OPENCTI_READ_ROUTING_MAX_AUDITS")
                .map(String::as_str)
                .unwrap_or("10000"),
        )?;
        let opencti_shadow = OpenCtiShadowConfig {
            reference_endpoint,
            reference_auth_token,
            reference_auth_token_source,
            reference_version,
            release,
            sample_basis_points,
            max_concurrency,
            timeout_ms,
            max_reports,
            sampling_policy_file,
            baseline_file,
            routing_policy_file,
            routing_max_audits,
        };

        let rate_limit_per_second = parse_positive_u64(
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND",
            vars.get("CORROBORE_HTTP_RATE_LIMIT_PER_SECOND")
                .map(String::as_str)
                .unwrap_or("50"),
        )?;

        let rate_limit_burst = parse_positive_u32(
            "CORROBORE_HTTP_RATE_LIMIT_BURST",
            vars.get("CORROBORE_HTTP_RATE_LIMIT_BURST")
                .map(String::as_str)
                .unwrap_or("200"),
        )?;

        let opencti_rate_limit_per_second = parse_positive_u64(
            "CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND",
            vars.get("CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND")
                .map(String::as_str)
                .unwrap_or("50"),
        )?;

        let opencti_rate_limit_burst = parse_positive_u32(
            "CORROBORE_OPENCTI_RATE_LIMIT_BURST",
            vars.get("CORROBORE_OPENCTI_RATE_LIMIT_BURST")
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
        let storage_max_hot_nodes = parse_positive_u64(
            "CORROBORE_STORAGE_MAX_HOT_NODES",
            vars.get("CORROBORE_STORAGE_MAX_HOT_NODES")
                .map(String::as_str)
                .unwrap_or("16384"),
        )?;
        let storage_max_hot_relationships = parse_positive_u64(
            "CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS",
            vars.get("CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS")
                .map(String::as_str)
                .unwrap_or("32768"),
        )?;
        let storage_max_warm_adjacency_entries = parse_positive_u64(
            "CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES",
            vars.get("CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES")
                .map(String::as_str)
                .unwrap_or("65536"),
        )?;
        if opencti_elastic_free && storage_mode != StorageMode::Persistent {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_ELASTIC_FREE",
                value: "true requires CORROBORE_STORAGE_MODE=persistent".to_owned(),
            });
        }
        if opencti_elastic_free && opencti_shadow.reference_endpoint.is_some() {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_ELASTIC_FREE",
                value: "true forbids CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT".to_owned(),
            });
        }
        if opencti_elastic_free && opencti_shadow.routing_policy_file.is_none() {
            return Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_ELASTIC_FREE",
                value: "true requires CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE".to_owned(),
            });
        }
        let (domain_provider_dir, domain_provider_manifest_file) =
            Self::parse_domain_provider_config(vars)?;

        reject_removed_license_variables(vars)?;

        Ok(Self {
            host,
            port,
            bolt_port,
            bolt_max_connections,
            sql_port,
            sql_max_connections,
            auth_mode,
            auth_token,
            auth_token_source,
            admin_auth_token,
            admin_auth_token_source,
            operational_endpoint_policy,
            memory_workspace_id,
            memory_actor_id,
            memory_agent_id,
            memory_session_id,
            memory_permissions,
            session_store_dir,
            log_dir,
            request_timeout_ms,
            shutdown_timeout_ms,
            session_idle_ttl_ms,
            max_body_bytes,
            import_max_body_bytes,
            opencti_sync_max_operations,
            opencti_sync_max_replay_identities,
            opencti_elastic_free,
            opencti_shadow,
            rate_limit_per_second,
            rate_limit_burst,
            opencti_rate_limit_per_second,
            opencti_rate_limit_burst,
            web_dir,
            storage_mode,
            storage_dir,
            storage_require_fsync,
            storage_strict_recovery,
            storage_max_hot_nodes,
            storage_max_hot_relationships,
            storage_max_warm_adjacency_entries,
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

fn trusted_memory_id(
    vars: &HashMap<String, String>,
    name: &'static str,
    default: &str,
) -> Result<String, ConfigError> {
    let value = vars
        .get(name)
        .map_or(default, String::as_str)
        .trim()
        .to_owned();
    if value.is_empty() || value.len() > 256 {
        return Err(ConfigError::InvalidEnv { name, value });
    }
    Ok(value)
}

fn parse_memory_permissions(
    vars: &HashMap<String, String>,
) -> Result<MemoryPermissions, ConfigError> {
    let raw = vars
        .get("CORROBORE_MEMORY_PERMISSIONS")
        .map_or("read,write,trace,forget,consolidate", String::as_str);
    let values = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.iter().any(|value| {
        !matches!(
            *value,
            "read" | "write" | "trace" | "forget" | "consolidate"
        )
    }) {
        return Err(ConfigError::InvalidEnv {
            name: "CORROBORE_MEMORY_PERMISSIONS",
            value: raw.to_owned(),
        });
    }
    Ok(MemoryPermissions {
        read: values.contains(&"read"),
        write: values.contains(&"write"),
        trace: values.contains(&"trace"),
        forget: values.contains(&"forget"),
        consolidate: values.contains(&"consolidate"),
    })
}

/// Variables of the signed-license gate Corrobore no longer evaluates.
const REMOVED_LICENSE_VARIABLES: [&str; 5] = [
    "CORROBORE_HTTP_LICENSE_PEM",
    "CORROBORE_HTTP_LICENSE_PEM_FILE",
    "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
    "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE",
    "CORROBORE_HTTP_LICENSED_MODULES",
];

/// Fails startup when a license variable is still set. Corrobore carries a
/// single license and gates no module, so a leftover variable from a
/// signed-license deployment is a configuration error, not a no-op: ignoring
/// it would let an operator believe a license gate is in force.
fn reject_removed_license_variables(vars: &HashMap<String, String>) -> Result<(), ConfigError> {
    match REMOVED_LICENSE_VARIABLES
        .into_iter()
        .find(|name| vars.contains_key(*name))
    {
        Some(name) => Err(ConfigError::InvalidEnv {
            name,
            value: "license-gated modules are not part of Corrobore; remove this variable"
                .to_owned(),
        }),
        None => Ok(()),
    }
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

fn parse_positive_u64(name: &'static str, value: &str) -> Result<u64, ConfigError> {
    let parsed = parse_u64(name, value)?;
    if parsed == 0 {
        return Err(ConfigError::InvalidEnv {
            name,
            value: value.to_owned(),
        });
    }
    Ok(parsed)
}

fn parse_u32(name: &'static str, value: &str) -> Result<u32, ConfigError> {
    value.parse::<u32>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

fn parse_positive_u32(name: &'static str, value: &str) -> Result<u32, ConfigError> {
    let parsed = parse_u32(name, value)?;
    if parsed == 0 {
        return Err(ConfigError::InvalidEnv {
            name,
            value: value.to_owned(),
        });
    }
    Ok(parsed)
}

fn parse_usize(name: &'static str, value: &str) -> Result<usize, ConfigError> {
    value.parse::<usize>().map_err(|_| ConfigError::InvalidEnv {
        name,
        value: value.to_owned(),
    })
}

fn parse_positive_usize(name: &'static str, value: &str) -> Result<usize, ConfigError> {
    let parsed = parse_usize(name, value)?;
    if parsed == 0 {
        return Err(ConfigError::InvalidEnv {
            name,
            value: value.to_owned(),
        });
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
        assert_eq!(config.opencti_sync_max_operations, 512);
        assert_eq!(config.opencti_sync_max_replay_identities, 4_096);
        assert_eq!(config.opencti_shadow.reference_endpoint, None);
        assert_eq!(config.opencti_shadow.sample_basis_points, 0);
        assert_eq!(config.opencti_shadow.max_concurrency, 4);
        assert_eq!(config.opencti_shadow.timeout_ms, 2_000);
        assert_eq!(config.opencti_shadow.max_reports, 10_000);
        assert_eq!(config.opencti_shadow.routing_policy_file, None);
        assert_eq!(config.opencti_shadow.routing_max_audits, 10_000);

        // 2.3: rate-limiting defaults are permissive but present.
        assert_eq!(config.rate_limit_per_second, 50);
        assert_eq!(config.rate_limit_burst, 200);
        assert_eq!(config.opencti_rate_limit_per_second, 50);
        assert_eq!(config.opencti_rate_limit_burst, 200);
        assert_eq!(config.web_dir, None);
        assert_eq!(config.storage_mode, StorageMode::Ephemeral);
        assert_eq!(config.storage_dir, None);
        assert_eq!(config.storage_max_hot_nodes, 16_384);
        assert_eq!(config.storage_max_hot_relationships, 32_768);
        assert_eq!(config.storage_max_warm_adjacency_entries, 65_536);
        assert_eq!(config.domain_provider_dir, None);
        assert_eq!(config.domain_provider_manifest_file, None);
    }

    #[test]
    fn config_contract_parses_bounded_opencti_shadow_controls() {
        let vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT".to_owned(),
                "https://reference.example.test/v1/knowledge-data".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION".to_owned(),
                "opensearch-2.19.2".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN".to_owned(),
                "reference-secret".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_RELEASE".to_owned(),
                "release-43".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS".to_owned(),
                "2500".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY".to_owned(),
                "7".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS".to_owned(),
                "750".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_MAX_REPORTS".to_owned(),
                "99".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE".to_owned(),
                "/etc/corrobore/opencti-read-routing.json".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_READ_ROUTING_MAX_AUDITS".to_owned(),
                "123".to_owned(),
            ),
        ]);

        let config = ServerConfig::from_map(&vars).expect("shadow configuration should parse");
        assert_eq!(
            config.opencti_shadow.reference_endpoint.as_deref(),
            Some("https://reference.example.test/v1/knowledge-data")
        );
        assert_eq!(
            config.opencti_shadow.reference_auth_token.as_deref(),
            Some("reference-secret")
        );
        assert_eq!(config.opencti_shadow.reference_version, "opensearch-2.19.2");
        assert_eq!(config.opencti_shadow.release, "release-43");
        assert_eq!(config.opencti_shadow.sample_basis_points, 2_500);
        assert_eq!(config.opencti_shadow.max_concurrency, 7);
        assert_eq!(config.opencti_shadow.timeout_ms, 750);
        assert_eq!(config.opencti_shadow.max_reports, 99);
        assert_eq!(
            config.opencti_shadow.routing_policy_file.as_deref(),
            Some("/etc/corrobore/opencti-read-routing.json")
        );
        assert_eq!(config.opencti_shadow.routing_max_audits, 123);
        assert!(!format!("{config:?}").contains("reference-secret"));
    }

    #[test]
    fn config_contract_rejects_out_of_range_shadow_sampling() {
        let vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS".to_owned(),
                "10001".to_owned(),
            ),
        ]);

        assert!(matches!(
            ServerConfig::from_map(&vars),
            Err(ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS",
                ..
            })
        ));
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
            "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS".to_owned(),
            "7".to_owned(),
        );
        vars.insert(
            "CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES".to_owned(),
            "11".to_owned(),
        );
        vars.insert(
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND".to_owned(),
            "1".to_owned(),
        );
        vars.insert("CORROBORE_HTTP_RATE_LIMIT_BURST".to_owned(), "1".to_owned());
        vars.insert(
            "CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND".to_owned(),
            "250".to_owned(),
        );
        vars.insert(
            "CORROBORE_OPENCTI_RATE_LIMIT_BURST".to_owned(),
            "2000".to_owned(),
        );

        let config = ServerConfig::from_map(&vars).expect("config should parse");
        assert_eq!(config.max_body_bytes, 10);
        assert_eq!(config.import_max_body_bytes, 64);
        assert_eq!(config.opencti_sync_max_operations, 7);
        assert_eq!(config.opencti_sync_max_replay_identities, 11);
        assert_eq!(config.rate_limit_per_second, 1);
        assert_eq!(config.rate_limit_burst, 1);
        assert_eq!(config.opencti_rate_limit_per_second, 250);
        assert_eq!(config.opencti_rate_limit_burst, 2_000);
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

    /// Corrobore carries a single license and evaluates no license claim. A
    /// variable left over from a signed-license deployment must stop startup
    /// with a pointer to the variable, never be ignored: an ignored variable
    /// would let an operator believe a license gate is in force.
    #[test]
    fn config_contract_rejects_removed_license_variables() {
        for name in [
            "CORROBORE_HTTP_LICENSE_PEM",
            "CORROBORE_HTTP_LICENSE_PEM_FILE",
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM",
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE",
            "CORROBORE_HTTP_LICENSED_MODULES",
        ] {
            let vars = HashMap::from([
                (
                    "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                    "token-123".to_owned(),
                ),
                (name.to_owned(), "cti".to_owned()),
            ]);

            let error = ServerConfig::from_map(&vars)
                .expect_err("a removed license variable must fail startup");

            assert_eq!(
                error,
                ConfigError::InvalidEnv {
                    name,
                    value: "license-gated modules are not part of Corrobore; remove this variable"
                        .to_owned(),
                }
            );
        }
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
    fn config_contract_rejects_zero_opencti_sync_limits() {
        let mut vars = HashMap::new();
        vars.insert(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        );
        vars.insert(
            "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS".to_owned(),
            "0".to_owned(),
        );

        let error =
            ServerConfig::from_map(&vars).expect_err("zero synchronization limit should fail");
        assert_eq!(
            error,
            ConfigError::InvalidEnv {
                name: "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS",
                value: "0".to_owned(),
            }
        );
    }

    #[test]
    fn config_contract_rejects_zero_rate_limits() {
        for name in [
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND",
            "CORROBORE_HTTP_RATE_LIMIT_BURST",
            "CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND",
            "CORROBORE_OPENCTI_RATE_LIMIT_BURST",
        ] {
            let vars = HashMap::from([
                (
                    "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                    "token-123".to_owned(),
                ),
                (name.to_owned(), "0".to_owned()),
            ]);
            assert!(
                matches!(
                    ServerConfig::from_map(&vars),
                    Err(ConfigError::InvalidEnv { name: invalid, .. }) if invalid == name
                ),
                "{name} should reject zero"
            );
        }
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
        assert_eq!(config.storage_max_hot_nodes, 16_384);
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

    #[test]
    fn config_contract_parses_and_validates_persistent_working_set_budgets() {
        let vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_STORAGE_MAX_HOT_NODES".to_owned(),
                "128".to_owned(),
            ),
            (
                "CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS".to_owned(),
                "256".to_owned(),
            ),
            (
                "CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES".to_owned(),
                "512".to_owned(),
            ),
        ]);
        let config = ServerConfig::from_map(&vars).expect("budgets should parse");
        assert_eq!(config.storage_max_hot_nodes, 128);
        assert_eq!(config.storage_max_hot_relationships, 256);
        assert_eq!(config.storage_max_warm_adjacency_entries, 512);

        let invalid = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            ("CORROBORE_STORAGE_MAX_HOT_NODES".to_owned(), "0".to_owned()),
        ]);
        assert!(matches!(
            ServerConfig::from_map(&invalid),
            Err(ConfigError::InvalidEnv {
                name: "CORROBORE_STORAGE_MAX_HOT_NODES",
                ..
            })
        ));
    }
}
