// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use corrobore_http_server::{AppState, RuntimeStoreProvider, ServerConfig};
use graph_core::{Graph, NodeInput, PropertyValue};
use graph_storage::{
    AtomicPersistentMutationBatch, AtomicPersistentMutationNodeRecord,
    AtomicPersistentRuntimeState, DurableTransactionId, JsonLinesRecordCodec, RecordCodec,
    RecordFormat, StorageRef, StorageSegment, StorageVersion,
    apply_atomic_persistent_mutation_batch, create_node_record_envelope, open_storage_root,
    resolve_latest_node_storage_ref,
};

const OWNERSHIP_CONFLICT_EXIT_CODE: i32 = 4;
const STORAGE_INCOMPATIBLE_EXIT_CODE: i32 = 5;
const STORAGE_RECOVERY_EXIT_CODE: i32 = 6;

fn corrobore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
}

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-ownership-{name}-{suffix}"));
    fs::create_dir_all(&path).expect("test directory should be created");
    path
}

fn persistent_config(storage_dir: &Path) -> ServerConfig {
    ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "ownership-secret".to_owned(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage_dir.display().to_string(),
        ),
    ]))
    .expect("persistent configuration should parse")
}

fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test port should be reserved");
    listener.local_addr().expect("local address").port()
}

fn spawn_server(storage_dir: &Path, runtime_dir: &Path, port: u16) -> Child {
    corrobore()
        .args([
            "server",
            "start",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-token",
            "process-secret",
            "--data-dir",
            runtime_dir
                .join(format!("runtime-{port}"))
                .to_str()
                .expect("UTF-8 path"),
            "--log-dir",
            runtime_dir
                .join(format!("logs-{port}"))
                .to_str()
                .expect("UTF-8 path"),
            "--storage-mode",
            "persistent",
            "--storage-dir",
            storage_dir.to_str().expect("UTF-8 path"),
        ])
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn")
}

fn wait_for_listener(child: &mut Child, port: u16) -> bool {
    for _ in 0..75 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        if child
            .try_wait()
            .expect("server status should be readable")
            .is_some()
        {
            return false;
        }
        thread::sleep(Duration::from_millis(40));
    }
    false
}

fn wait_for_exit(mut child: Child) -> Output {
    for _ in 0..75 {
        if child
            .try_wait()
            .expect("server status should be readable")
            .is_some()
        {
            return child
                .wait_with_output()
                .expect("server output should be collected");
        }
        thread::sleep(Duration::from_millis(40));
    }
    let _ = child.kill();
    child
        .wait_with_output()
        .expect("timed-out server output should be collected")
}

#[test]
fn independent_persistent_handles_exclude_each_other_until_drop() {
    let root = temp_dir("independent-handles").join("graph");
    let first = AppState::new(persistent_config(&root)).expect("first owner should initialize");

    let error = match AppState::new(persistent_config(&root)) {
        Ok(_) => panic!("a second independent owner must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("already owned"));
    assert!(
        root.parent()
            .expect("storage root should have a parent")
            .join(".graph.corrobore.lock")
            .is_file()
    );

    drop(first);
    let restarted =
        AppState::new(persistent_config(&root)).expect("ownership should release after drop");
    drop(restarted);
}

#[test]
fn ephemeral_state_does_not_create_or_require_a_filesystem_lock() {
    let root = temp_dir("ephemeral");
    let config = ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "ephemeral-secret".to_owned(),
    )]))
    .expect("ephemeral configuration should parse");

    let first = AppState::new(config.clone()).expect("first ephemeral state should initialize");
    let second = AppState::new(config).expect("second ephemeral state should initialize");

    assert!(!root.join(".corrobore.lock").exists());
    drop((first, second));
}

#[test]
fn committed_persistent_catalog_data_survives_clean_server_restarts() {
    let root = temp_dir("clean-restart").join("graph");
    let first = AppState::new(persistent_config(&root)).expect("storage should initialize");
    drop(first);

    let storage_root = open_storage_root(root.clone()).expect("storage root should reopen");
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("restart.example".to_owned())),
        )
        .expect("node should be created");
    let node = graph
        .get_node(&node_id)
        .expect("node lookup should succeed")
        .expect("node should exist");
    let envelope = create_node_record_envelope(
        &node,
        StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 0,
            length: 1,
            checksum: None,
        },
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("node envelope should build");
    let batch = AtomicPersistentMutationBatch {
        transaction_id: DurableTransactionId::new("tx--server-clean-restart")
            .expect("transaction id should be valid"),
        node_records: vec![AtomicPersistentMutationNodeRecord {
            encoded_record: JsonLinesRecordCodec
                .encode_envelope(&envelope)
                .expect("node envelope should encode"),
            envelope,
            labels: vec!["Indicator".to_owned()],
            read_index: Default::default(),
        }],
        relationship_records: Vec::new(),
        outgoing_adjacency: Vec::new(),
        incoming_adjacency: Vec::new(),
        audit_events: vec!["server clean restart fixture".to_owned()],
    };
    let mut runtime_state = AtomicPersistentRuntimeState::default();
    apply_atomic_persistent_mutation_batch(&storage_root, &mut runtime_state, batch, None)
        .expect("mutation should commit");

    let restarted = AppState::new(persistent_config(&root)).expect("server should recover storage");
    let RuntimeStoreProvider::Persistent(runtime) = &restarted.runtime_store else {
        panic!("persistent runtime should be selected");
    };
    assert!(
        resolve_latest_node_storage_ref(runtime.store.catalog(), &node_id).is_ok(),
        "restarted server must retain committed catalog data"
    );
    drop(restarted);

    let second_restart =
        AppState::new(persistent_config(&root)).expect("second restart should also recover");
    drop(second_restart);
}

#[test]
fn real_server_processes_enforce_and_release_directory_ownership() {
    let directory = temp_dir("processes");
    let root = directory.join("graph");
    let first_port = reserve_port();
    let second_port = reserve_port();
    let mut first = spawn_server(&root, &directory, first_port);
    assert!(
        wait_for_listener(&mut first, first_port),
        "first server should listen"
    );

    let second = wait_for_exit(spawn_server(&root, &directory, second_port));
    let second_stderr = String::from_utf8_lossy(&second.stderr);
    assert_eq!(
        second.status.code(),
        Some(OWNERSHIP_CONFLICT_EXIT_CODE),
        "{second_stderr}"
    );
    assert!(second_stderr.contains("storage ownership"));
    assert!(!second_stderr.contains("process-secret"));

    let _ = first.kill();
    let _ = first.wait();

    let mut restarted = spawn_server(&root, &directory, second_port);
    let listening_after_release = wait_for_listener(&mut restarted, second_port);
    let _ = restarted.kill();
    let output = restarted
        .wait_with_output()
        .expect("restarted server output should be collected");
    assert!(
        listening_after_release,
        "ownership was not released after process termination: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn incompatible_manifest_has_a_distinct_actionable_exit_code() {
    let directory = temp_dir("incompatible");
    let root = directory.join("graph");
    fs::create_dir_all(&root).expect("storage root should be created");
    fs::write(
        root.join("manifest.json"),
        r#"{
  "storage_version": "V999",
  "graph_id": {"value": "graph--incompatible"},
  "created_at": {"value": "2026-07-23T00:00:00Z"},
  "updated_at": {"value": "2026-07-23T00:00:00Z"},
  "record_format": "JsonLinesV1"
}
"#,
    )
    .expect("manifest fixture should be written");

    let output = wait_for_exit(spawn_server(&root, &directory, reserve_port()));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(STORAGE_INCOMPATIBLE_EXIT_CODE),
        "{stderr}"
    );
    assert!(stderr.contains("incompatible"));
    assert!(stderr.contains("V999"));
    assert!(!stderr.contains("process-secret"));
}

#[test]
fn strict_recovery_rejects_corruption_with_a_distinct_exit_code() {
    let directory = temp_dir("corrupted");
    let root = directory.join("graph");
    let initial = AppState::new(persistent_config(&root)).expect("storage should initialize");
    drop(initial);
    fs::write(root.join("nodes").join("node_records.log"), b"not-json\n")
        .expect("node log should be corrupted");
    let _ = fs::remove_file(root.join("catalog").join("catalog_metadata.json"));

    let output = wait_for_exit(spawn_server(&root, &directory, reserve_port()));
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(STORAGE_RECOVERY_EXIT_CODE),
        "{stderr}"
    );
    assert!(stderr.contains("recovery"));
    assert!(!stderr.contains("process-secret"));
}
