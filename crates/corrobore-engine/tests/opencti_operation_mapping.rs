// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use corrobore_engine::OperationKind;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct InventoryOperation {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ContractMapping {
    operation_id: String,
    contract_operation: Option<OperationKind>,
    support: String,
    reason: Option<String>,
}

#[test]
fn every_catalogued_opencti_operation_maps_to_a_typed_or_unsupported_capability() {
    let inventory: Vec<InventoryOperation> = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/operations.json"
    ))
    .expect("operation inventory should parse");
    let mappings: Vec<ContractMapping> = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/knowledge-data-engine-mapping.json"
    ))
    .expect("contract mapping should parse");

    let inventory_ids: BTreeSet<_> = inventory
        .into_iter()
        .map(|operation| operation.id)
        .collect();
    let mapping_ids: BTreeSet<_> = mappings
        .iter()
        .map(|mapping| mapping.operation_id.clone())
        .collect();
    assert_eq!(mapping_ids, inventory_ids);

    for mapping in mappings {
        match mapping.support.as_str() {
            "typed" => {
                assert!(
                    mapping
                        .contract_operation
                        .is_some_and(|operation| OperationKind::ALL.contains(&operation))
                );
                assert!(mapping.reason.is_none());
            }
            "unsupported" => assert!(
                mapping.contract_operation.is_none()
                    && mapping
                        .reason
                        .as_deref()
                        .is_some_and(|reason| !reason.trim().is_empty()),
                "unsupported operation {} must explain why",
                mapping.operation_id
            ),
            other => panic!("unknown support classification: {other}"),
        }
    }
}
