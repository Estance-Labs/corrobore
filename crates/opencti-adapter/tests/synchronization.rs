// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use graph_core::Graph;
use opencti_adapter::{
    BulkLimits, DivergenceStatus, MutationClass, OpenCtiMutation, OpenCtiSyncBatch,
    OpenCtiSynchronizer, OperationStatus, SyncCheckpoint, SyncPhase,
};
use serde_json::{Value, json};

fn object(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "type": "indicator",
        "name": name,
        "object_marking_refs": ["marking-definition--clear"]
    })
}

fn relationship(id: &str, source: &str, target: &str) -> Value {
    json!({
        "id": id,
        "type": "relationship",
        "relationship_type": "indicates",
        "source_ref": source,
        "target_ref": target
    })
}

fn mutation(
    operation_id: &str,
    sequence: u64,
    class: MutationClass,
    record: Value,
) -> OpenCtiMutation {
    OpenCtiMutation::new(operation_id, sequence, class, record).unwrap()
}

fn batch(
    phase: SyncPhase,
    high_water_mark: u64,
    snapshot_complete: bool,
    operations: Vec<OpenCtiMutation>,
) -> OpenCtiSyncBatch {
    OpenCtiSyncBatch::new(
        "opencti--primary",
        "snapshot--2026-07-25",
        phase,
        high_water_mark,
        snapshot_complete,
        operations,
    )
    .unwrap()
}

#[test]
fn consistent_snapshot_plus_catch_up_matches_quiescent_export() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits::default());
    let mut quiescent = Graph::new();
    let mut quiescent_checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    synchronizer
        .apply_batch(
            &mut quiescent,
            &mut quiescent_checkpoint,
            batch(
                SyncPhase::Snapshot,
                4,
                true,
                vec![
                    mutation(
                        "q-1",
                        1,
                        MutationClass::Upsert,
                        object("indicator--1", "final"),
                    ),
                    mutation(
                        "q-2",
                        2,
                        MutationClass::Upsert,
                        object("malware--1", "malware"),
                    ),
                    mutation(
                        "q-3",
                        3,
                        MutationClass::RelationshipUpsert,
                        relationship("relationship--1", "indicator--1", "malware--1"),
                    ),
                    mutation(
                        "q-4",
                        4,
                        MutationClass::AccessPolicyUpdate,
                        object("identity--1", "allowed"),
                    ),
                ],
            ),
        )
        .unwrap();

    let mut shadow = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    synchronizer
        .apply_batch(
            &mut shadow,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                2,
                true,
                vec![
                    mutation(
                        "s-1",
                        1,
                        MutationClass::Upsert,
                        object("indicator--1", "old"),
                    ),
                    mutation(
                        "s-2",
                        2,
                        MutationClass::Upsert,
                        object("malware--1", "malware"),
                    ),
                ],
            ),
        )
        .unwrap();
    synchronizer
        .apply_batch(
            &mut shadow,
            &mut checkpoint,
            batch(
                SyncPhase::CatchUp,
                4,
                false,
                vec![
                    mutation(
                        "c-3",
                        3,
                        MutationClass::Upsert,
                        object("indicator--1", "final"),
                    ),
                    mutation(
                        "c-4",
                        4,
                        MutationClass::RelationshipUpsert,
                        relationship("relationship--1", "indicator--1", "malware--1"),
                    ),
                    mutation(
                        "c-5",
                        5,
                        MutationClass::AccessPolicyUpdate,
                        object("identity--1", "allowed"),
                    ),
                ],
            ),
        )
        .unwrap();

    assert_eq!(
        synchronizer.digest(&shadow).unwrap(),
        synchronizer.digest(&quiescent).unwrap()
    );
}

#[test]
fn replay_and_checkpoint_resume_are_idempotent() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits::default());
    let original = batch(
        SyncPhase::Snapshot,
        1,
        true,
        vec![mutation(
            "operation--1",
            1,
            MutationClass::Upsert,
            object("indicator--1", "one"),
        )],
    );
    let mut graph = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");

    let first = synchronizer
        .apply_batch(&mut graph, &mut checkpoint, original.clone())
        .unwrap();
    let first_digest = synchronizer.digest(&graph).unwrap();
    let persisted = serde_json::to_vec(&checkpoint).unwrap();
    let mut resumed: SyncCheckpoint = serde_json::from_slice(&persisted).unwrap();
    let replay = synchronizer
        .apply_batch(&mut graph, &mut resumed, original)
        .unwrap();

    assert_eq!(first.operations[0].status, OperationStatus::Applied);
    assert_eq!(replay.operations[0].status, OperationStatus::Duplicate);
    assert_eq!(synchronizer.digest(&graph).unwrap(), first_digest);
    assert_eq!(resumed.last_acknowledged_sequence, 1);
}

#[test]
fn bulk_limits_and_retryable_dependencies_bound_memory_and_checkpoint_progress() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits {
        max_operations: 2,
        max_payload_bytes: 1_024,
        max_replay_identities: 16,
    });
    let mut graph = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");

    let over_limit = batch(
        SyncPhase::Snapshot,
        3,
        false,
        vec![
            mutation(
                "o-1",
                1,
                MutationClass::Upsert,
                object("indicator--1", "one"),
            ),
            mutation(
                "o-2",
                2,
                MutationClass::Upsert,
                object("indicator--2", "two"),
            ),
            mutation(
                "o-3",
                3,
                MutationClass::Upsert,
                object("indicator--3", "three"),
            ),
        ],
    );
    assert!(
        synchronizer
            .apply_batch(&mut graph, &mut checkpoint, over_limit)
            .unwrap_err()
            .to_string()
            .contains("max_operations")
    );

    let blocked = synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                2,
                false,
                vec![
                    mutation(
                        "r-1",
                        1,
                        MutationClass::RelationshipUpsert,
                        relationship("relationship--1", "missing--1", "missing--2"),
                    ),
                    mutation(
                        "r-2",
                        2,
                        MutationClass::Upsert,
                        object("indicator--2", "two"),
                    ),
                ],
            ),
        )
        .unwrap();

    assert_eq!(blocked.operations[0].status, OperationStatus::Retryable);
    assert_eq!(blocked.operations[1].status, OperationStatus::Retryable);
    assert_eq!(checkpoint.last_acknowledged_sequence, 0);
    assert_eq!(checkpoint.queue_depth, 2);
    assert_eq!(checkpoint.retry_count, 2);
}

#[test]
fn per_operation_results_distinguish_every_required_outcome() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits::default());
    let mut graph = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    let initial = batch(
        SyncPhase::Snapshot,
        1,
        false,
        vec![mutation(
            "same-id",
            1,
            MutationClass::Upsert,
            object("indicator--1", "one"),
        )],
    );
    synchronizer
        .apply_batch(&mut graph, &mut checkpoint, initial.clone())
        .unwrap();

    let duplicate = synchronizer
        .apply_batch(&mut graph, &mut checkpoint, initial)
        .unwrap();
    assert_eq!(duplicate.operations[0].status, OperationStatus::Duplicate);

    let permanent = synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                2,
                false,
                vec![mutation(
                    "bad-record",
                    2,
                    MutationClass::Upsert,
                    json!({"type": "indicator"}),
                )],
            ),
        )
        .unwrap();
    assert_eq!(
        permanent.operations[0].status,
        OperationStatus::PermanentlyRejected
    );

    let quarantined = synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                2,
                false,
                vec![mutation(
                    "same-id",
                    2,
                    MutationClass::Upsert,
                    object("indicator--2", "conflict"),
                )],
            ),
        )
        .unwrap();
    assert_eq!(
        quarantined.operations[0].status,
        OperationStatus::Quarantined
    );
    assert_eq!(checkpoint.rejected_operations, 1);
    assert_eq!(checkpoint.quarantined_operations, 1);
    assert_eq!(checkpoint.dead_letters.len(), 2);
    assert_eq!(
        checkpoint.dead_letters[0].status,
        OperationStatus::PermanentlyRejected
    );
    assert_eq!(
        checkpoint.dead_letters[1].status,
        OperationStatus::Quarantined
    );
}

#[test]
fn validation_gates_shadow_reads_on_records_relations_access_and_projection_freshness() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits::default());
    let mut graph = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                3,
                true,
                vec![
                    mutation(
                        "v-1",
                        1,
                        MutationClass::Upsert,
                        object("indicator--1", "one"),
                    ),
                    mutation(
                        "v-2",
                        2,
                        MutationClass::Upsert,
                        object("malware--1", "malware"),
                    ),
                    mutation(
                        "v-3",
                        3,
                        MutationClass::RelationshipUpsert,
                        relationship("relationship--1", "indicator--1", "malware--1"),
                    ),
                ],
            ),
        )
        .unwrap();

    let expected = synchronizer.digest(&graph).unwrap();
    let valid = synchronizer.validate(&graph, &expected, true).unwrap();
    assert_eq!(valid.divergence, DivergenceStatus::InSync);
    assert!(valid.shadow_reads_enabled);

    let mut wrong = expected.clone();
    wrong.property_checksum = "different-properties".to_owned();
    wrong.access_policy_checksum = "different".to_owned();
    let divergent = synchronizer.validate(&graph, &wrong, false).unwrap();
    assert_eq!(divergent.divergence, DivergenceStatus::Diverged);
    assert!(!divergent.shadow_reads_enabled);
    assert!(
        divergent
            .differences
            .iter()
            .any(|value| value == "properties")
    );
    assert!(
        divergent
            .differences
            .iter()
            .any(|value| value == "access_policy")
    );
    assert!(
        divergent
            .differences
            .iter()
            .any(|value| value == "projections")
    );
}

#[test]
fn relationship_and_object_deletes_match_a_quiescent_export() {
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits::default());
    let mut graph = Graph::new();
    let mut checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::Snapshot,
                3,
                true,
                vec![
                    mutation(
                        "d-1",
                        1,
                        MutationClass::Upsert,
                        object("indicator--keep", "kept"),
                    ),
                    mutation(
                        "d-2",
                        2,
                        MutationClass::Upsert,
                        object("malware--delete", "deleted"),
                    ),
                    mutation(
                        "d-3",
                        3,
                        MutationClass::RelationshipUpsert,
                        relationship("relationship--delete", "indicator--keep", "malware--delete"),
                    ),
                ],
            ),
        )
        .unwrap();
    synchronizer
        .apply_batch(
            &mut graph,
            &mut checkpoint,
            batch(
                SyncPhase::CatchUp,
                6,
                false,
                vec![
                    mutation(
                        "d-4",
                        4,
                        MutationClass::AccessPolicyUpdate,
                        json!({
                            "id": "relationship--delete",
                            "type": "relationship",
                            "relationship_type": "indicates",
                            "source_ref": "indicator--keep",
                            "target_ref": "malware--delete",
                            "object_marking_refs": ["marking-definition--restricted"]
                        }),
                    ),
                    mutation(
                        "d-5",
                        5,
                        MutationClass::RelationshipDelete,
                        json!({"id": "relationship--delete"}),
                    ),
                    mutation(
                        "d-6",
                        6,
                        MutationClass::Delete,
                        json!({"id": "malware--delete"}),
                    ),
                ],
            ),
        )
        .unwrap();

    let mut quiescent = Graph::new();
    let mut expected_checkpoint = SyncCheckpoint::new("opencti--primary", "snapshot--2026-07-25");
    synchronizer
        .apply_batch(
            &mut quiescent,
            &mut expected_checkpoint,
            batch(
                SyncPhase::Snapshot,
                1,
                true,
                vec![mutation(
                    "expected-1",
                    1,
                    MutationClass::Upsert,
                    object("indicator--keep", "kept"),
                )],
            ),
        )
        .unwrap();

    assert_eq!(
        synchronizer.digest(&graph).unwrap(),
        synchronizer.digest(&quiescent).unwrap()
    );
}

#[test]
fn small_and_medium_compatibility_corpus_reach_zero_persistent_divergence() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/parity-corpus.json"
    ))
    .unwrap();
    let fixtures = corpus["fixtures"].as_array().unwrap();
    let synchronizer = OpenCtiSynchronizer::new(BulkLimits {
        max_operations: 4_096,
        max_payload_bytes: 16 * 1024 * 1024,
        max_replay_identities: 8_192,
    });

    for multiplier in [1_u64, 8] {
        let mut graph = Graph::new();
        let mut checkpoint =
            SyncCheckpoint::new("opencti--primary", format!("snapshot--corpus-{multiplier}"));
        let mut operations = Vec::new();
        let mut sequence = 0_u64;
        for copy in 0..multiplier {
            for fixture in fixtures {
                sequence += 1;
                let mut record = fixture.clone();
                if copy > 0 {
                    let id = record["id"].as_str().unwrap();
                    record["id"] = Value::String(format!("{id}-copy-{copy}"));
                    if record.get("source_ref").is_some() || record.get("target_ref").is_some() {
                        continue;
                    }
                }
                let class = if record.get("source_ref").is_some() {
                    MutationClass::RelationshipUpsert
                } else {
                    MutationClass::Upsert
                };
                operations.push(mutation(
                    &format!("corpus-{copy}-{sequence}"),
                    operations.len() as u64 + 1,
                    class,
                    record,
                ));
            }
        }
        let high_water_mark = operations.len() as u64;
        let outcome = synchronizer
            .apply_batch(
                &mut graph,
                &mut checkpoint,
                OpenCtiSyncBatch::new(
                    "opencti--primary",
                    format!("snapshot--corpus-{multiplier}"),
                    SyncPhase::Snapshot,
                    high_water_mark,
                    true,
                    operations,
                )
                .unwrap(),
            )
            .unwrap();
        assert!(outcome.operations.iter().all(|result| matches!(
            result.status,
            OperationStatus::Applied
                | OperationStatus::PermanentlyRejected
                | OperationStatus::Quarantined
        )));

        let expected = synchronizer.digest(&graph).unwrap();
        let restored_checkpoint: SyncCheckpoint =
            serde_json::from_slice(&serde_json::to_vec(&checkpoint).unwrap()).unwrap();
        let validation = synchronizer.validate(&graph, &expected, true).unwrap();
        assert_eq!(validation.divergence, DivergenceStatus::InSync);
        assert_eq!(
            restored_checkpoint.last_acknowledged_sequence,
            high_water_mark
        );
    }
}
