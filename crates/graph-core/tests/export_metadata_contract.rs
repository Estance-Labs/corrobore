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
use graph_core::{
    ExportMetadata, ExportMode, ExportProfile, GraphError, TransactionId, ValidationReportRef,
};

fn assert_clone_debug<T: Clone + std::fmt::Debug>() {
    let _ = std::any::type_name::<T>();
}

//
// Verify that deterministic export contract primitives are available through
// the graph-core public facade.
//
// Given stable export metadata model types,
// when an integration test imports those types from `graph_core`,
// then the imports should compile without private module access.
#[test]
fn public_facade_exports_export_metadata_contract_types() {
    assert_clone_debug::<ExportMode>();
    assert_clone_debug::<ExportProfile>();
    assert_clone_debug::<ValidationReportRef>();
    assert_clone_debug::<ExportMetadata>();
}

//
// Verify that export metadata captures the deterministic identity tuple required
// to reproduce an export payload.
//
// Given snapshot ID, transaction ID, exporter version, profile, and mode,
// when export metadata is created,
// then the metadata should preserve all values and expose a deterministic key.
#[test]
fn export_metadata_preserves_identity_tuple_and_builds_determinism_key() {
    let transaction_id =
        TransactionId::new("transaction--export-001").expect("transaction ID should be valid");
    let validation_report = ValidationReportRef::new(
        "validation-report--001",
        Some("ruleset--2026-07-06".to_owned()),
    )
    .expect("validation report reference should be valid");

    let metadata = ExportMetadata::new(
        "snapshot--001",
        transaction_id.clone(),
        "stix-mvp-v1",
        ExportProfile::StixMvp,
        ExportMode::Strict,
        Some(validation_report.clone()),
    )
    .expect("export metadata should be valid");

    assert_eq!(metadata.snapshot_id(), "snapshot--001");
    assert_eq!(metadata.transaction_id(), &transaction_id);
    assert_eq!(metadata.exporter_version(), "stix-mvp-v1");
    assert_eq!(metadata.profile(), &ExportProfile::StixMvp);
    assert_eq!(metadata.mode(), ExportMode::Strict);
    assert_eq!(metadata.validation_report(), Some(&validation_report));
    assert_eq!(
        metadata.determinism_key(),
        "snapshot--001|transaction--export-001|stix-mvp-v1|stix-mvp|strict"
    );
}

//
// Verify that determinism identity is stable for equivalent metadata and
// different for profile or mode changes.
//
// Given two equivalent metadata instances and one differing mode,
// when determinism keys are compared,
// then equivalent metadata should produce the same key and differing mode should
// produce a different key.
#[test]
fn determinism_key_is_stable_for_equivalent_identity_and_changes_when_mode_changes() {
    let transaction_id =
        TransactionId::new("transaction--export-002").expect("transaction ID should be valid");

    let strict_a = ExportMetadata::new(
        "snapshot--002",
        transaction_id.clone(),
        "fimi-mvp-v1",
        ExportProfile::FimiJsonMvp,
        ExportMode::Strict,
        None,
    )
    .expect("strict metadata should be valid");
    let strict_b = ExportMetadata::new(
        "snapshot--002",
        transaction_id,
        "fimi-mvp-v1",
        ExportProfile::FimiJsonMvp,
        ExportMode::Strict,
        None,
    )
    .expect("strict metadata should be valid");
    let permissive = ExportMetadata::new(
        "snapshot--002",
        TransactionId::new("transaction--export-002").expect("transaction ID should be valid"),
        "fimi-mvp-v1",
        ExportProfile::FimiJsonMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("permissive metadata should be valid");

    assert_eq!(strict_a.determinism_key(), strict_b.determinism_key());
    assert_ne!(strict_a.determinism_key(), permissive.determinism_key());
}

//
// Verify field-level validation for required non-empty export metadata values.
//
// Given empty or whitespace-only required values,
// when metadata constructors are called,
// then typed field validation errors should be returned.
#[test]
fn export_metadata_rejects_empty_required_fields() {
    let transaction_id =
        TransactionId::new("transaction--export-003").expect("transaction ID should be valid");

    let snapshot_error = ExportMetadata::new(
        " ",
        transaction_id.clone(),
        "stix-mvp-v1",
        ExportProfile::StixMvp,
        ExportMode::Strict,
        None,
    )
    .expect_err("snapshot ID should be required");
    let exporter_error = ExportMetadata::new(
        "snapshot--003",
        transaction_id,
        "",
        ExportProfile::StixMvp,
        ExportMode::Strict,
        None,
    )
    .expect_err("exporter version should be required");
    let report_error = ValidationReportRef::new("\n\t", None)
        .expect_err("validation report ID should be required");

    assert!(matches!(
    snapshot_error,
    GraphError::InvalidExportMetadataField(field) if field == "snapshot_id"
    ));
    assert!(matches!(
    exporter_error,
    GraphError::InvalidExportMetadataField(field) if field == "exporter_version"
    ));
    assert!(matches!(
    report_error,
    GraphError::InvalidExportMetadataField(field) if field == "validation_report_id"
    ));
}
