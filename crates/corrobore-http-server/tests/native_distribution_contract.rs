// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{fs, path::Path};

fn workspace_file(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn release_workflow_packages_the_unified_native_executable_with_revision_metadata() {
    let release = workspace_file(".github/workflows/release.yml");

    assert!(release.contains("target/release/corrobore"));
    assert!(release.contains("CORROBORE_BUILD_REVISION: ${{ github.sha }}"));
    assert!(release.contains("corrobore server version"));
    assert!(!release.contains("target/release/corrobore-http-server"));
}

#[test]
fn unified_executable_reports_reproducible_product_metadata() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_corrobore"))
        .args(["server", "version"])
        .output()
        .expect("unified executable should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("corrobore version="));
    assert!(stdout.contains("target="));
    assert!(stdout.contains("revision="));
}
