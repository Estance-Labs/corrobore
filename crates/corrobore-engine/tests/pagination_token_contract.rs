// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use corrobore_engine::{KnowledgeDataErrorCode, PaginationTokenClaims, PaginationTokenCodec};

fn codec() -> PaginationTokenCodec {
    PaginationTokenCodec::new(b"issue-39-test-key-with-at-least-32-bytes")
        .expect("test pagination key should be accepted")
}

fn claims() -> PaginationTokenClaims {
    PaginationTokenClaims {
        version: 1,
        query_fingerprint: "query-fingerprint-1".to_owned(),
        schema_version: "corrobore-graph-v1".to_owned(),
        cursor: "node--000042".to_owned(),
        snapshot_fingerprint: "snapshot--fixture".to_owned(),
        returned: 10,
        policy_version: String::new(),
        access_fingerprint: String::new(),
    }
}

#[test]
fn pagination_token_round_trip_is_opaque_and_versioned() {
    let token = codec().issue(&claims()).expect("token should be issued");
    assert!(!token.contains("node--000042"));
    assert!(!token.contains("query-fingerprint-1"));

    let decoded = codec()
        .verify(&token, "query-fingerprint-1", "corrobore-graph-v1")
        .expect("matching token should verify");
    assert_eq!(decoded, claims());
}

#[test]
fn pagination_token_rejects_tampering() {
    let token = codec().issue(&claims()).expect("token should be issued");
    let mut bytes = token.into_bytes();
    let index = bytes.len() / 2;
    bytes[index] = if bytes[index] == b'a' { b'b' } else { b'a' };
    let tampered = String::from_utf8(bytes).expect("mutated token should remain UTF-8");

    let error = codec()
        .verify(&tampered, "query-fingerprint-1", "corrobore-graph-v1")
        .expect_err("tampered token must fail");
    assert_eq!(error.code, KnowledgeDataErrorCode::InvalidPaginationToken);
}

#[test]
fn pagination_token_rejects_query_and_schema_mismatches() {
    let token = codec().issue(&claims()).expect("token should be issued");

    for (query, schema) in [
        ("different-query", "corrobore-graph-v1"),
        ("query-fingerprint-1", "corrobore-graph-v2"),
    ] {
        let error = codec()
            .verify(&token, query, schema)
            .expect_err("incompatible token must fail");
        assert_eq!(
            error.code,
            KnowledgeDataErrorCode::IncompatiblePaginationToken
        );
    }
}

#[test]
fn pagination_token_rejects_unknown_token_version() {
    let mut unsupported = claims();
    unsupported.version = 2;
    let token = codec()
        .issue(&unsupported)
        .expect("codec should encode claims for compatibility testing");

    let error = codec()
        .verify(&token, "query-fingerprint-1", "corrobore-graph-v1")
        .expect_err("unknown token version must fail");
    assert_eq!(
        error.code,
        KnowledgeDataErrorCode::IncompatiblePaginationToken
    );
}

#[test]
fn pagination_token_key_rejects_short_secrets() {
    let error =
        PaginationTokenCodec::new(b"short").expect_err("short pagination key must be rejected");
    assert_eq!(error.code, KnowledgeDataErrorCode::InvalidRequest);
}
