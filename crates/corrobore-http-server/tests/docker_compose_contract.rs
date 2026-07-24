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
use std::{fs, path::Path};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repository_file(path: &str) -> String {
    let path = workspace_root().join(path);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn compose_stack_wires_runtime_auth_health_and_persistence() {
    let compose = repository_file("docker-compose.yml");

    for expected in [
        "CORROBORE_HTTP_AUTH_TOKEN: ${CORROBORE_HTTP_AUTH_TOKEN:?",
        "healthcheck:",
        "\"server\", \"status\"",
        "/etc/corrobore/corrobore.toml",
        "CORROBORE_TLS_CERTIFICATE_FILE",
        "CORROBORE_TLS_PRIVATE_KEY_FILE",
        "stop_grace_period: 15s",
        "corrobore-data:/data",
    ] {
        assert!(
            compose.contains(expected),
            "Compose stack should include {expected}"
        );
    }

    for forbidden in [
        "runtime-config:",
        "condition: service_completed_successfully",
        "corrobore-runtime-config:/runtime-config",
        "corrobore-runtime-config:/app/web/runtime-config:ro",
        "corrobore-runtime-config:",
    ] {
        assert!(
            !compose.contains(forbidden),
            "Compose stack should not include {forbidden}"
        );
    }
}

#[test]
fn container_image_stays_secret_free_and_backend_only() {
    let dockerfile = repository_file("Dockerfile");

    assert!(
        dockerfile.contains("\"server\", \"status\""),
        "the image should use the product's bounded authenticated TLS probe"
    );
    assert!(
        !dockerfile.contains("web-builder"),
        "container image should not build frontend assets from this repository"
    );
    assert!(
        !dockerfile.contains("CORROBORE_HTTP_WEB_DIR=/app/web"),
        "container runtime should not force a baked-in web directory"
    );
    assert!(
        !dockerfile.contains("CORROBORE_HTTP_AUTH_TOKEN="),
        "the image must never bake the API token"
    );
    for expected in [
        "target/release/corrobore",
        "ENTRYPOINT [\"/usr/local/bin/corrobore\"]",
        "CMD [\"server\", \"start\"",
        "USER 65532:65532",
        "VOLUME [\"/data\"]",
        "STOPSIGNAL SIGTERM",
        "HEALTHCHECK",
        "\"server\", \"status\"",
        "org.opencontainers.image.version",
        "org.opencontainers.image.revision",
    ] {
        assert!(
            dockerfile.contains(expected),
            "production image should include {expected}"
        );
    }
    assert!(
        !dockerfile.contains("target/release/corrobore-http-server"),
        "the legacy HTTP-only binary must not be the container entrypoint"
    );
}

#[test]
fn production_configuration_and_systemd_unit_use_the_same_foreground_product() {
    let config = repository_file("packaging/corrobore.production.toml");
    for expected in [
        "host = \"0.0.0.0\"",
        "auth_token_file = \"/run/secrets/corrobore-http-token\"",
        "mode = \"persistent\"",
        "directory = \"/data/graph\"",
        "endpoint_policy = \"authenticated\"",
        "enabled = true",
        "certificate_file = \"/run/secrets/tls.crt\"",
        "private_key_file = \"/run/secrets/tls.key\"",
    ] {
        assert!(
            config.contains(expected),
            "production configuration should include {expected}"
        );
    }

    let service = repository_file("packaging/systemd/corrobore.service");
    for expected in [
        "User=corrobore",
        "Group=corrobore",
        "ExecStart=/usr/local/bin/corrobore server start --config /etc/corrobore/corrobore.toml",
        "KillSignal=SIGTERM",
        "TimeoutStopSec=15s",
        "Restart=on-failure",
        "NoNewPrivileges=true",
    ] {
        assert!(
            service.contains(expected),
            "systemd unit should include {expected}"
        );
    }
    assert!(!service.contains("--daemon"));
}
