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
        "http://127.0.0.1:8080/health",
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
        dockerfile.contains("/bin/busybox"),
        "distroless runtime should include a bounded healthcheck client"
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
}
