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
use corrobore_http_server::{AppState, ServerConfig, build_router, logging::init_logging};
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::info;

const CONFIG_EXIT_CODE: u8 = 2;
const STARTUP_EXIT_CODE: u8 = 3;

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
    /// Override the configured administrative bearer token.
    #[arg(long)]
    admin_auth_token: Option<String>,
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
    tls: FileTls,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileServer {
    host: Option<String>,
    port: Option<u16>,
    auth_token: Option<String>,
    admin_auth_token: Option<String>,
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

struct OperationalConfig {
    server: ServerConfig,
    log_level: String,
    log_format: String,
    interfaces: Vec<String>,
    maintenance: FileMaintenance,
    tls: FileTls,
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
                Err(error) => {
                    eprintln!("startup error: {error}");
                    ExitCode::from(STARTUP_EXIT_CODE)
                }
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
    }
}

fn load_config(args: &ConfigArgs) -> Result<OperationalConfig, String> {
    let mut values = HashMap::new();
    if let Some(path) = &args.config {
        apply_file(path, &mut values)?;
    }
    for (key, value) in std::env::vars().filter(|(key, _)| key.starts_with("CORROBORE_")) {
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
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        config.server.admin_auth_token.clone(),
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
    insert(values, "CORROBORE_HTTP_HOST", args.host.clone());
    insert_num(values, "CORROBORE_HTTP_PORT", args.port);
    insert(values, "CORROBORE_HTTP_AUTH_TOKEN", args.auth_token.clone());
    insert(
        values,
        "CORROBORE_HTTP_ADMIN_AUTH_TOKEN",
        args.admin_auth_token.clone(),
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

fn print_effective(config: &OperationalConfig) {
    println!("server.host = {:?}", config.server.host);
    println!("server.port = {}", config.server.port);
    println!("server.auth_token = \"<redacted>\"");
    println!("server.admin_auth_token = \"<redacted>\"");
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
    if config.tls.enabled {
        return Err("TLS listener is not available in this release".into());
    }
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
    let mut server = config.server;
    if !web_enabled {
        server.web_dir = None;
    }
    let logging_runtime = init_logging(0, Some(&config.log_level), &server.log_dir)?;
    let _logging_guard = logging_runtime.guard;
    let addr: SocketAddr = format!("{}:{}", server.host, server.port).parse()?;
    let state = AppState::new(server)?;
    let listener = TcpListener::bind(addr).await?;
    info!(
        %addr,
        maintenance_enabled = config.maintenance.enabled,
        maintenance_interval_ms = config.maintenance.interval_ms,
        "corrobore server listening"
    );
    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
