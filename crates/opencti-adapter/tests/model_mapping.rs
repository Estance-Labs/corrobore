// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use graph_core::{Graph, NodeInput, PropertyValue};
use opencti_adapter::{
    Identifier, IdentifierKind, MappedRecord, MappingVersion, OpenCtiAdapter, RecordFamily,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct FixtureBundle {
    schema_version: u32,
    opencti_version: String,
    source_commit: String,
    records: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    fixture_id: String,
    family: String,
    record: Value,
}

#[derive(Debug, Deserialize)]
struct ParityCorpus {
    fixtures: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct MappingManifest {
    schema_version: u32,
    mapping_version: MappingVersion,
    opencti: MappingOpenCtiLock,
    record_families: MappingFamilies,
    forward_compatibility: Value,
}

#[derive(Debug, Deserialize)]
struct MappingOpenCtiLock {
    version: String,
    source_commit: String,
}

#[derive(Debug, Deserialize)]
struct MappingFamilies {
    objects: Vec<String>,
    relationships: Vec<String>,
}

fn fixtures() -> FixtureBundle {
    serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/model-fixtures.json"
    ))
    .expect("model fixtures should be valid JSON")
}

fn parity_corpus() -> ParityCorpus {
    serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/parity-corpus.json"
    ))
    .expect("parity corpus should be valid JSON")
}

fn mapping_manifest() -> MappingManifest {
    serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/model-mapping.json"
    ))
    .expect("mapping manifest should be valid JSON")
}

fn graph_round_trip(adapter: &OpenCtiAdapter, mapped: &MappedRecord) -> MappedRecord {
    match mapped {
        MappedRecord::Object(object) => {
            let mut graph = Graph::new();
            let node_id = graph
                .create_node(object.to_node_input())
                .expect("mapped object should create a generic node");
            let node = graph
                .get_node(&node_id)
                .expect("node lookup should succeed")
                .expect("created node should exist");
            adapter
                .restore_node(&node)
                .expect("generic node should restore to the OpenCTI record")
        }
        MappedRecord::Relationship(relationship) => {
            let mut graph = Graph::new();
            let source_id = graph
                .create_node(NodeInput::new(["FixtureEndpoint"]))
                .expect("source fixture node should be created");
            let target_id = graph
                .create_node(NodeInput::new(["FixtureEndpoint"]))
                .expect("target fixture node should be created");
            let relationship_id = graph
                .create_relationship(
                    relationship
                        .to_relationship_input(source_id, target_id)
                        .expect("mapped relationship input should be valid"),
                )
                .expect("mapped relationship should create a generic edge");
            let edge = graph
                .get_relationship(&relationship_id)
                .expect("relationship lookup should succeed")
                .expect("created relationship should exist");
            adapter
                .restore_relationship(&edge)
                .expect("generic edge should restore to the OpenCTI record")
        }
    }
}

#[test]
fn pinned_fixture_manifest_covers_every_object_and_relationship_family() {
    let fixtures = fixtures();
    let families = fixtures
        .records
        .iter()
        .map(|fixture| fixture.family.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(fixtures.schema_version, 1);
    assert_eq!(fixtures.opencti_version, "7.260722.0");
    assert_eq!(
        fixtures.source_commit,
        "e41adc1c3fd98a849602db33dbe550f689fe6d83"
    );
    assert_eq!(
        families,
        BTreeSet::from([
            "internal_object",
            "internal_relationship",
            "stix_core_relationship",
            "stix_cyber_observable",
            "stix_domain_object",
            "stix_meta_object",
            "stix_ref_relationship",
            "stix_sighting_relationship",
            "unknown_object",
            "unknown_relationship",
        ])
    );
}

#[test]
fn mapping_contract_is_independently_versioned_and_pinned_to_the_source_lock() {
    let manifest = mapping_manifest();

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.mapping_version, MappingVersion::CURRENT);
    assert_eq!(manifest.opencti.version, "7.260722.0");
    assert_eq!(
        OpenCtiAdapter::pinned().opencti_version(),
        manifest.opencti.version
    );
    assert_eq!(
        manifest.opencti.source_commit,
        "e41adc1c3fd98a849602db33dbe550f689fe6d83"
    );
    assert_eq!(
        manifest
            .record_families
            .objects
            .into_iter()
            .chain(manifest.record_families.relationships)
            .collect::<BTreeSet<_>>(),
        fixtures()
            .records
            .into_iter()
            .map(|fixture| fixture.family)
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        manifest.forward_compatibility["unknown_type_policy"],
        "preserve_as_generic_typed_record"
    );
}

#[test]
fn every_fixture_round_trips_through_generic_graph_records_without_field_loss() {
    let adapter = OpenCtiAdapter::pinned();

    for fixture in fixtures().records {
        let mapped = adapter
            .map(fixture.record.clone())
            .unwrap_or_else(|error| panic!("{} should map: {error}", fixture.fixture_id));

        assert_eq!(mapped.raw(), &fixture.record, "{}", fixture.fixture_id);
        assert_eq!(
            mapped.family().as_str(),
            fixture.family,
            "{}",
            fixture.fixture_id
        );
        assert_eq!(mapped.mapping_version(), MappingVersion::CURRENT);

        let restored = graph_round_trip(&adapter, &mapped);

        assert_eq!(
            restored.raw(),
            &fixture.record,
            "{} lost fields during graph round-trip",
            fixture.fixture_id
        );
    }
}

#[test]
fn every_existing_compatibility_fixture_maps_and_round_trips_losslessly() {
    let adapter = OpenCtiAdapter::pinned();

    for raw in parity_corpus().fixtures {
        let fixture_id = raw["id"].as_str().unwrap_or("<missing id>");
        let mapped = adapter
            .map(raw.clone())
            .unwrap_or_else(|error| panic!("{fixture_id} should map: {error}"));
        let expected_family = match fixture_id {
            "relationship--00000000-0000-4000-8000-000000000060"
            | "relationship--00000000-0000-4000-8000-000000000061" => {
                Some(RecordFamily::StixCoreRelationship)
            }
            "relationship--00000000-0000-4000-8000-000000000062" => {
                Some(RecordFamily::StixRefRelationship)
            }
            "relationship--00000000-0000-4000-8000-000000000063" => {
                Some(RecordFamily::InternalRelationship)
            }
            _ => None,
        };
        if let Some(expected_family) = expected_family {
            assert_eq!(
                mapped.family(),
                expected_family,
                "{fixture_id} should keep its pinned relationship family"
            );
        }
        let restored = graph_round_trip(&adapter, &mapped);

        assert_eq!(
            restored.raw(),
            &raw,
            "{fixture_id} lost fields during graph round-trip"
        );
    }
}

#[test]
fn scalar_lists_nested_extensions_and_timestamps_keep_their_types() {
    let fixture = fixtures()
        .records
        .into_iter()
        .find(|fixture| fixture.fixture_id == "stix-domain-object")
        .expect("domain fixture should exist");
    let adapter = OpenCtiAdapter::pinned();
    let mapped = adapter
        .map(fixture.record.clone())
        .expect("fixture should map");
    let object = mapped.as_object().expect("fixture should be an object");
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(object.to_node_input())
        .expect("mapped object should create a node");
    let node = graph
        .get_node(&node_id)
        .expect("node lookup should succeed")
        .expect("created node should exist");

    assert_eq!(
        node.property("opencti.raw"),
        Some(&PropertyValue::Json(fixture.record.clone()))
    );
    assert_eq!(
        mapped.timestamps().created.as_deref(),
        Some("2026-01-01T00:00:00.000Z")
    );
    assert_eq!(
        mapped.timestamps().updated_at.as_deref(),
        Some("2026-01-02T00:00:01.000Z")
    );
    assert_eq!(
        mapped.raw()["x_opencti_extension"]["confidence_history"][0]["score"],
        json!(70)
    );
}

#[test]
fn identifiers_references_provenance_and_access_inputs_are_extracted_without_enforcement() {
    let fixture = fixtures()
        .records
        .into_iter()
        .find(|fixture| fixture.fixture_id == "stix-domain-object")
        .expect("domain fixture should exist");
    let mapped = OpenCtiAdapter::pinned()
        .map(fixture.record)
        .expect("fixture should map");

    for identifier in [
        Identifier::new(
            IdentifierKind::Internal,
            "internal--00000000-0000-4000-8000-000000000101",
        ),
        Identifier::new(
            IdentifierKind::Standard,
            "attack-pattern--00000000-0000-4000-8000-000000000101",
        ),
        Identifier::new(
            IdentifierKind::Stix,
            "attack-pattern--00000000-0000-4000-8000-000000000100",
        ),
        Identifier::new(IdentifierKind::External, "CAPEC-98"),
        Identifier::new(IdentifierKind::Alias, "Synthetic phishing"),
        Identifier::new(IdentifierKind::Alias, "Documentation lure"),
        Identifier::new(IdentifierKind::Alias, "dedup--credential-phishing"),
        Identifier::new(IdentifierKind::Deduplication, "dedup--credential-phishing"),
    ] {
        assert!(
            mapped
                .identifiers()
                .contains(&identifier.expect("identifier should be valid")),
            "missing identifier"
        );
    }

    assert_eq!(
        mapped.access().marking_ids,
        ["marking-definition--00000000-0000-4000-8000-000000000111"]
    );
    assert_eq!(
        mapped.access().organization_ids,
        ["identity--00000000-0000-4000-8000-000000000112"]
    );
    assert_eq!(
        mapped.access().tenant_ids,
        [
            "grouping--00000000-0000-4000-8000-000000000115",
            "grouping--00000000-0000-4000-8000-000000000118"
        ]
    );
    assert_eq!(
        mapped.access().creator_ids,
        ["identity--00000000-0000-4000-8000-000000000110"]
    );
    assert_eq!(
        mapped.access().owner_ids,
        [
            "identity--00000000-0000-4000-8000-000000000116",
            "identity--00000000-0000-4000-8000-000000000117"
        ]
    );
    assert_eq!(mapped.access().authorized_members.len(), 2);
    assert_eq!(
        mapped.access().sharing_policy,
        Some(json!({
            "allowed": ["identity--00000000-0000-4000-8000-000000000112"],
            "mode": "restricted"
        }))
    );
    assert!(
        mapped
            .references()
            .iter()
            .any(|reference| reference.field == "created_by_ref")
    );
    assert_eq!(mapped.provenance().external_references.len(), 1);
}

#[test]
fn relationship_direction_type_properties_markings_and_history_are_preserved() {
    let fixture = fixtures()
        .records
        .into_iter()
        .find(|fixture| fixture.fixture_id == "stix-core-relationship")
        .expect("core relationship fixture should exist");
    let mapped = OpenCtiAdapter::pinned()
        .map(fixture.record.clone())
        .expect("fixture should map");
    let relationship = mapped
        .as_relationship()
        .expect("fixture should map as a relationship");

    assert_eq!(relationship.relationship_type(), "uses");
    assert_eq!(
        relationship.source_ref(),
        "threat-actor--00000000-0000-4000-8000-000000000151"
    );
    assert_eq!(
        relationship.target_ref(),
        "malware--00000000-0000-4000-8000-000000000152"
    );
    assert_eq!(
        relationship.raw()["history_changes"][0]["field"],
        json!("confidence")
    );
    assert_eq!(
        relationship.access().marking_ids,
        ["marking-definition--00000000-0000-4000-8000-000000000111"]
    );
}

#[test]
fn future_types_are_preserved_as_generic_opencti_objects_and_never_identity() {
    let fixture = fixtures()
        .records
        .into_iter()
        .find(|fixture| fixture.fixture_id == "future-object")
        .expect("future fixture should exist");
    let mapped = OpenCtiAdapter::pinned()
        .map(fixture.record.clone())
        .expect("future types should use generic preservation");
    let object = mapped
        .as_object()
        .expect("future record should be an object");
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(object.to_node_input())
        .expect("future object should create a generic node");
    let node = graph
        .get_node(&node_id)
        .expect("node lookup should succeed")
        .expect("created node should exist");

    assert_eq!(mapped.family(), RecordFamily::UnknownObject);
    assert!(node.has_label("OpenCtiObject"));
    assert!(node.has_label("OpenCtiUnknownObject"));
    assert!(!node.has_label("Identity"));
    assert_eq!(
        node.property("opencti.entity_type"),
        Some(&PropertyValue::String("Future-OpenCTI-Type".to_owned()))
    );
    assert_eq!(
        adapter_round_trip(&mapped),
        fixture.record,
        "future extension fields must remain intact"
    );
}

#[test]
fn malformed_records_are_rejected_explicitly() {
    let adapter = OpenCtiAdapter::pinned();

    for invalid in [
        json!({"id": "indicator--missing-type"}),
        json!({"type": "indicator"}),
        json!({
            "id": "relationship--missing-target",
            "type": "relationship",
            "relationship_type": "uses",
            "source_ref": "threat-actor--source"
        }),
    ] {
        assert!(
            adapter.map(invalid).is_err(),
            "malformed record should be rejected"
        );
    }
}

fn adapter_round_trip(mapped: &MappedRecord) -> Value {
    mapped.raw().clone()
}
