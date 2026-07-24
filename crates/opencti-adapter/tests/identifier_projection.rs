// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use opencti_adapter::{
    IdentifierKind, IdentifierProjection, IdentifierTransaction, MergeSource, OpenCtiAdapter,
    ProjectionApply, ProjectionError,
};
use serde_json::{Value, json};

fn record(overrides: Value) -> opencti_adapter::MappedRecord {
    let mut base = json!({
        "id": "indicator--00000000-0000-4000-8000-000000000201",
        "internal_id": "internal--00000000-0000-4000-8000-000000000201",
        "standard_id": "indicator--00000000-0000-4000-8000-000000000201",
        "x_opencti_stix_ids": [
            "indicator--00000000-0000-4000-8000-000000000200"
        ],
        "i_aliases_ids": ["dedup--synthetic-indicator"],
        "type": "indicator",
        "entity_type": "Indicator",
        "parent_types": ["Stix-Object", "Stix-Core-Object", "Stix-Domain-Object"],
        "name": "Synthetic indicator",
        "aliases": ["Synthetic alias"],
        "external_references": [{
            "source_name": "synthetic",
            "external_id": "SYNTH-201"
        }]
    });
    merge_json(
        base.as_object_mut()
            .expect("base record should be an object"),
        overrides,
    );
    OpenCtiAdapter::pinned()
        .map(base)
        .expect("test record should map")
}

fn merge_json(target: &mut serde_json::Map<String, Value>, overrides: Value) {
    for (key, value) in overrides
        .as_object()
        .expect("overrides should be an object")
    {
        target.insert(key.clone(), value.clone());
    }
}

#[test]
fn create_indexes_every_identifier_kind_and_returns_the_current_record() {
    let mapped = record(json!({}));
    let record_ref = mapped.record_ref();
    let mut projection = IdentifierProjection::new();
    let transaction =
        IdentifierTransaction::new("tx-create").upsert(mapped.projection_record(1), None);

    assert_eq!(
        projection.apply(transaction).expect("create should apply"),
        ProjectionApply::Applied
    );

    for (kind, value) in [
        (
            IdentifierKind::Internal,
            "internal--00000000-0000-4000-8000-000000000201",
        ),
        (
            IdentifierKind::Standard,
            "indicator--00000000-0000-4000-8000-000000000201",
        ),
        (
            IdentifierKind::Stix,
            "indicator--00000000-0000-4000-8000-000000000200",
        ),
        (IdentifierKind::External, "SYNTH-201"),
        (IdentifierKind::Alias, "Synthetic alias"),
        (IdentifierKind::Deduplication, "dedup--synthetic-indicator"),
    ] {
        assert_eq!(
            projection.lookup(kind, value),
            Some(&record_ref),
            "{kind:?} lookup should resolve"
        );
    }
}

#[test]
fn update_replaces_stale_identifiers_only_at_the_expected_revision() {
    let original = record(json!({}));
    let updated = record(json!({
        "aliases": ["Updated alias"],
        "external_references": [{
            "source_name": "synthetic",
            "external_id": "SYNTH-UPDATED"
        }]
    }));
    let mut projection = IdentifierProjection::new();
    projection
        .apply(IdentifierTransaction::new("tx-create").upsert(original.projection_record(1), None))
        .expect("create should apply");

    projection
        .apply(
            IdentifierTransaction::new("tx-update").upsert(updated.projection_record(2), Some(1)),
        )
        .expect("revision-matched update should apply");

    assert_eq!(
        projection.lookup(IdentifierKind::Alias, "Synthetic alias"),
        None
    );
    assert_eq!(
        projection.lookup(IdentifierKind::Alias, "Updated alias"),
        Some(&updated.record_ref())
    );
    assert!(matches!(
        projection.apply(
            IdentifierTransaction::new("tx-stale-update")
                .upsert(updated.projection_record(3), Some(1))
        ),
        Err(ProjectionError::RevisionConflict { .. })
    ));
}

#[test]
fn conflicting_transaction_rolls_back_every_identifier_change() {
    let first = record(json!({}));
    let second = record(json!({
        "id": "indicator--00000000-0000-4000-8000-000000000202",
        "internal_id": "internal--00000000-0000-4000-8000-000000000202",
        "standard_id": "indicator--00000000-0000-4000-8000-000000000202",
        "x_opencti_stix_ids": [],
        "i_aliases_ids": [],
        "external_references": [],
        "aliases": ["Synthetic alias"]
    }));
    let mut projection = IdentifierProjection::new();

    let error = projection
        .apply(
            IdentifierTransaction::new("tx-conflict")
                .upsert(first.projection_record(1), None)
                .upsert(second.projection_record(1), None),
        )
        .expect_err("duplicate alias should reject the complete transaction");

    assert!(matches!(error, ProjectionError::IdentifierConflict { .. }));
    assert!(projection.is_empty());
}

#[test]
fn merge_moves_identifiers_to_the_survivor_and_tombstones_sources_atomically() {
    let survivor = record(json!({}));
    let source = record(json!({
        "id": "indicator--00000000-0000-4000-8000-000000000202",
        "internal_id": "internal--00000000-0000-4000-8000-000000000202",
        "standard_id": "indicator--00000000-0000-4000-8000-000000000202",
        "x_opencti_stix_ids": [],
        "i_aliases_ids": ["dedup--merged-source"],
        "external_references": [],
        "aliases": ["Merged source alias"]
    }));
    let merged = record(json!({
        "aliases": ["Synthetic alias", "Merged source alias"],
        "i_aliases_ids": [
            "dedup--synthetic-indicator",
            "dedup--merged-source"
        ]
    }));
    let mut projection = IdentifierProjection::new();
    projection
        .apply(
            IdentifierTransaction::new("tx-create-pair")
                .upsert(survivor.projection_record(1), None)
                .upsert(source.projection_record(1), None),
        )
        .expect("records should be created");

    projection
        .apply(IdentifierTransaction::new("tx-merge").merge(
            merged.projection_record(2),
            Some(1),
            [MergeSource::new(source.record_ref(), 1, 2)],
        ))
        .expect("merge should apply");

    assert_eq!(
        projection.lookup(IdentifierKind::Alias, "Merged source alias"),
        Some(&survivor.record_ref())
    );
    assert_eq!(
        projection.lookup(IdentifierKind::Deduplication, "dedup--merged-source"),
        Some(&survivor.record_ref())
    );
    assert!(projection.is_deleted(&source.record_ref()));
}

#[test]
fn delete_and_transaction_replay_are_idempotent_but_changed_replays_conflict() {
    let mapped = record(json!({}));
    let mut projection = IdentifierProjection::new();
    projection
        .apply(IdentifierTransaction::new("tx-create").upsert(mapped.projection_record(1), None))
        .expect("create should apply");
    let delete = IdentifierTransaction::new("tx-delete").delete(mapped.record_ref(), 1, 2);

    assert_eq!(
        projection
            .apply(delete.clone())
            .expect("delete should apply"),
        ProjectionApply::Applied
    );
    assert_eq!(
        projection
            .apply(delete)
            .expect("identical replay should be accepted"),
        ProjectionApply::Replayed
    );
    assert!(projection.is_deleted(&mapped.record_ref()));
    assert_eq!(
        projection.lookup(IdentifierKind::Alias, "Synthetic alias"),
        None
    );
    assert!(matches!(
        projection.apply(IdentifierTransaction::new("tx-delete").delete(mapped.record_ref(), 2, 3)),
        Err(ProjectionError::TransactionReplayConflict { .. })
    ));
}

#[test]
fn migration_updates_mapping_version_and_rebuild_matches_live_projection() {
    let original = record(json!({"mapping_version": "0.9"}));
    let migrated = record(json!({
        "mapping_version": "1.0",
        "aliases": ["Migrated alias"]
    }));
    let mut live = IdentifierProjection::new();
    live.apply(IdentifierTransaction::new("tx-create").upsert(original.projection_record(1), None))
        .expect("create should apply");
    live.apply(
        IdentifierTransaction::new("tx-migrate").upsert(migrated.projection_record(2), Some(1)),
    )
    .expect("migration update should apply");

    let rebuilt = IdentifierProjection::rebuild([migrated.projection_record(2)])
        .expect("projection should rebuild from canonical records");

    for (kind, value) in [
        (IdentifierKind::Alias, "Migrated alias"),
        (
            IdentifierKind::Internal,
            "internal--00000000-0000-4000-8000-000000000201",
        ),
        (
            IdentifierKind::Standard,
            "indicator--00000000-0000-4000-8000-000000000201",
        ),
    ] {
        assert_eq!(live.lookup(kind, value), rebuilt.lookup(kind, value));
    }
    assert_eq!(live.active_records(), rebuilt.active_records());
}
