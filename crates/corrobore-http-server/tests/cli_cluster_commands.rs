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

//! Issue #388: `corrobore cluster validate | plan | apply | status`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use corrobore_http_server::{AppState, ServerConfig};

static DIRS: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "corrobore-cluster-cli-{name}-{}-{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn corrobore(args: &[&str], paths: &[&Path]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_corrobore"));
    command.args(args);
    for path in paths {
        command.arg(path);
    }
    command.output().unwrap()
}

fn bundle(dir: &Path, query: &str) {
    std::fs::create_dir_all(dir.join("queries")).unwrap();
    std::fs::write(
        dir.join("cluster.yaml"),
        "version: 1\nmetadata: { name: intel }\npolicy: graph.policy.yaml\nqueries: queries/\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("graph.policy.yaml"),
        "version: 1\nrules:\n  - id: all\n    allow: { actors: any, actions: [read] }\n",
    )
    .unwrap();
    std::fs::write(dir.join("queries/count.cypher"), query).unwrap();
}

fn json(output: &Output) -> serde_json::Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn validate_plan_apply_and_status() {
    let config = temp_dir("config");
    bundle(&config, "MATCH (n) RETURN count(n)");
    let root = temp_dir("root").join("graph");

    let output = corrobore(&["cluster", "validate", "--config"], &[&config]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 stored query, a policy"));

    let plan = json(&corrobore(
        &["cluster", "plan", "--root"],
        &[&root, Path::new("--config"), &config],
    ));
    assert_eq!(
        plan["changes"],
        serde_json::json!([{"kind": "policy_install"}, {"kind": "query_add", "name": "count"}])
    );

    let apply = |id: &str| {
        corrobore(
            &[
                "cluster",
                "apply",
                "--deployment-id",
                id,
                "--as",
                "act-ops",
                "--root",
            ],
            &[&root, Path::new("--config"), &config],
        )
    };
    let first = json(&apply("first"));
    assert_eq!(first["applied"], true);
    assert_eq!(first["deployment"]["actor"], "act-ops");
    let again = json(&apply("first"));
    assert_eq!(again["applied"], false);

    // Changing the bundle under the same id is refused.
    bundle(&config, "MATCH (n:Indicator) RETURN count(n)");
    let conflict = apply("first");
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("already applied"));
    let plan = json(&corrobore(
        &["cluster", "plan", "--root"],
        &[&root, Path::new("--config"), &config],
    ));
    assert_eq!(
        plan["changes"],
        serde_json::json!([{"kind": "query_update", "name": "count"}])
    );

    let status = json(&corrobore(&["cluster", "status", "--root"], &[&root]));
    assert_eq!(status["applied"]["name"], "intel");
    assert_eq!(status["applied"]["queries"], serde_json::json!(["count"]));
    assert_eq!(status["deployments"].as_array().unwrap().len(), 1);
}

#[test]
fn a_running_server_keeps_direct_apply_out() {
    let config = temp_dir("config-owned");
    bundle(&config, "MATCH (n) RETURN count(n)");
    let root = temp_dir("root-owned").join("graph");
    let vars = HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned()),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            root.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            temp_dir("sessions").display().to_string(),
        ),
    ]);
    let state = AppState::new(ServerConfig::from_map(&vars).unwrap()).unwrap();
    let output = corrobore(
        &["cluster", "apply", "--deployment-id", "x", "--root"],
        &[&root, Path::new("--config"), &config],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("owned by another process"));
    drop(state);
}

#[test]
fn a_bundle_for_another_root_is_refused() {
    let config = temp_dir("config-other");
    bundle(&config, "MATCH (n) RETURN count(n)");
    let text = std::fs::read_to_string(config.join("cluster.yaml")).unwrap();
    std::fs::write(
        config.join("cluster.yaml"),
        format!("{text}storage: /srv/elsewhere\n"),
    )
    .unwrap();
    let root = temp_dir("root-other").join("graph");
    let output = corrobore(
        &["cluster", "plan", "--root"],
        &[&root, Path::new("--config"), &config],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("/srv/elsewhere"));
}
