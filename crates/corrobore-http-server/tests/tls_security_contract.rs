// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![cfg(unix)]

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rcgen::{CertificateParams, CertifiedKey, KeyPair, date_time_ymd, generate_simple_self_signed};

const STARTUP_EXIT_CODE: i32 = 3;
const AUTH_TOKEN: &str = "tls-never-log-this-secret";

struct CertificateFixture {
    certificate: PathBuf,
    private_key: PathBuf,
    certificate_pem: String,
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-tls-{name}-{suffix}"));
    fs::create_dir_all(&path).expect("test directory should be created");
    path
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("test port should be reserved")
        .local_addr()
        .expect("reserved address")
        .port()
}

fn write_current_certificate(directory: &Path, name: &str) -> CertificateFixture {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("certificate should generate");
    write_certificate(directory, name, cert.pem(), signing_key.serialize_pem())
}

fn write_expired_certificate(directory: &Path) -> CertificateFixture {
    let mut params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("parameters should build");
    params.not_before = date_time_ymd(2019, 1, 1);
    params.not_after = date_time_ymd(2020, 1, 1);
    let signing_key = KeyPair::generate().expect("key should generate");
    let cert = params
        .self_signed(&signing_key)
        .expect("expired certificate should generate");
    write_certificate(
        directory,
        "expired",
        cert.pem(),
        signing_key.serialize_pem(),
    )
}

fn write_certificate(
    directory: &Path,
    name: &str,
    certificate_pem: String,
    private_key_pem: String,
) -> CertificateFixture {
    let certificate = directory.join(format!("{name}.crt"));
    let private_key = directory.join(format!("{name}.key"));
    fs::write(&certificate, &certificate_pem).expect("certificate should be written");
    fs::write(&private_key, private_key_pem).expect("private key should be written");
    CertificateFixture {
        certificate,
        private_key,
        certificate_pem,
    }
}

fn corrobore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
}

fn spawn_server(
    directory: &Path,
    port: u16,
    certificate: &CertificateFixture,
    extra_args: &[&str],
) -> Child {
    let mut command = corrobore();
    command.args([
        "server",
        "start",
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--data-dir",
        directory.join("runtime").to_str().expect("UTF-8 path"),
        "--log-dir",
        directory.join("logs").to_str().expect("UTF-8 path"),
        "--tls-enabled",
        "true",
        "--tls-certificate-file",
        certificate.certificate.to_str().expect("UTF-8 path"),
        "--tls-private-key-file",
        certificate.private_key.to_str().expect("UTF-8 path"),
    ]);
    command
        .args(extra_args)
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn")
}

fn tls_client(certificate_pem: &str) -> reqwest::Client {
    let certificate = reqwest::Certificate::from_pem(certificate_pem.as_bytes())
        .expect("test certificate should parse");
    reqwest::Client::builder()
        .add_root_certificate(certificate)
        .timeout(Duration::from_secs(2))
        .build()
        .expect("TLS client should build")
}

async fn wait_for_https(child: &mut Child, client: &reqwest::Client, port: u16, auth_token: &str) {
    for _ in 0..100 {
        if client
            .get(format!("https://localhost:{port}/health/live"))
            .bearer_auth(auth_token)
            .send()
            .await
            .is_ok()
        {
            return;
        }
        assert!(
            child
                .try_wait()
                .expect("server status should be readable")
                .is_none(),
            "server exited before HTTPS became ready"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("HTTPS listener did not become ready");
}

fn terminate(child: Child) -> Output {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("termination signal should run");
    assert!(status.success());
    child
        .wait_with_output()
        .expect("server output should be collected")
}

#[tokio::test]
async fn https_listener_enforces_api_and_operational_auth_without_leaking_secrets() {
    let directory = temp_dir("authenticated");
    let certificate = write_current_certificate(&directory, "server");
    let port = reserve_port();
    let mut child = spawn_server(
        &directory,
        port,
        &certificate,
        &[
            "--auth-token",
            AUTH_TOKEN,
            "--operational-endpoint-policy",
            "authenticated",
        ],
    );
    let client = tls_client(&certificate.certificate_pem);
    wait_for_https(&mut child, &client, port, AUTH_TOKEN).await;

    let unauthenticated_operations = client
        .get(format!("https://localhost:{port}/health/live"))
        .send()
        .await
        .expect("operational request should complete");
    assert_eq!(
        unauthenticated_operations.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

    let unauthenticated_api = client
        .post(format!("https://localhost:{port}/v1/cypher/read"))
        .json(&serde_json::json!({"query": "MATCH (n) RETURN n"}))
        .send()
        .await
        .expect("API request should complete");
    assert_eq!(
        unauthenticated_api.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

    let authenticated_api = client
        .post(format!("https://localhost:{port}/v1/cypher/read"))
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({"query": "MATCH (n) RETURN n"}))
        .send()
        .await
        .expect("authenticated API request should complete");
    assert_eq!(authenticated_api.status(), reqwest::StatusCode::OK);

    let metrics = client
        .get(format!("https://localhost:{port}/metrics"))
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("metrics request should complete")
        .text()
        .await
        .expect("metrics should be readable");
    assert!(!metrics.contains(AUTH_TOKEN));

    let output = terminate(child);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains(AUTH_TOKEN));
    let logs = fs::read_to_string(directory.join("logs/http-server.session.log.jsonl"))
        .expect("structured logs should be readable");
    assert!(!logs.contains(AUTH_TOKEN));
}

#[tokio::test]
async fn local_insecure_mode_allows_loopback_without_an_authentication_secret() {
    let directory = temp_dir("local-insecure");
    let port = reserve_port();
    let mut child = corrobore()
        .args([
            "server",
            "start",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-mode",
            "local-insecure",
            "--data-dir",
            directory.join("runtime").to_str().expect("UTF-8 path"),
            "--log-dir",
            directory.join("logs").to_str().expect("UTF-8 path"),
        ])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client should build");
    let mut response = None;
    for _ in 0..100 {
        match client
            .post(format!("http://127.0.0.1:{port}/v1/cypher/read"))
            .json(&serde_json::json!({"query": "MATCH (n) RETURN n"}))
            .send()
            .await
        {
            Ok(value) => {
                response = Some(value);
                break;
            }
            Err(_) => {
                assert!(
                    child
                        .try_wait()
                        .expect("server status should be readable")
                        .is_none(),
                    "server exited before HTTP became ready"
                );
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }
    }
    assert_eq!(
        response.expect("HTTP listener should respond").status(),
        reqwest::StatusCode::OK
    );
    assert!(terminate(child).status.success());
}

#[test]
fn invalid_unreadable_mismatched_and_expired_tls_material_fail_closed() {
    let directory = temp_dir("invalid-material");
    let current_a = write_current_certificate(&directory, "current-a");
    let current_b = write_current_certificate(&directory, "current-b");
    let expired = write_expired_certificate(&directory);

    let cases = [
        (
            directory.join("missing.crt"),
            current_a.private_key.clone(),
            "tls.certificate_file",
        ),
        (
            current_a.certificate.clone(),
            current_b.private_key.clone(),
            "does not match",
        ),
        (
            expired.certificate.clone(),
            expired.private_key.clone(),
            "expired",
        ),
    ];

    for (certificate, private_key, expected) in cases {
        let output = corrobore()
            .args([
                "server",
                "start",
                "--host",
                "127.0.0.1",
                "--port",
                &reserve_port().to_string(),
                "--auth-token",
                AUTH_TOKEN,
                "--tls-enabled",
                "true",
                "--tls-certificate-file",
                certificate.to_str().expect("UTF-8 path"),
                "--tls-private-key-file",
                private_key.to_str().expect("UTF-8 path"),
            ])
            .env_clear()
            .output()
            .expect("server command should execute");
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        assert_eq!(output.status.code(), Some(STARTUP_EXIT_CODE), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.contains(AUTH_TOKEN));
    }
}

#[tokio::test]
async fn tls_material_is_reloaded_on_process_restart_without_data_migration() {
    let directory = temp_dir("rotation");
    let first = write_current_certificate(&directory, "first");
    let second = write_current_certificate(&directory, "second");
    let auth_token_file = directory.join("server.token");
    fs::write(&auth_token_file, AUTH_TOKEN).expect("initial auth token should be written");
    let auth_token_file = auth_token_file.to_str().expect("UTF-8 path").to_owned();
    let port = reserve_port();

    let mut first_child = spawn_server(
        &directory,
        port,
        &first,
        &[
            "--auth-token-file",
            &auth_token_file,
            "--operational-endpoint-policy",
            "authenticated",
        ],
    );
    let first_client = tls_client(&first.certificate_pem);
    wait_for_https(&mut first_child, &first_client, port, AUTH_TOKEN).await;
    assert!(terminate(first_child).status.success());

    let rotated_token = "rotated-auth-secret";
    fs::write(&auth_token_file, rotated_token).expect("rotated auth token should be written");
    let mut second_child = spawn_server(
        &directory,
        port,
        &second,
        &[
            "--auth-token-file",
            &auth_token_file,
            "--operational-endpoint-policy",
            "authenticated",
        ],
    );
    let second_client = tls_client(&second.certificate_pem);
    wait_for_https(&mut second_child, &second_client, port, rotated_token).await;
    let old_token_response = second_client
        .get(format!("https://localhost:{port}/health/live"))
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .expect("old token request should complete");
    assert_eq!(
        old_token_response.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let response = second_client
        .get(format!("https://localhost:{port}/version"))
        .bearer_auth(rotated_token)
        .send()
        .await
        .expect("rotated listener should respond");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(terminate(second_child).status.success());
}
