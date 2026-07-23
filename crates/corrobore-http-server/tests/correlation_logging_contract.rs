// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![cfg(unix)]

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CORRELATION_ID: &str = "operator-probe-20";

fn temp_dir() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-correlation-{suffix}"));
    fs::create_dir_all(&path).expect("test directory should be created");
    path
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("test port should be reserved")
        .local_addr()
        .expect("reserved address should be available")
        .port()
}

fn wait_for_listener(child: &mut Child, port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        assert!(
            child
                .try_wait()
                .expect("server status should be readable")
                .is_none(),
            "server exited before listening"
        );
        thread::sleep(Duration::from_millis(40));
    }
    panic!("server did not start listening");
}

#[test]
fn structured_request_logs_and_error_envelope_share_the_response_correlation_id() {
    let directory = temp_dir();
    let log_dir = directory.join("logs");
    let port = reserve_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_corrobore"))
        .args([
            "server",
            "start",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-token",
            "correlation-secret",
            "--data-dir",
            directory.join("runtime").to_str().expect("UTF-8 path"),
            "--log-dir",
            log_dir.to_str().expect("UTF-8 path"),
        ])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn");
    wait_for_listener(&mut child, port);

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("request should connect");
    write!(
        stream,
        "POST /v1/cypher/read HTTP/1.1\r\nHost: localhost\r\nX-Request-Id: {CORRELATION_ID}\r\nContent-Type: application/json\r\nContent-Length: 30\r\nConnection: close\r\n\r\n{{\"query\":\"MATCH (n) RETURN n\"}}"
    )
    .expect("request should be sent");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("response should be readable");
    assert!(response.contains("401 Unauthorized"), "{response}");
    assert!(
        response
            .to_ascii_lowercase()
            .contains(&format!("x-request-id: {CORRELATION_ID}")),
        "{response}"
    );
    assert!(
        response.contains(&format!(r#""correlation_id":"{CORRELATION_ID}""#)),
        "{response}"
    );

    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("termination signal should run");
    assert!(status.success());
    let output = child
        .wait_with_output()
        .expect("server output should be collected");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let logs = fs::read_to_string(log_dir.join("http-server.session.log.jsonl"))
        .expect("structured logs should be readable");
    assert!(
        logs.lines().any(|line| {
            line.contains(CORRELATION_ID)
                && line.contains("http_request")
                && (line.contains("started processing request")
                    || line.contains("finished processing request"))
        }),
        "request log should carry the shared correlation ID:\n{logs}"
    );
}
