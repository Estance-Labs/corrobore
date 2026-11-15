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

//! Issue #384: `corrobore policy validate | test | explain`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::PathBuf, process::Command};

fn corrobore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
}

static DIRS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn file(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corrobore-policy-cli-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        DIRS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

const POLICY: &str = "version: 1
groups:
  writers: [act-writer]
protected_branches: [main]
rules:
  - id: writers-change-drafts
    allow:
      actors: { group: writers }
      actions: [change]
      branch_scope: unprotected
";

#[test]
fn validate_reports_a_valid_policy_and_refuses_an_invalid_one() {
    let output = corrobore()
        .args(["policy", "validate", "--file"])
        .arg(file("policy.yaml", POLICY))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("policy is valid"), "{stdout}");
    assert!(stdout.contains("1 rule"), "{stdout}");

    let output = corrobore()
        .args(["policy", "validate", "--file"])
        .arg(file(
            "bad.yaml",
            "version: 1\nrules: [{id: r, allow: {actors: any, actions: [fly]}}]\n",
        ))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fly"));
}

#[test]
fn test_runs_cases_and_fails_on_a_wrong_expectation() {
    let policy = file("policy.yaml", POLICY);
    let passing = file(
        "cases.yaml",
        "version: 1
cases:
  - { id: drafts, actor: act-writer, action: change, branch: drafts, expect: allow }
  - { id: main, actor: act-writer, action: change, branch: main, expect: deny }
  - { id: stranger, actor: act-other, action: change, branch: drafts, expect: deny }
",
    );
    let output = corrobore()
        .args(["policy", "test", "--file"])
        .arg(&policy)
        .arg("--tests")
        .arg(&passing)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("3 cases passed"));

    let failing = file(
        "wrong.yaml",
        "version: 1
cases:
  - { id: main-write, actor: act-writer, action: change, branch: main, expect: allow }
",
    );
    let output = corrobore()
        .args(["policy", "test", "--file"])
        .arg(&policy)
        .arg("--tests")
        .arg(&failing)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FAIL main-write: expected allow"),
        "{stdout}"
    );
}

#[test]
fn explain_names_the_rule_that_decides() {
    let output = corrobore()
        .args(["policy", "explain", "--file"])
        .arg(file("policy.yaml", POLICY))
        .args([
            "--actor",
            "act-writer",
            "--action",
            "change",
            "--branch",
            "drafts",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let decision: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(decision["allowed"], true);
    assert_eq!(
        decision["rules"],
        serde_json::json!(["writers-change-drafts"])
    );

    let output = corrobore()
        .args(["policy", "explain", "--file"])
        .arg(file("policy.yaml", POLICY))
        .args([
            "--actor",
            "act-writer",
            "--action",
            "change",
            "--branch",
            "main",
        ])
        .output()
        .unwrap();
    let decision: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(decision["allowed"], false);
}
