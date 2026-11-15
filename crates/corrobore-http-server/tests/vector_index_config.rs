// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #342: the vector index policy is configured through
//! `CORROBORE_STORAGE_VECTOR_INDEX` and `CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use corrobore_http_server::ServerConfig;
use graph_storage::{VectorIndexKind, VectorIndexMode, VectorIndexPolicy};

fn vars(extra: &[(&str, &str)]) -> HashMap<String, String> {
    let mut values = HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "token-342".to_owned(),
    )]);
    for (name, value) in extra {
        values.insert((*name).to_owned(), (*value).to_owned());
    }
    values
}

#[test]
fn vector_index_defaults_to_exact_below_ten_thousand_vectors() {
    let config = ServerConfig::from_map(&vars(&[])).unwrap();
    assert_eq!(config.storage_vector_index, VectorIndexMode::Auto);
    assert_eq!(config.storage_vector_hnsw_min_vectors, 10_000);
    let policy = config.vector_index_policy();
    assert_eq!(policy, VectorIndexPolicy::default());
    assert_eq!(policy.kind_for(9_999), VectorIndexKind::Exact);
    assert_eq!(policy.kind_for(10_000), VectorIndexKind::Hnsw);
}

#[test]
fn vector_index_mode_and_threshold_are_overridable() {
    let config = ServerConfig::from_map(&vars(&[
        ("CORROBORE_STORAGE_VECTOR_INDEX", "HNSW"),
        ("CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS", "500"),
    ]))
    .unwrap();
    assert_eq!(config.storage_vector_index, VectorIndexMode::Hnsw);
    assert_eq!(config.storage_vector_hnsw_min_vectors, 500);
    let policy = config.vector_index_policy();
    assert_eq!(policy.mode, VectorIndexMode::Hnsw);
    assert_eq!(policy.hnsw_min_vectors, 500);

    let exact = ServerConfig::from_map(&vars(&[("CORROBORE_STORAGE_VECTOR_INDEX", "exact")]))
        .unwrap()
        .vector_index_policy();
    assert_eq!(exact.kind_for(usize::MAX), VectorIndexKind::Exact);
}

#[test]
fn invalid_vector_index_settings_are_rejected() {
    assert_eq!(
        ServerConfig::from_map(&vars(&[("CORROBORE_STORAGE_VECTOR_INDEX", "ivf")]))
            .unwrap_err()
            .to_string(),
        "invalid environment variable CORROBORE_STORAGE_VECTOR_INDEX: ivf"
    );
    for value in ["0", "-1", "many"] {
        assert_eq!(
            ServerConfig::from_map(&vars(&[(
                "CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS",
                value
            )]))
            .unwrap_err()
            .to_string(),
            format!(
                "invalid environment variable CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS: {value}"
            )
        );
    }
}
