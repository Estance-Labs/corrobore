// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use opencti_adapter::{OpenCtiAdapter, RecordFamily};
use serde_json::json;

#[test]
fn standard_stix_file_is_an_exportable_cyber_observable() {
    let mapped = OpenCtiAdapter::pinned()
        .map(json!({
            "type": "file",
            "id": "file--00000000-0000-4000-8000-000000000001",
            "name": "synthetic.bin",
            "hashes": {"SHA-256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        }))
        .expect("standard STIX file must map");

    assert_eq!(mapped.family(), RecordFamily::StixCyberObservable);
}

#[test]
fn explicitly_parented_opencti_file_remains_internal() {
    let mapped = OpenCtiAdapter::pinned()
        .map(json!({
            "entity_type": "File",
            "internal_id": "file--opencti-internal",
            "parent_types": ["Internal-Object"],
            "name": "attachment.pdf"
        }))
        .expect("OpenCTI internal file must map");

    assert_eq!(mapped.family(), RecordFamily::InternalObject);
}
