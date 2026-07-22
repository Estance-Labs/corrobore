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
use cypher_parser::{ParseErrorCode, mvp_supported_clauses, parse_query};

#[test]
fn mvp_supported_clauses_matrix_is_deterministic() {
    let clauses = mvp_supported_clauses();

    assert_eq!(
        clauses,
        vec![
            "MATCH",
            "OPTIONAL MATCH",
            "WHERE",
            "RETURN",
            "WITH",
            "ORDER BY",
            "SKIP",
            "LIMIT",
            "DISTINCT",
            "COUNT",
            "SUM",
            "AVG",
            "MIN",
            "MAX",
            "CREATE",
            "MERGE",
            "SET",
            "REMOVE",
            "DELETE",
        ]
    );
}

#[test]
fn parse_query_rejects_unwind_clause_as_unsupported_mvp_feature() {
    let error = parse_query("UNWIND [1, 2, 3] AS x RETURN x")
        .expect_err("UNWIND should be rejected in MVP subset");

    assert_eq!(error.code, ParseErrorCode::UnsupportedFeature);
    assert!(error.message.contains("UNWIND"));
    assert!(error.suggestion.is_some());
}

#[test]
fn parse_query_rejects_detach_delete_by_default() {
    let error = parse_query("MATCH (n) DETACH DELETE n")
        .expect_err("DETACH DELETE should be rejected by default");

    assert_eq!(error.code, ParseErrorCode::UnsupportedFeature);
    assert!(error.message.contains("DETACH DELETE"));
    assert!(error.suggestion.is_some());
}
