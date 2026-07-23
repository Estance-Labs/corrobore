// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
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
    for command in ["start", "validate-config", "version"] {
        assert!(
            stdout.contains(command),
            "missing server command: {command}"
        );
    }
}

#[test]
fn start_help_covers_the_operational_configuration_surface() {
    let output = run(corrobore().args(["server", "start", "--help"]));
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    for option in [
        "--host",
        "--port",
        "--data-dir",
        "--storage-mode",
        "--storage-dir",
        "--log-dir",
        "--log-level",
        "--log-format",
        "--query-timeout-ms",
        "--shutdown-timeout-ms",
        "--max-body-bytes",
        "--interfaces",
        "--web-dir",
        "--maintenance-enabled",
        "--tls-enabled",
        "--tls-certificate-file",
        "--tls-private-key-file",
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
        .env("CORROBORE_HTTP_HOST", "0.0.0.0")
        .env("CORROBORE_HTTP_PORT", "18081")
        .output()
        .expect("corrobore command should execute");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("0.0.0.0"));
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
rate_limit_per_second = 25
rate_limit_burst = 75

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
        "25",
        "75",
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
fn start_rejects_tls_instead_of_silently_serving_plaintext() {
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
    assert!(stderr.contains("TLS listener"));
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
