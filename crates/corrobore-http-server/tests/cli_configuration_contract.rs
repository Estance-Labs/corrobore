// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use corrobore_http_server::ServerConfig;

const CONFIG_EXIT_CODE: i32 = 2;
const STARTUP_EXIT_CODE: i32 = 3;

fn corrobore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-{name}-{suffix}"));
    fs::create_dir_all(&path).expect("test directory should be created");
    path
}

fn write_config(directory: &Path, contents: &str) -> PathBuf {
    let path = directory.join("corrobore.toml");
    fs::write(&path, contents).expect("test configuration should be written");
    path
}

fn run(command: &mut Command) -> Output {
    command
        .env_clear()
        .output()
        .expect("corrobore command should execute")
}

#[test]
fn unified_cli_exposes_the_server_command_surface() {
    let output = run(corrobore().args(["server", "--help"]));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for command in [
        "start",
        "validate-config",
        "version",
        "status",
        "snapshot",
        "validate-snapshot",
        "export-snapshot-s3",
        "restore",
        "migrate",
        "rollback",
        "rebuild-indexes",
        "cancel-rebuild",
    ] {
        assert!(
            stdout.contains(command),
            "missing server command: {command}"
        );
    }
}

#[test]
fn status_is_bounded_and_reports_unavailable_with_a_stable_exit_code() {
    const STATUS_UNAVAILABLE_EXIT_CODE: i32 = 8;
    let reservation = TcpListener::bind("127.0.0.1:0").expect("test port should be reserved");
    let port = reservation.local_addr().expect("local address").port();
    drop(reservation);
    let started = std::time::Instant::now();

    let output = corrobore()
        .args([
            "server",
            "status",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-token",
            "status-secret",
            "--query-timeout-ms",
            "100",
        ])
        .env_clear()
        .output()
        .expect("status command should execute");

    assert_eq!(output.status.code(), Some(STATUS_UNAVAILABLE_EXIT_CODE));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "status probe must remain bounded"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("unavailable"));
}

fn spawn_operational_fixture(
    listener: TcpListener,
    version_payload: &'static str,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("fixture should accept a probe");
            let mut request = [0_u8; 2048];
            let size = stream
                .read(&mut request)
                .expect("fixture should read a probe");
            let request = String::from_utf8_lossy(&request[..size]);
            let body = if request.starts_with("GET /health/ready ") {
                r#"{"status":"ready","ready":true,"lifecycle_state":"ready"}"#
            } else if request.starts_with("GET /version ") {
                version_payload
            } else {
                panic!("unexpected operational probe: {request}");
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("fixture should write a response");
        }
    })
}

fn status_command(port: u16) -> Output {
    corrobore()
        .args([
            "server",
            "status",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-token",
            "status-secret",
            "--query-timeout-ms",
            "500",
        ])
        .env_clear()
        .output()
        .expect("status command should execute")
}

#[test]
fn status_reports_ready_when_operational_contract_is_compatible() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
    let port = listener.local_addr().expect("fixture address").port();
    let fixture = spawn_operational_fixture(
        listener,
        r#"{"service":"corrobore-http-server","version":"0.2.2","commit":"fixture","build_target":"test","storage_compatibility":{"supported_versions":["V1"],"supported_record_formats":["JsonLinesV1"],"active_storage_version":null,"active_record_format":null}}"#,
    );

    let output = status_command(port);
    fixture.join().expect("fixture should finish");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ready"));
    assert!(stdout.contains("version=0.2.2"));
}

#[test]
fn status_reports_incompatible_storage_with_a_stable_exit_code() {
    const STATUS_INCOMPATIBLE_EXIT_CODE: i32 = 9;
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
    let port = listener.local_addr().expect("fixture address").port();
    let fixture = spawn_operational_fixture(
        listener,
        r#"{"service":"corrobore-http-server","version":"9.0.0","commit":"fixture","build_target":"test","storage_compatibility":{"supported_versions":["V9"],"supported_record_formats":["BinaryV9"],"active_storage_version":"V9","active_record_format":"BinaryV9"}}"#,
    );

    let output = status_command(port);
    fixture.join().expect("fixture should finish");

    assert_eq!(output.status.code(), Some(STATUS_INCOMPATIBLE_EXIT_CODE));
    assert!(String::from_utf8_lossy(&output.stderr).contains("incompatible"));
}

#[test]
fn start_help_covers_the_operational_configuration_surface() {
    let output = run(corrobore().args(["server", "start", "--help"]));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for option in [
        "--host",
        "--port",
        "--auth-mode",
        "--auth-token-file",
        "--data-dir",
        "--storage-mode",
        "--storage-dir",
        "--log-dir",
        "--log-level",
        "--log-format",
        "--query-timeout-ms",
        "--shutdown-timeout-ms",
        "--max-body-bytes",
        "--opencti-sync-max-operations",
        "--opencti-sync-max-replay-identities",
        "--interfaces",
        "--web-dir",
        "--maintenance-enabled",
        "--tls-enabled",
        "--tls-certificate-file",
        "--tls-private-key-file",
        "--operational-endpoint-policy",
    ] {
        assert!(stdout.contains(option), "missing CLI option: {option}");
    }
}

#[test]
fn validate_config_accepts_toml_without_starting_or_mutating_storage() {
    let directory = temp_dir("validate");
    let storage = directory.join("graph-data");
    let config = write_config(
        &directory,
        &format!(
            "[server]\nhost = \"127.0.0.1\"\nport = 18080\nauth_token = \"file-secret\"\n\n[storage]\nmode = \"persistent\"\ndirectory = \"{}\"\n",
            storage.display()
        ),
    );

    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
    ]));

    assert!(output.status.success());
    assert!(!storage.exists(), "validation must not mutate storage");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("file-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("file-secret"));
}

#[test]
fn valid_toml_starts_a_foreground_server_listener() {
    let directory = temp_dir("foreground-start");
    let reservation = TcpListener::bind("127.0.0.1:0").expect("test port should be reserved");
    let port = reservation.local_addr().expect("local address").port();
    drop(reservation);
    let config = write_config(
        &directory,
        &format!("[server]\nhost = \"127.0.0.1\"\nport = {port}\nauth_token = \"start-secret\"\n"),
    );

    let mut child = corrobore()
        .args([
            "server",
            "start",
            "--config",
            config.to_str().expect("UTF-8 path"),
            "--data-dir",
            directory.join("runtime").to_str().expect("UTF-8 path"),
            "--log-dir",
            directory.join("logs").to_str().expect("UTF-8 path"),
        ])
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn");

    let mut listening = false;
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            listening = true;
            break;
        }
        if child
            .try_wait()
            .expect("server status should be readable")
            .is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }
    let _ = child.kill();
    let output = child
        .wait_with_output()
        .expect("server output should be collected");

    assert!(
        listening,
        "server did not listen in the foreground: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("start-secret"));
}

#[test]
fn status_accepts_a_dns_probe_host_without_overriding_the_bind_address() {
    let directory = temp_dir("status-probe-host");
    let config = write_config(
        &directory,
        "[server]\nhost = \"127.0.0.1\"\nport = 9\nauth_token = \"status-secret\"\n",
    );

    let output = run(corrobore().args([
        "server",
        "status",
        "--config",
        config.to_str().expect("UTF-8 path"),
        "--probe-host",
        "corrobore",
        "--query-timeout-ms",
        "50",
    ]));

    assert_eq!(
        output.status.code(),
        Some(8),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("server status=unavailable"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("expected an IP address"));
}

#[test]
fn cli_values_override_environment_and_file_values() {
    let directory = temp_dir("precedence");
    let config = write_config(
        &directory,
        "[server]\nhost = \"127.0.0.1\"\nport = 18080\nauth_token = \"file-secret\"\n",
    );

    let output = corrobore()
        .args([
            "server",
            "validate-config",
            "--config",
            config.to_str().expect("UTF-8 path"),
            "--port",
            "28080",
            "--auth-token",
            "cli-secret",
            "--print-effective",
        ])
        .env_clear()
        .env("CORROBORE_HTTP_PORT", "38080")
        .env("CORROBORE_HTTP_AUTH_TOKEN", "environment-secret")
        .output()
        .expect("corrobore command should execute");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("28080"), "CLI port must win");
    for secret in ["file-secret", "environment-secret", "cli-secret"] {
        assert!(!stdout.contains(secret), "effective config leaked {secret}");
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains(secret),
            "diagnostic leaked {secret}"
        );
    }
}

#[test]
fn operational_values_follow_cli_environment_file_precedence() {
    let directory = temp_dir("operational-precedence");
    let config = write_config(
        &directory,
        r#"
[server]
auth_token = "file-secret"
shutdown_timeout_ms = 1000

[storage]
mode = "ephemeral"

[interfaces]
enabled = ["http"]

[maintenance]
enabled = false
interval_ms = 2000
"#,
    );

    let output = corrobore()
        .args([
            "server",
            "validate-config",
            "--config",
            config.to_str().expect("UTF-8 path"),
            "--shutdown-timeout-ms",
            "3000",
            "--storage-mode",
            "persistent",
            "--storage-dir",
            ".cli/graph",
            "--interfaces",
            "http,web",
            "--web-dir",
            ".cli/web",
            "--maintenance-enabled",
            "true",
            "--maintenance-interval-ms",
            "4000",
            "--print-effective",
        ])
        .env_clear()
        .env("CORROBORE_HTTP_AUTH_TOKEN", "environment-secret")
        .env("CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS", "5000")
        .env("CORROBORE_STORAGE_MODE", "ephemeral")
        .env("CORROBORE_SERVER_INTERFACES", "web")
        .env("CORROBORE_MAINTENANCE_ENABLED", "false")
        .output()
        .expect("corrobore command should execute");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for expected in [
        "3000",
        "persistent",
        ".cli/graph",
        ".cli/web",
        "http",
        "web",
        "4000",
    ] {
        assert!(stdout.contains(expected), "missing CLI value: {expected}");
    }
    assert!(stdout.contains("maintenance.enabled = true"));
    for secret in ["file-secret", "environment-secret"] {
        assert!(!stdout.contains(secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    }
}

#[test]
fn environment_only_configuration_remains_supported() {
    let output = corrobore()
        .args(["server", "validate-config", "--print-effective"])
        .env_clear()
        .env("CORROBORE_HTTP_AUTH_TOKEN", "environment-only-secret")
        .env("CORROBORE_HTTP_HOST", "127.0.0.1")
        .env("CORROBORE_HTTP_PORT", "18081")
        .output()
        .expect("corrobore command should execute");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("127.0.0.1"));
    assert!(stdout.contains("18081"));
    assert!(!stdout.contains("environment-only-secret"));
}

#[test]
fn invalid_configuration_has_field_diagnostics_and_stable_exit_code() {
    let directory = temp_dir("invalid");
    let config = write_config(
        &directory,
        "[server]\nport = 0\nauth_token = \"never-print-this\"\n",
    );

    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("server.port"));
    assert!(!stderr.contains("never-print-this"));
}

#[test]
fn version_is_reproducible_and_available_without_runtime_configuration() {
    let first = run(corrobore().args(["server", "version"]));
    let second = run(corrobore().args(["server", "version"]));

    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    let stdout = String::from_utf8_lossy(&first.stdout);
    for expected in [
        env!("CARGO_PKG_VERSION"),
        "corrobore",
        "version=",
        "target=",
        "revision=",
    ] {
        assert!(
            stdout.contains(expected),
            "missing build metadata: {expected}"
        );
    }
}

#[test]
fn server_configuration_debug_output_redacts_secrets() {
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "primary-never-print".to_owned(),
        ),
        (
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-never-print".to_owned(),
        ),
    ]))
    .expect("configuration should be valid");
    let debug = format!("{config:?}");

    assert!(!debug.contains("primary-never-print"));
    assert!(!debug.contains("admin-never-print"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn toml_covers_operational_server_settings() {
    let directory = temp_dir("operational-settings");
    let config = write_config(
        &directory,
        r#"
[server]
host = "127.0.0.1"
port = 19090
auth_token = "primary-secret"
admin_auth_token = "admin-secret"
data_directory = ".runtime"
shutdown_timeout_ms = 9000

[storage]
mode = "ephemeral"
require_fsync = false
strict_recovery = false

[logging]
directory = ".runtime/logs"
level = "debug"
format = "json"

[limits]
request_timeout_ms = 45000
max_body_bytes = 4096
import_max_body_bytes = 8192
opencti_sync_max_operations = 64
opencti_sync_max_replay_identities = 128
rate_limit_per_second = 25
rate_limit_burst = 75
opencti_rate_limit_per_second = 250
opencti_rate_limit_burst = 2000

[interfaces]
enabled = ["http"]

[maintenance]
enabled = true
interval_ms = 60000

[tls]
enabled = false
"#,
    );
    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
        "--print-effective",
    ]));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for expected in [
        "19090",
        ".runtime",
        ".runtime/logs",
        "debug",
        "json",
        "9000",
        "45000",
        "4096",
        "8192",
        "64",
        "128",
        "25",
        "75",
        "250",
        "2000",
        "60000",
        "http",
    ] {
        assert!(
            stdout.contains(expected),
            "missing effective value: {expected}"
        );
    }
    assert!(!stdout.contains("primary-secret"));
    assert!(!stdout.contains("admin-secret"));
}

#[test]
fn conflicting_tls_configuration_has_a_field_level_error() {
    let directory = temp_dir("tls-conflict");
    let config = write_config(
        &directory,
        r#"
[server]
auth_token = "never-print-this"

[tls]
enabled = true
certificate_file = "server.crt"
"#,
    );
    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("tls.private_key_file"));
    assert!(!stderr.contains("never-print-this"));
}

#[test]
fn start_rejects_invalid_tls_material_instead_of_serving_plaintext() {
    let directory = temp_dir("tls-start");
    let config = write_config(
        &directory,
        r#"
[server]
auth_token = "never-print-this"

[tls]
enabled = true
certificate_file = "server.crt"
private_key_file = "server.key"
"#,
    );
    let output = run(corrobore().args([
        "server",
        "start",
        "--config",
        config.to_str().expect("UTF-8 path"),
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(STARTUP_EXIT_CODE));
    assert!(stderr.contains("tls.certificate_file"));
    assert!(!stderr.contains("never-print-this"));
}

#[test]
fn invalid_toml_diagnostics_never_echo_secret_source_lines() {
    let directory = temp_dir("invalid-secret-toml");
    let config = write_config(
        &directory,
        "[server]\nauth_token = [\"must-never-appear\"]\n",
    );
    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("config.file"));
    assert!(!stderr.contains("must-never-appear"));
}

#[test]
fn required_authentication_can_load_tokens_from_protected_files() {
    let directory = temp_dir("auth-token-files");
    let primary = directory.join("primary.token");
    let admin = directory.join("admin.token");
    fs::write(&primary, "primary-file-secret\n").expect("primary secret should be written");
    fs::write(&admin, "admin-file-secret\n").expect("admin secret should be written");
    let config = write_config(
        &directory,
        &format!(
            r#"
[server]
auth_mode = "required"
auth_token_file = "{}"
admin_auth_token_file = "{}"
"#,
            primary.display(),
            admin.display()
        ),
    );

    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--config",
        config.to_str().expect("UTF-8 path"),
        "--print-effective",
    ]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    for secret in ["primary-file-secret", "admin-file-secret"] {
        assert!(!stdout.contains(secret));
        assert!(!stderr.contains(secret));
    }
    assert!(stdout.contains("server.auth_mode = \"required\""));
    assert!(stdout.contains("server.auth_token = \"<redacted>\""));
    assert!(stdout.contains("server.auth_token_source = \"file\""));
}

#[test]
fn inline_and_file_secret_sources_are_mutually_exclusive() {
    let directory = temp_dir("auth-source-conflict");
    let token_file = directory.join("token");
    fs::write(&token_file, "file-secret").expect("secret should be written");
    let output = corrobore()
        .args([
            "server",
            "validate-config",
            "--auth-mode",
            "required",
            "--auth-token",
            "inline-secret",
            "--auth-token-file",
            token_file.to_str().expect("UTF-8 path"),
        ])
        .env_clear()
        .output()
        .expect("command should execute");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("server.auth_token"));
    assert!(stderr.contains("mutually exclusive"));
    assert!(!stderr.contains("inline-secret"));
    assert!(!stderr.contains("file-secret"));
}

#[test]
fn higher_precedence_secret_source_replaces_lower_precedence_source_kind() {
    let directory = temp_dir("auth-source-precedence");
    let token_file = directory.join("token");
    fs::write(&token_file, "file-secret").expect("secret should be written");
    let config = write_config(
        &directory,
        &format!(
            "[server]\nauth_token_file = {:?}\n",
            token_file.to_str().expect("UTF-8 path")
        ),
    );
    let output = corrobore()
        .args([
            "server",
            "validate-config",
            "--config",
            config.to_str().expect("UTF-8 path"),
            "--auth-token",
            "higher-precedence-secret",
            "--print-effective",
        ])
        .env_clear()
        .output()
        .expect("command should execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(stdout.contains("server.auth_token_source = \"inline\""));
    for secret in ["file-secret", "higher-precedence-secret"] {
        assert!(!stdout.contains(secret));
        assert!(!stderr.contains(secret));
    }
}

#[test]
fn local_insecure_mode_is_explicit_and_limited_to_loopback() {
    let local = run(corrobore().args([
        "server",
        "validate-config",
        "--host",
        "127.0.0.1",
        "--auth-mode",
        "local-insecure",
        "--print-effective",
    ]));
    assert!(
        local.status.success(),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );
    assert!(
        String::from_utf8_lossy(&local.stdout).contains("server.auth_mode = \"local-insecure\"")
    );

    let public = run(corrobore().args([
        "server",
        "validate-config",
        "--host",
        "0.0.0.0",
        "--auth-mode",
        "local-insecure",
    ]));
    let stderr = String::from_utf8_lossy(&public.stderr);
    assert_eq!(public.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("server.host"));
    assert!(stderr.contains("local-insecure"));
}

#[test]
fn non_loopback_exposure_requires_tls_authentication_and_protected_operations() {
    let directory = temp_dir("public-policy");
    let certificate = directory.join("server.crt");
    let private_key = directory.join("server.key");

    let plaintext = run(corrobore().args([
        "server",
        "validate-config",
        "--host",
        "0.0.0.0",
        "--auth-token",
        "public-secret",
    ]));
    let plaintext_stderr = String::from_utf8_lossy(&plaintext.stderr);
    assert_eq!(plaintext.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(plaintext_stderr.contains("tls.enabled"));
    assert!(!plaintext_stderr.contains("public-secret"));

    let public_operations = run(corrobore().args([
        "server",
        "validate-config",
        "--host",
        "0.0.0.0",
        "--auth-token",
        "public-secret",
        "--tls-enabled",
        "true",
        "--tls-certificate-file",
        certificate.to_str().expect("UTF-8 path"),
        "--tls-private-key-file",
        private_key.to_str().expect("UTF-8 path"),
        "--operational-endpoint-policy",
        "public",
    ]));
    let operations_stderr = String::from_utf8_lossy(&public_operations.stderr);
    assert_eq!(public_operations.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(operations_stderr.contains("operations.endpoint_policy"));
    assert!(!operations_stderr.contains("public-secret"));

    let secure = run(corrobore().args([
        "server",
        "validate-config",
        "--host",
        "0.0.0.0",
        "--auth-token",
        "public-secret",
        "--tls-enabled",
        "true",
        "--tls-certificate-file",
        certificate.to_str().expect("UTF-8 path"),
        "--tls-private-key-file",
        private_key.to_str().expect("UTF-8 path"),
        "--operational-endpoint-policy",
        "authenticated",
    ]));
    assert!(
        secure.status.success(),
        "{}",
        String::from_utf8_lossy(&secure.stderr)
    );
}

#[test]
fn unreadable_secret_file_has_actionable_non_secret_diagnostics() {
    let directory = temp_dir("missing-secret");
    let missing = directory.join("missing.token");
    let output = run(corrobore().args([
        "server",
        "validate-config",
        "--auth-token-file",
        missing.to_str().expect("UTF-8 path"),
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(CONFIG_EXIT_CODE));
    assert!(stderr.contains("server.auth_token_file"));
    assert!(stderr.contains("cannot read"));
}
