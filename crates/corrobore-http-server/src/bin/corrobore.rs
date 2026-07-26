// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Args, Parser, Subcommand};
use corrobore_http_server::{
    AppState, AppStateInitError, DataDirectoryOwnership, ServerConfig, ServerLifecycleError,
    build_router, install_shutdown_signal,
    logging::init_logging,
    s3_snapshot_store::{S3SnapshotArtifactStore, S3SnapshotStoreConfig},
    security::{OperationalEndpointPolicy, TlsMaterialPaths, load_tls_material},
    serve_tls_with_lifecycle, serve_with_lifecycle,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalStoreOptions, MigrationRequest, SnapshotRequest,
    cancel_derived_index_rebuild, create_consistent_snapshot, export_snapshot_to_store,
    migrate_storage, open_storage_root, rebuild_derived_indexes, restore_consistent_snapshot,
    rollback_storage_migration, validate_consistent_snapshot,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::info;

const CONFIG_EXIT_CODE: u8 = 2;
const STARTUP_EXIT_CODE: u8 = 3;
const OWNERSHIP_CONFLICT_EXIT_CODE: u8 = 4;
const STORAGE_INCOMPATIBLE_EXIT_CODE: u8 = 5;
const STORAGE_RECOVERY_EXIT_CODE: u8 = 6;
const FORCED_SHUTDOWN_EXIT_CODE: u8 = 7;
const STATUS_UNAVAILABLE_EXIT_CODE: u8 = 8;
const STATUS_INCOMPATIBLE_EXIT_CODE: u8 = 9;
const DATABASE_OPERATION_EXIT_CODE: u8 = 10;

#[derive(Parser)]
#[command(name = "corrobore", about = "Operate a Corrobore standalone server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Operate the standalone server.
    Server(ServerArgs),
}

#[derive(Args)]
struct ServerArgs {
    #[command(subcommand)]
    command: ServerCommand,
}

#[derive(Subcommand)]
enum ServerCommand {
    /// Start the server in the foreground.
    Start(ConfigArgs),
    /// Validate the effective configuration without side effects.
    ValidateConfig(ValidateConfigArgs),
    /// Print reproducible build metadata.
    Version,
    /// Probe the configured operational endpoints with a bounded timeout.
    Status(ConfigArgs),
    /// Create a coherent offline snapshot under exclusive storage ownership.
    Snapshot(SnapshotArgs),
    /// Validate a local snapshot and all component checksums.
    ValidateSnapshot(ValidateSnapshotArgs),
    /// Export a validated local snapshot to an S3 or MinIO bucket.
    ExportSnapshotS3(ExportSnapshotS3Args),
    /// Restore a validated snapshot into a new empty data directory.
    Restore(RestoreArgs),
    /// Run or resume the supported previous-version storage migration.
    Migrate(MigrationArgs),
    /// Roll back the compatible manifest boundary after a completed migration.
    Rollback(MigrationRootArgs),
    /// Rebuild every derived index from canonical data.
    RebuildIndexes(MigrationRootArgs),
    /// Cancel an incomplete derived-index rebuild at its durable boundary.
    CancelRebuild(MigrationRootArgs),
}

#[derive(Args)]
struct SnapshotArgs {
    /// Persistent source data directory.
    #[arg(long)]
    storage_dir: PathBuf,
    /// New local snapshot artifact directory.
    #[arg(long)]
    destination: PathBuf,
    /// Optional key-provider identity; key material is never stored.
    #[arg(long)]
    encryption_key_id: Option<String>,
    /// Optional provider lifecycle/retention hook.
    #[arg(long)]
    retention_hook: Option<String>,
}

#[derive(Args)]
struct ValidateSnapshotArgs {
    /// Local snapshot artifact directory.
    #[arg(long)]
    snapshot: PathBuf,
    /// Expected key-provider identity.
    #[arg(long)]
    encryption_key_id: Option<String>,
}

#[derive(Args)]
struct ExportSnapshotS3Args {
    /// Validated local snapshot artifact directory.
    #[arg(long)]
    snapshot: PathBuf,
    /// S3 or MinIO endpoint.
    #[arg(long)]
    endpoint: String,
    /// Destination bucket.
    #[arg(long)]
    bucket: String,
    /// Destination object prefix.
    #[arg(long)]
    prefix: String,
    /// AWS signing region.
    #[arg(long, default_value = "us-east-1")]
    region: String,
}

#[derive(Args)]
struct RestoreArgs {
    /// Local snapshot artifact directory.
    #[arg(long)]
    snapshot: PathBuf,
    /// New empty target data directory.
    #[arg(long)]
    target: PathBuf,
    /// Expected key-provider identity.
    #[arg(long)]
    encryption_key_id: Option<String>,
}

#[derive(Args)]
struct MigrationArgs {
    /// Offline persistent data directory.
    #[arg(long)]
    storage_dir: PathBuf,
    /// Previous storage version. Only V0 is currently supported.
    #[arg(long, default_value = "V0")]
    from: String,
    /// Target storage version. Only V1 is currently supported.
    #[arg(long, default_value = "V1")]
    to: String,
}

#[derive(Args)]
struct MigrationRootArgs {
    /// Offline persistent data directory.
    #[arg(long)]
    storage_dir: PathBuf,
}

#[derive(Args)]
struct ConfigArgs {
    /// Optional TOML configuration file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override the configured bind host.
    #[arg(long)]
    host: Option<String>,
    /// Override the configured bind port.
    #[arg(long)]
    port: Option<u16>,
    /// Override the configured bearer token.
    #[arg(long)]
    auth_token: Option<String>,
    /// Override the authentication mode (`required` or `local-insecure`).
    #[arg(long)]
    auth_mode: Option<String>,
    /// Load the configured bearer token from a file.
    #[arg(long)]
    auth_token_file: Option<PathBuf>,
    /// Override the configured administrative bearer token.
    #[arg(long)]
    admin_auth_token: Option<String>,
    /// Load the configured administrative bearer token from a file.
    #[arg(long)]
    admin_auth_token_file: Option<PathBuf>,
    /// Configure operational endpoints as `public` or `authenticated`.
    #[arg(long)]
    operational_endpoint_policy: Option<String>,
    /// Override the runtime data directory.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Override graph persistence mode (`ephemeral` or `persistent`).
    #[arg(long)]
    storage_mode: Option<String>,
    /// Override the persistent graph directory.
    #[arg(long)]
    storage_dir: Option<PathBuf>,
    /// Override whether persistent writes require fsync.
    #[arg(long, action = clap::ArgAction::Set)]
    storage_require_fsync: Option<bool>,
    /// Override whether persistent recovery is strict.
    #[arg(long, action = clap::ArgAction::Set)]
    storage_strict_recovery: Option<bool>,
    /// Override the maximum resident hot node payloads.
    #[arg(long)]
    storage_max_hot_nodes: Option<u64>,
    /// Override the maximum resident hot relationship payloads.
    #[arg(long)]
    storage_max_hot_relationships: Option<u64>,
    /// Override the maximum resident warm adjacency entries.
    #[arg(long)]
    storage_max_warm_adjacency_entries: Option<u64>,
    /// Override the structured log directory.
    #[arg(long)]
    log_dir: Option<PathBuf>,
    /// Override the tracing filter or log level.
    #[arg(long)]
    log_level: Option<String>,
    /// Override the structured log format (`json`).
    #[arg(long)]
    log_format: Option<String>,
    /// Override the query and request timeout.
    #[arg(long)]
    query_timeout_ms: Option<u64>,
    /// Override the graceful shutdown budget.
    #[arg(long)]
    shutdown_timeout_ms: Option<u64>,
    /// Override the standard request body limit.
    #[arg(long)]
    max_body_bytes: Option<usize>,
    /// Override the import request body limit.
    #[arg(long)]
    import_max_body_bytes: Option<usize>,
    /// Override the maximum OpenCTI mutations accepted per synchronization batch.
    #[arg(long)]
    opencti_sync_max_operations: Option<usize>,
    /// Override the bounded replay and dead-letter retention.
    #[arg(long)]
    opencti_sync_max_replay_identities: Option<usize>,
    /// Override the sustained request rate.
    #[arg(long)]
    rate_limit_per_second: Option<u64>,
    /// Override the request burst allowance.
    #[arg(long)]
    rate_limit_burst: Option<u32>,
    /// Override enabled interfaces as a comma-separated list.
    #[arg(long, value_delimiter = ',')]
    interfaces: Vec<String>,
    /// Override the directory containing the web interface build.
    #[arg(long)]
    web_dir: Option<PathBuf>,
    /// Override whether maintenance tasks are enabled.
    #[arg(long, action = clap::ArgAction::Set)]
    maintenance_enabled: Option<bool>,
    /// Override the maintenance interval.
    #[arg(long)]
    maintenance_interval_ms: Option<u64>,
    /// Override whether TLS is enabled.
    #[arg(long, action = clap::ArgAction::Set)]
    tls_enabled: Option<bool>,
    /// Override the TLS certificate path.
    #[arg(long)]
    tls_certificate_file: Option<PathBuf>,
    /// Override the TLS private-key path.
    #[arg(long)]
    tls_private_key_file: Option<PathBuf>,
}

#[derive(Args)]
struct ValidateConfigArgs {
    #[command(flatten)]
    config: ConfigArgs,
    /// Print the effective non-secret configuration.
    #[arg(long)]
    print_effective: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    server: FileServer,
    #[serde(default)]
    storage: FileStorage,
    #[serde(default)]
    logging: FileLogging,
    #[serde(default)]
    limits: FileLimits,
    #[serde(default)]
    interfaces: FileInterfaces,
    #[serde(default)]
    maintenance: FileMaintenance,
    #[serde(default)]
    operations: FileOperations,
    #[serde(default)]
    tls: FileTls,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileServer {
    host: Option<String>,
    port: Option<u16>,
    auth_mode: Option<String>,
    auth_token: Option<String>,
    auth_token_file: Option<String>,
    admin_auth_token: Option<String>,
    admin_auth_token_file: Option<String>,
    data_directory: Option<String>,
    shutdown_timeout_ms: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileStorage {
    mode: Option<String>,
    directory: Option<String>,
    require_fsync: Option<bool>,
    strict_recovery: Option<bool>,
    max_hot_nodes: Option<u64>,
    max_hot_relationships: Option<u64>,
    max_warm_adjacency_entries: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLogging {
    directory: Option<String>,
    level: Option<String>,
    format: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLimits {
    #[serde(alias = "query_timeout_ms")]
    request_timeout_ms: Option<u64>,
    max_body_bytes: Option<usize>,
    import_max_body_bytes: Option<usize>,
    opencti_sync_max_operations: Option<usize>,
    opencti_sync_max_replay_identities: Option<usize>,
    rate_limit_per_second: Option<u64>,
    rate_limit_burst: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInterfaces {
    #[serde(default = "default_interfaces")]
    enabled: Vec<String>,
    web_directory: Option<String>,
}

impl Default for FileInterfaces {
    fn default() -> Self {
        Self {
            enabled: default_interfaces(),
            web_directory: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileMaintenance {
    enabled: bool,
    interval_ms: u64,
}

impl Default for FileMaintenance {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_ms: 60_000,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTls {
    enabled: bool,
    certificate_file: Option<String>,
    private_key_file: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOperations {
    endpoint_policy: Option<String>,
}

struct OperationalConfig {
    server: ServerConfig,
    log_level: String,
    log_format: String,
    interfaces: Vec<String>,
    maintenance: FileMaintenance,
    tls: FileTls,
}

#[derive(Deserialize)]
struct StatusReadinessResponse {
    ready: bool,
    lifecycle_state: String,
}

#[derive(Deserialize)]
struct StatusVersionResponse {
    version: String,
    storage_compatibility: StatusStorageCompatibility,
}

#[derive(Deserialize)]
struct StatusStorageCompatibility {
    supported_versions: Vec<String>,
    supported_record_formats: Vec<String>,
    active_storage_version: Option<String>,
    active_record_format: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Server(ServerArgs {
            command: ServerCommand::Start(args),
        }) => match load_config(&args) {
            Ok(config) => match start_server(config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => startup_failure(error.as_ref()),
            },
            Err(error) => config_failure(error),
        },
        Command::Server(ServerArgs {
            command: ServerCommand::ValidateConfig(args),
        }) => match load_config(&args.config) {
            Ok(config) => {
                if args.print_effective {
                    print_effective(&config);
                } else {
                    println!("configuration is valid");
                }
                ExitCode::SUCCESS
            }
            Err(error) => config_failure(error),
        },
        Command::Server(ServerArgs {
            command: ServerCommand::Version,
        }) => {
            println!(
                "corrobore version={} target={}-{} revision={}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::ARCH,
                std::env::consts::OS,
                option_env!("CORROBORE_BUILD_REVISION").unwrap_or("unknown")
            );
            ExitCode::SUCCESS
        }
        Command::Server(ServerArgs {
            command: ServerCommand::Status(args),
        }) => match load_config(&args) {
            Ok(config) => status_probe(config).await,
            Err(error) => config_failure(error),
        },
        Command::Server(ServerArgs {
            command: ServerCommand::Snapshot(args),
        }) => run_snapshot(args),
        Command::Server(ServerArgs {
            command: ServerCommand::ValidateSnapshot(args),
        }) => run_snapshot_validation(args),
        Command::Server(ServerArgs {
            command: ServerCommand::ExportSnapshotS3(args),
        }) => run_snapshot_s3_export(args),
        Command::Server(ServerArgs {
            command: ServerCommand::Restore(args),
        }) => run_restore(args),
        Command::Server(ServerArgs {
            command: ServerCommand::Migrate(args),
        }) => run_migration(args),
        Command::Server(ServerArgs {
            command: ServerCommand::Rollback(args),
        }) => run_rollback(args),
        Command::Server(ServerArgs {
            command: ServerCommand::RebuildIndexes(args),
        }) => run_index_rebuild(args),
        Command::Server(ServerArgs {
            command: ServerCommand::CancelRebuild(args),
        }) => run_index_rebuild_cancellation(args),
    }
}

fn run_snapshot(args: SnapshotArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.storage_dir) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("snapshot", started, error),
    };
    let root = match open_storage_root(&args.storage_dir) {
        Ok(root) => root,
        Err(error) => return database_operation_failure("snapshot", started, error),
    };
    let result = create_consistent_snapshot(
        &root,
        &args.destination,
        SnapshotRequest {
            created_at: chrono::Utc::now().to_rfc3339(),
            encryption_key_id: args.encryption_key_id,
            retention_hook: args.retention_hook,
        },
    );
    drop(ownership);
    print_database_operation("snapshot", started, result)
}

fn run_snapshot_validation(args: ValidateSnapshotArgs) -> ExitCode {
    let started = std::time::Instant::now();
    print_database_operation(
        "validate_snapshot",
        started,
        validate_consistent_snapshot(args.snapshot, args.encryption_key_id.as_deref()),
    )
}

fn run_snapshot_s3_export(args: ExportSnapshotS3Args) -> ExitCode {
    let started = std::time::Instant::now();
    let access_key = match std::env::var("CORROBORE_S3_ACCESS_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            return database_operation_failure(
                "export_snapshot_s3",
                started,
                "CORROBORE_S3_ACCESS_KEY is required",
            );
        }
    };
    let secret_key = match std::env::var("CORROBORE_S3_SECRET_KEY") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            return database_operation_failure(
                "export_snapshot_s3",
                started,
                "CORROBORE_S3_SECRET_KEY is required",
            );
        }
    };
    let session_token = std::env::var("CORROBORE_S3_SESSION_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let mut store = match S3SnapshotArtifactStore::new(S3SnapshotStoreConfig {
        endpoint: args.endpoint,
        bucket: args.bucket,
        region: args.region,
        access_key,
        secret_key,
        session_token,
    }) {
        Ok(store) => store,
        Err(error) => return database_operation_failure("export_snapshot_s3", started, error),
    };
    print_database_operation(
        "export_snapshot_s3",
        started,
        export_snapshot_to_store(args.snapshot, &mut store, &args.prefix),
    )
}

fn run_restore(args: RestoreArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.target) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("restore", started, error),
    };
    let result = restore_consistent_snapshot(
        args.snapshot,
        &args.target,
        args.encryption_key_id.as_deref(),
    );
    drop(ownership);
    print_database_operation("restore", started, result)
}

fn run_migration(args: MigrationArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.storage_dir) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("migrate", started, error),
    };
    let result = migrate_storage(
        &args.storage_dir,
        MigrationRequest {
            source_version: args.from,
            target_version: args.to,
            started_at: chrono::Utc::now().to_rfc3339(),
        },
        None,
    );
    drop(ownership);
    print_database_operation("migrate", started, result)
}

fn run_rollback(args: MigrationRootArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.storage_dir) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("rollback", started, error),
    };
    let result = rollback_storage_migration(&args.storage_dir);
    drop(ownership);
    print_database_operation("rollback", started, result)
}

fn run_index_rebuild(args: MigrationRootArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.storage_dir) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("rebuild_indexes", started, error),
    };
    let result = open_storage_root(&args.storage_dir)
        .and_then(|root| CanonicalEngineStore::open(root, CanonicalStoreOptions::default()))
        .and_then(|mut store| rebuild_derived_indexes(&mut store, None));
    drop(ownership);
    print_database_operation("rebuild_indexes", started, result)
}

fn run_index_rebuild_cancellation(args: MigrationRootArgs) -> ExitCode {
    let started = std::time::Instant::now();
    let ownership = match DataDirectoryOwnership::acquire(&args.storage_dir) {
        Ok(ownership) => ownership,
        Err(error) => return database_operation_failure("cancel_rebuild", started, error),
    };
    let result = cancel_derived_index_rebuild(&args.storage_dir);
    drop(ownership);
    print_database_operation("cancel_rebuild", started, result)
}

fn print_database_operation<T: serde::Serialize, E: std::fmt::Display>(
    operation: &'static str,
    started: std::time::Instant,
    result: Result<T, E>,
) -> ExitCode {
    match result {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(report) => {
                println!("{report}");
                eprintln!(
                    "database operation completed: operation={operation} duration_ms={}",
                    started.elapsed().as_millis()
                );
                ExitCode::SUCCESS
            }
            Err(error) => database_operation_failure(operation, started, error),
        },
        Err(error) => database_operation_failure(operation, started, error),
    }
}

fn database_operation_failure(
    operation: &'static str,
    started: std::time::Instant,
    error: impl std::fmt::Display,
) -> ExitCode {
    eprintln!(
        "database operation failed: operation={operation} duration_ms={} error={error}",
        started.elapsed().as_millis()
    );
    ExitCode::from(DATABASE_OPERATION_EXIT_CODE)
}

/// Probe readiness and version compatibility, returning stable operational
/// exit codes for automation.
async fn status_probe(config: OperationalConfig) -> ExitCode {
    let timeout = std::time::Duration::from_millis(config.server.request_timeout_ms);
    match tokio::time::timeout(timeout, probe_operational_endpoints(&config)).await {
        Ok(Ok(version)) => {
            println!("server status=ready version={version}");
            ExitCode::SUCCESS
        }
        Ok(Err(StatusProbeError::Incompatible(reason))) => {
            eprintln!("server status=incompatible: {reason}");
            ExitCode::from(STATUS_INCOMPATIBLE_EXIT_CODE)
        }
        Ok(Err(StatusProbeError::Unavailable(reason))) => {
            eprintln!("server status=unavailable: {reason}");
            ExitCode::from(STATUS_UNAVAILABLE_EXIT_CODE)
        }
        Err(_) => {
            eprintln!(
                "server status=unavailable: probe timed out after {} ms",
                config.server.request_timeout_ms
            );
            ExitCode::from(STATUS_UNAVAILABLE_EXIT_CODE)
        }
    }
}

enum StatusProbeError {
    Unavailable(String),
    Incompatible(String),
}

async fn probe_operational_endpoints(
    config: &OperationalConfig,
) -> Result<String, StatusProbeError> {
    let host = match config.server.host.as_str() {
        "0.0.0.0" => "127.0.0.1".to_owned(),
        "::" => "[::1]".to_owned(),
        host if host.contains(':') && !host.starts_with('[') => format!("[{host}]"),
        host => host.to_owned(),
    };
    let scheme = if config.tls.enabled { "https" } else { "http" };
    let base_url = format!("{scheme}://{host}:{}", config.server.port);
    let mut client_builder = reqwest::Client::builder().timeout(std::time::Duration::from_millis(
        config.server.request_timeout_ms,
    ));
    if config.tls.enabled {
        let certificate_path = config.tls.certificate_file.as_ref().ok_or_else(|| {
            StatusProbeError::Unavailable("TLS certificate is missing".to_owned())
        })?;
        let certificate_pem = fs::read(certificate_path).map_err(|_| {
            StatusProbeError::Unavailable("TLS certificate cannot be read".to_owned())
        })?;
        let certificate = reqwest::Certificate::from_pem(&certificate_pem).map_err(|_| {
            StatusProbeError::Unavailable("TLS certificate cannot be parsed".to_owned())
        })?;
        client_builder = client_builder.add_root_certificate(certificate);
    }
    let client = client_builder
        .build()
        .map_err(|error| StatusProbeError::Unavailable(error.to_string()))?;

    let mut readiness_request = client.get(format!("{base_url}/health/ready"));
    if config.server.operational_endpoint_policy == OperationalEndpointPolicy::Authenticated
        && let Some(token) = config.server.auth_token.as_deref()
    {
        readiness_request = readiness_request.bearer_auth(token);
    }
    let readiness_response = readiness_request
        .send()
        .await
        .map_err(|error| StatusProbeError::Unavailable(format!("{error:?}")))?;
    if !readiness_response.status().is_success() {
        return Err(StatusProbeError::Unavailable(format!(
            "readiness endpoint returned HTTP {}",
            readiness_response.status().as_u16()
        )));
    }
    let readiness = readiness_response
        .json::<StatusReadinessResponse>()
        .await
        .map_err(|error| {
            StatusProbeError::Incompatible(format!(
                "readiness endpoint returned an invalid contract: {error}"
            ))
        })?;
    if !readiness.ready {
        return Err(StatusProbeError::Unavailable(format!(
            "server lifecycle is {}",
            readiness.lifecycle_state
        )));
    }

    let mut version_request = client.get(format!("{base_url}/version"));
    if config.server.operational_endpoint_policy == OperationalEndpointPolicy::Authenticated
        && let Some(token) = config.server.auth_token.as_deref()
    {
        version_request = version_request.bearer_auth(token);
    }
    let version_response = version_request
        .send()
        .await
        .map_err(|error| StatusProbeError::Unavailable(format!("{error:?}")))?;
    if !version_response.status().is_success() {
        return Err(StatusProbeError::Unavailable(format!(
            "version endpoint returned HTTP {}",
            version_response.status().as_u16()
        )));
    }
    let version = version_response
        .json::<StatusVersionResponse>()
        .await
        .map_err(|error| {
            StatusProbeError::Incompatible(format!(
                "version endpoint returned an invalid contract: {error}"
            ))
        })?;
    validate_status_compatibility(&version.storage_compatibility)?;
    Ok(version.version)
}

fn validate_status_compatibility(
    compatibility: &StatusStorageCompatibility,
) -> Result<(), StatusProbeError> {
    let supports_version = compatibility
        .supported_versions
        .iter()
        .any(|version| version == "V1");
    let supports_record_format = compatibility
        .supported_record_formats
        .iter()
        .any(|format| format == "JsonLinesV1");
    let active_version_compatible = compatibility
        .active_storage_version
        .as_deref()
        .is_none_or(|version| version == "V1");
    let active_format_compatible = compatibility
        .active_record_format
        .as_deref()
        .is_none_or(|format| format == "JsonLinesV1");
    if supports_version
        && supports_record_format
        && active_version_compatible
        && active_format_compatible
    {
        Ok(())
    } else {
        Err(StatusProbeError::Incompatible(format!(
            "storage versions {:?}, record formats {:?}, active version {:?}, active format {:?}",
            compatibility.supported_versions,
            compatibility.supported_record_formats,
            compatibility.active_storage_version,
            compatibility.active_record_format,
        )))
    }
}

fn load_config(args: &ConfigArgs) -> Result<OperationalConfig, String> {
    let mut values = HashMap::new();
    if let Some(path) = &args.config {
        apply_file(path, &mut values)?;
    }
    let environment = std::env::vars()
        .filter(|(key, _)| key.starts_with("CORROBORE_"))
        .collect::<HashMap<_, _>>();
    reconcile_secret_precedence(
        &mut values,
        &environment,
        "CORROBORE_HTTP_AUTH_TOKEN",
        "CORROBORE_HTTP_AUTH_TOKEN_FILE",
    );
    reconcile_secret_precedence(
        &mut values,
        &environment,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE",
    );
    for (key, value) in environment {
        values.insert(key, value);
    }
    if !values.contains_key("CORROBORE_LOG_LEVEL")
        && let Ok(value) = std::env::var("RUST_LOG")
    {
        values.insert("CORROBORE_LOG_LEVEL".to_owned(), value);
    }
    apply_cli(args, &mut values);

    let server = ServerConfig::from_map(&values).map_err(redact_config_error)?;
    if server.port == 0 {
        return Err("server.port: must be greater than zero".to_owned());
    }
    let operational = parse_operational(server, &values)?;
    validate_operational(&operational)?;
    Ok(operational)
}

fn apply_file(path: &Path, values: &mut HashMap<String, String>) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("config.file: cannot read {}: {error}", path.display()))?;
    let config: FileConfig =
        toml::from_str(&source).map_err(|error| safe_toml_diagnostic(path, &source, &error))?;

    insert(values, "CORROBORE_HTTP_HOST", config.server.host.clone());
    insert(
        values,
        "CORROBORE_HTTP_PORT",
        config.server.port.map(|value| value.to_string()),
    );
    insert(
        values,
        "CORROBORE_HTTP_AUTH_TOKEN",
        config.server.auth_token.clone(),
    );
    insert(
        values,
        "CORROBORE_HTTP_AUTH_MODE",
        config.server.auth_mode.clone(),
    );
    insert(
        values,
        "CORROBORE_HTTP_AUTH_TOKEN_FILE",
        config.server.auth_token_file.clone(),
    );
    insert(
        values,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        config.server.admin_auth_token.clone(),
    );
    insert(
        values,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE",
        config.server.admin_auth_token_file.clone(),
    );
    insert(
        values,
        "CORROBORE_HTTP_SESSION_STORE_DIR",
        config.server.data_directory.clone(),
    );
    insert_num(
        values,
        "CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS",
        config.server.shutdown_timeout_ms,
    );
    insert(
        values,
        "CORROBORE_STORAGE_MODE",
        config.storage.mode.clone(),
    );
    insert(
        values,
        "CORROBORE_STORAGE_DIR",
        config.storage.directory.clone(),
    );
    insert_bool(
        values,
        "CORROBORE_STORAGE_REQUIRE_FSYNC",
        config.storage.require_fsync,
    );
    insert_bool(
        values,
        "CORROBORE_STORAGE_STRICT_RECOVERY",
        config.storage.strict_recovery,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_HOT_NODES",
        config.storage.max_hot_nodes,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS",
        config.storage.max_hot_relationships,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES",
        config.storage.max_warm_adjacency_entries,
    );
    insert(
        values,
        "CORROBORE_HTTP_LOG_DIR",
        config.logging.directory.clone(),
    );
    insert(values, "CORROBORE_LOG_LEVEL", config.logging.level.clone());
    insert(
        values,
        "CORROBORE_LOG_FORMAT",
        config.logging.format.clone(),
    );
    insert_num(
        values,
        "CORROBORE_HTTP_REQUEST_TIMEOUT_MS",
        config.limits.request_timeout_ms,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_MAX_BODY_BYTES",
        config.limits.max_body_bytes,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES",
        config.limits.import_max_body_bytes,
    );
    insert_num(
        values,
        "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS",
        config.limits.opencti_sync_max_operations,
    );
    insert_num(
        values,
        "CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES",
        config.limits.opencti_sync_max_replay_identities,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND",
        config.limits.rate_limit_per_second,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_RATE_LIMIT_BURST",
        config.limits.rate_limit_burst,
    );
    insert(
        values,
        "CORROBORE_HTTP_WEB_DIR",
        config.interfaces.web_directory.clone(),
    );
    insert(
        values,
        "CORROBORE_SERVER_INTERFACES",
        Some(config.interfaces.enabled.join(",")),
    );
    insert_bool(
        values,
        "CORROBORE_MAINTENANCE_ENABLED",
        Some(config.maintenance.enabled),
    );
    insert_num(
        values,
        "CORROBORE_MAINTENANCE_INTERVAL_MS",
        Some(config.maintenance.interval_ms),
    );
    insert(
        values,
        "CORROBORE_OPERATIONAL_ENDPOINT_POLICY",
        config.operations.endpoint_policy,
    );
    insert_bool(values, "CORROBORE_TLS_ENABLED", Some(config.tls.enabled));
    insert(
        values,
        "CORROBORE_TLS_CERTIFICATE_FILE",
        config.tls.certificate_file,
    );
    insert(
        values,
        "CORROBORE_TLS_PRIVATE_KEY_FILE",
        config.tls.private_key_file,
    );
    Ok(())
}

fn safe_toml_diagnostic(path: &Path, source: &str, error: &toml::de::Error) -> String {
    let location = error.span().map_or_else(String::new, |span| {
        let prefix = source
            .get(..span.start.min(source.len()))
            .unwrap_or_default();
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix.len() + 1, |(_, tail)| tail.len() + 1);
        format!(" at line {line}, column {column}")
    });
    format!(
        "config.file: invalid TOML in {}{location}: {}",
        path.display(),
        error.message()
    )
}

fn apply_cli(args: &ConfigArgs, values: &mut HashMap<String, String>) {
    reconcile_cli_secret_precedence(
        values,
        args.auth_token.is_some(),
        args.auth_token_file.is_some(),
        "CORROBORE_HTTP_AUTH_TOKEN",
        "CORROBORE_HTTP_AUTH_TOKEN_FILE",
    );
    reconcile_cli_secret_precedence(
        values,
        args.admin_auth_token.is_some(),
        args.admin_auth_token_file.is_some(),
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE",
    );
    insert(values, "CORROBORE_HTTP_HOST", args.host.clone());
    insert_num(values, "CORROBORE_HTTP_PORT", args.port);
    insert(values, "CORROBORE_HTTP_AUTH_TOKEN", args.auth_token.clone());
    insert(values, "CORROBORE_HTTP_AUTH_MODE", args.auth_mode.clone());
    insert_path(
        values,
        "CORROBORE_HTTP_AUTH_TOKEN_FILE",
        args.auth_token_file.as_deref(),
    );
    insert(
        values,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        args.admin_auth_token.clone(),
    );
    insert_path(
        values,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE",
        args.admin_auth_token_file.as_deref(),
    );
    insert(
        values,
        "CORROBORE_OPERATIONAL_ENDPOINT_POLICY",
        args.operational_endpoint_policy.clone(),
    );
    insert_path(
        values,
        "CORROBORE_HTTP_SESSION_STORE_DIR",
        args.data_dir.as_deref(),
    );
    insert(values, "CORROBORE_STORAGE_MODE", args.storage_mode.clone());
    insert_path(values, "CORROBORE_STORAGE_DIR", args.storage_dir.as_deref());
    insert_bool(
        values,
        "CORROBORE_STORAGE_REQUIRE_FSYNC",
        args.storage_require_fsync,
    );
    insert_bool(
        values,
        "CORROBORE_STORAGE_STRICT_RECOVERY",
        args.storage_strict_recovery,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_HOT_NODES",
        args.storage_max_hot_nodes,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS",
        args.storage_max_hot_relationships,
    );
    insert_num(
        values,
        "CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES",
        args.storage_max_warm_adjacency_entries,
    );
    insert_path(values, "CORROBORE_HTTP_LOG_DIR", args.log_dir.as_deref());
    insert(values, "CORROBORE_LOG_LEVEL", args.log_level.clone());
    insert(values, "CORROBORE_LOG_FORMAT", args.log_format.clone());
    insert_num(
        values,
        "CORROBORE_HTTP_REQUEST_TIMEOUT_MS",
        args.query_timeout_ms,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS",
        args.shutdown_timeout_ms,
    );
    insert_num(values, "CORROBORE_HTTP_MAX_BODY_BYTES", args.max_body_bytes);
    insert_num(
        values,
        "CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES",
        args.import_max_body_bytes,
    );
    insert_num(
        values,
        "CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS",
        args.opencti_sync_max_operations,
    );
    insert_num(
        values,
        "CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES",
        args.opencti_sync_max_replay_identities,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND",
        args.rate_limit_per_second,
    );
    insert_num(
        values,
        "CORROBORE_HTTP_RATE_LIMIT_BURST",
        args.rate_limit_burst,
    );
    if !args.interfaces.is_empty() {
        insert(
            values,
            "CORROBORE_SERVER_INTERFACES",
            Some(args.interfaces.join(",")),
        );
    }
    insert_path(values, "CORROBORE_HTTP_WEB_DIR", args.web_dir.as_deref());
    insert_bool(
        values,
        "CORROBORE_MAINTENANCE_ENABLED",
        args.maintenance_enabled,
    );
    insert_num(
        values,
        "CORROBORE_MAINTENANCE_INTERVAL_MS",
        args.maintenance_interval_ms,
    );
    insert_bool(values, "CORROBORE_TLS_ENABLED", args.tls_enabled);
    insert_path(
        values,
        "CORROBORE_TLS_CERTIFICATE_FILE",
        args.tls_certificate_file.as_deref(),
    );
    insert_path(
        values,
        "CORROBORE_TLS_PRIVATE_KEY_FILE",
        args.tls_private_key_file.as_deref(),
    );
}

fn reconcile_secret_precedence(
    lower: &mut HashMap<String, String>,
    higher: &HashMap<String, String>,
    inline_key: &str,
    file_key: &str,
) {
    match (
        higher.contains_key(inline_key),
        higher.contains_key(file_key),
    ) {
        (true, false) => {
            lower.remove(file_key);
        }
        (false, true) => {
            lower.remove(inline_key);
        }
        _ => {}
    }
}

fn reconcile_cli_secret_precedence(
    lower: &mut HashMap<String, String>,
    has_inline: bool,
    has_file: bool,
    inline_key: &str,
    file_key: &str,
) {
    match (has_inline, has_file) {
        (true, false) => {
            lower.remove(file_key);
        }
        (false, true) => {
            lower.remove(inline_key);
        }
        _ => {}
    }
}

fn insert_path(values: &mut HashMap<String, String>, key: &str, value: Option<&Path>) {
    insert(
        values,
        key,
        value.map(|path| path.to_string_lossy().into_owned()),
    );
}

fn insert(values: &mut HashMap<String, String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        values.insert(key.to_owned(), value);
    }
}

fn insert_num<T: ToString>(values: &mut HashMap<String, String>, key: &str, value: Option<T>) {
    insert(values, key, value.map(|value| value.to_string()));
}

fn insert_bool(values: &mut HashMap<String, String>, key: &str, value: Option<bool>) {
    insert_num(values, key, value);
}

fn default_interfaces() -> Vec<String> {
    vec!["http".to_owned()]
}

fn parse_operational(
    server: ServerConfig,
    values: &HashMap<String, String>,
) -> Result<OperationalConfig, String> {
    let interfaces = values
        .get("CORROBORE_SERVER_INTERFACES")
        .map_or("http", String::as_str)
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    let maintenance = FileMaintenance {
        enabled: parse_bool_value(values, "CORROBORE_MAINTENANCE_ENABLED", false)?,
        interval_ms: parse_u64_value(values, "CORROBORE_MAINTENANCE_INTERVAL_MS", 60_000)?,
    };
    let tls = FileTls {
        enabled: parse_bool_value(values, "CORROBORE_TLS_ENABLED", false)?,
        certificate_file: nonempty(values.get("CORROBORE_TLS_CERTIFICATE_FILE")),
        private_key_file: nonempty(values.get("CORROBORE_TLS_PRIVATE_KEY_FILE")),
    };
    let log_level = values
        .get("CORROBORE_LOG_LEVEL")
        .map_or("info", String::as_str)
        .trim()
        .to_owned();
    let log_format = values
        .get("CORROBORE_LOG_FORMAT")
        .map_or("json", String::as_str)
        .trim()
        .to_ascii_lowercase();
    Ok(OperationalConfig {
        server,
        log_level,
        log_format,
        interfaces,
        maintenance,
        tls,
    })
}

fn parse_bool_value(
    values: &HashMap<String, String>,
    name: &'static str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = values.get(name) else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "{}: expected a boolean value",
            operational_field(name)
        )),
    }
}

fn parse_u64_value(
    values: &HashMap<String, String>,
    name: &'static str,
    default: u64,
) -> Result<u64, String> {
    values.get(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("{}: expected an unsigned integer", operational_field(name)))
    })
}

fn operational_field(name: &str) -> &str {
    match name {
        "CORROBORE_MAINTENANCE_ENABLED" => "maintenance.enabled",
        "CORROBORE_MAINTENANCE_INTERVAL_MS" => "maintenance.interval_ms",
        "CORROBORE_TLS_ENABLED" => "tls.enabled",
        _ => "configuration",
    }
}

fn nonempty(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn validate_operational(config: &OperationalConfig) -> Result<(), String> {
    if config.log_level.is_empty() {
        return Err("logging.level: must not be empty".to_owned());
    }
    if config.log_format != "json" {
        return Err("logging.format: only `json` is supported".to_owned());
    }
    if config.interfaces.is_empty() {
        return Err("interfaces.enabled: must contain at least one interface".to_owned());
    }
    for interface in &config.interfaces {
        if !matches!(interface.as_str(), "http" | "web") {
            return Err(format!(
                "interfaces.enabled: unsupported interface {interface:?}"
            ));
        }
    }
    if config.interfaces.iter().any(|interface| interface == "web")
        && config.server.web_dir.is_none()
    {
        return Err(
            "interfaces.web_directory: required when the web interface is enabled".to_owned(),
        );
    }
    if config.maintenance.enabled && config.maintenance.interval_ms == 0 {
        return Err("maintenance.interval_ms: must be greater than zero when enabled".to_owned());
    }
    if config.tls.enabled {
        if config.tls.certificate_file.is_none() {
            return Err("tls.certificate_file: required when TLS is enabled".to_owned());
        }
        if config.tls.private_key_file.is_none() {
            return Err("tls.private_key_file: required when TLS is enabled".to_owned());
        }
    }
    config
        .server
        .validate_network_exposure(config.tls.enabled)
        .map_err(str::to_owned)?;
    Ok(())
}

fn redact_config_error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    if message.contains("AUTH_TOKEN") {
        "server.auth_token: missing or invalid secret value".to_owned()
    } else {
        message
    }
}

fn config_failure(error: String) -> ExitCode {
    eprintln!("configuration error: {error}");
    ExitCode::from(CONFIG_EXIT_CODE)
}

fn startup_failure(error: &(dyn std::error::Error + 'static)) -> ExitCode {
    eprintln!("startup error: {error}");
    let code = if error.downcast_ref::<ServerLifecycleError>().is_some() {
        FORCED_SHUTDOWN_EXIT_CODE
    } else {
        error
            .downcast_ref::<AppStateInitError>()
            .map_or(STARTUP_EXIT_CODE, |error| match error {
                AppStateInitError::PersistentStorageOwnershipConflict { .. } => {
                    OWNERSHIP_CONFLICT_EXIT_CODE
                }
                AppStateInitError::PersistentStorageIncompatible { .. } => {
                    STORAGE_INCOMPATIBLE_EXIT_CODE
                }
                AppStateInitError::PersistentStorageRecoveryFailed { .. } => {
                    STORAGE_RECOVERY_EXIT_CODE
                }
                _ => STARTUP_EXIT_CODE,
            })
    };
    ExitCode::from(code)
}

fn print_effective(config: &OperationalConfig) {
    println!("server.host = {:?}", config.server.host);
    println!("server.port = {}", config.server.port);
    println!("server.auth_mode = {:?}", config.server.auth_mode.as_str());
    println!("server.auth_token = \"<redacted>\"");
    if let Some(source) = config.server.auth_token_source {
        println!("server.auth_token_source = {:?}", source.as_str());
    }
    println!("server.admin_auth_token = \"<redacted>\"");
    if let Some(source) = config.server.admin_auth_token_source {
        println!("server.admin_auth_token_source = {:?}", source.as_str());
    }
    println!(
        "operations.endpoint_policy = {:?}",
        config.server.operational_endpoint_policy.as_str()
    );
    println!(
        "server.shutdown_timeout_ms = {}",
        config.server.shutdown_timeout_ms
    );
    println!(
        "server.data_directory = {:?}",
        config.server.session_store_dir
    );
    println!("logging.directory = {:?}", config.server.log_dir);
    println!("logging.level = {:?}", config.log_level);
    println!("logging.format = {:?}", config.log_format);
    println!(
        "limits.request_timeout_ms = {}",
        config.server.request_timeout_ms
    );
    println!("limits.max_body_bytes = {}", config.server.max_body_bytes);
    println!(
        "limits.import_max_body_bytes = {}",
        config.server.import_max_body_bytes
    );
    println!(
        "limits.opencti_sync_max_operations = {}",
        config.server.opencti_sync_max_operations
    );
    println!(
        "limits.opencti_sync_max_replay_identities = {}",
        config.server.opencti_sync_max_replay_identities
    );
    println!(
        "limits.rate_limit_per_second = {}",
        config.server.rate_limit_per_second
    );
    println!(
        "limits.rate_limit_burst = {}",
        config.server.rate_limit_burst
    );
    println!("storage.mode = {:?}", config.server.storage_mode.as_str());
    if let Some(directory) = &config.server.storage_dir {
        println!("storage.directory = {directory:?}");
    }
    println!(
        "storage.require_fsync = {}",
        config.server.storage_require_fsync
    );
    println!(
        "storage.strict_recovery = {}",
        config.server.storage_strict_recovery
    );
    println!(
        "storage.max_hot_nodes = {}",
        config.server.storage_max_hot_nodes
    );
    println!(
        "storage.max_hot_relationships = {}",
        config.server.storage_max_hot_relationships
    );
    println!(
        "storage.max_warm_adjacency_entries = {}",
        config.server.storage_max_warm_adjacency_entries
    );
    println!("interfaces.enabled = {:?}", config.interfaces);
    if let Some(directory) = &config.server.web_dir {
        println!("interfaces.web_directory = {directory:?}");
    }
    println!("maintenance.enabled = {}", config.maintenance.enabled);
    println!(
        "maintenance.interval_ms = {}",
        config.maintenance.interval_ms
    );
    println!("tls.enabled = {}", config.tls.enabled);
    println!(
        "tls.certificate_file_configured = {}",
        config.tls.certificate_file.is_some()
    );
    println!(
        "tls.private_key_file_configured = {}",
        config.tls.private_key_file.is_some()
    );
}

async fn start_server(config: OperationalConfig) -> Result<(), Box<dyn std::error::Error>> {
    if !config
        .interfaces
        .iter()
        .any(|interface| interface == "http")
    {
        return Err(
            "interfaces.enabled: the HTTP interface is required to start the server".into(),
        );
    }
    let web_enabled = config.interfaces.iter().any(|interface| interface == "web");
    let tls = if config.tls.enabled {
        let paths = TlsMaterialPaths {
            certificate_file: PathBuf::from(
                config
                    .tls
                    .certificate_file
                    .as_ref()
                    .ok_or("tls.certificate_file is missing")?,
            ),
            private_key_file: PathBuf::from(
                config
                    .tls
                    .private_key_file
                    .as_ref()
                    .ok_or("tls.private_key_file is missing")?,
            ),
        };
        Some(load_tls_material(&paths).await?)
    } else {
        None
    };
    let mut server = config.server;
    if !web_enabled {
        server.web_dir = None;
    }
    let logging_runtime = init_logging(0, Some(&config.log_level), &server.log_dir)?;
    let _logging_guard = logging_runtime.guard;
    let addr: SocketAddr = format!("{}:{}", server.host, server.port).parse()?;
    let state = AppState::new(server)?;
    let shutdown_signal = install_shutdown_signal()?;
    info!(
        %addr,
        scheme = if tls.is_some() { "https" } else { "http" },
        maintenance_enabled = config.maintenance.enabled,
        maintenance_interval_ms = config.maintenance.interval_ms,
        "corrobore server listening"
    );
    if let Some(tls) = tls {
        serve_tls_with_lifecycle(
            addr,
            build_router(state.clone()),
            state,
            shutdown_signal,
            tls,
        )
        .await?;
    } else {
        let listener = TcpListener::bind(addr).await?;
        serve_with_lifecycle(
            listener,
            build_router(state.clone()),
            state,
            shutdown_signal,
        )
        .await?;
    }
    Ok(())
}
