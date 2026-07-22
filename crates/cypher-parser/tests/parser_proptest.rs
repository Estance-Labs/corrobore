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
//! Property-based tests for the Cypher parser (audit §6 — testing depth).
//!
//! The parser exposes no renderer, so a literal `parse -> render -> parse`
//! round-trip is not expressible. Instead we validate the invariants the parser
//! *does* guarantee:
//!
//! 1. **No panics.** `parse_query` must return a typed `Result` for any input,
//!    including arbitrary/adversarial byte-ish strings. A panic here would be a
//!    denial-of-service vector on the agent-facing query surface.
//! 2. **Determinism.** Parsing the same text twice yields identical output.
//! 3. **Whitespace idempotence.** Because `parse_query` normalizes runs of
//!    whitespace, two textually different but whitespace-equivalent queries must
//!    produce byte-identical ASTs.
//! 4. **Structural correctness.** Generated valid MVP queries parse successfully
//!    and expose the clause structure the grammar promises.

#![allow(clippy::unwrap_used)]

use cypher_parser::{ClauseKind, QueryKind, parse_query};
use proptest::prelude::*;

/// Lowercase Cypher identifier (bound variable name).
fn identifier() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,7}".prop_map(|s| s)
}

/// Capitalized node label.
fn label() -> impl Strategy<Value = String> {
    "[A-Z][A-Za-z]{0,9}".prop_map(|s| s)
}

/// A small non-negative integer used for `LIMIT` / comparison literals.
fn small_int() -> impl Strategy<Value = u32> {
    0u32..1000
}

/// Build the token stream for a valid read query, returning the individual
/// tokens so tests can re-join them with arbitrary whitespace.
fn read_query_tokens(var: &str, lbl: &str, limit: u32) -> Vec<String> {
    vec![
        "MATCH".to_owned(),
        format!("({var}:{lbl})"),
        "WHERE".to_owned(),
        format!("{var}.score"),
        ">".to_owned(),
        limit.to_string(),
        "RETURN".to_owned(),
        var.to_owned(),
    ]
}

/// Build the token stream for a valid create query.
fn create_query_tokens(var: &str, lbl: &str) -> Vec<String> {
    vec!["CREATE".to_owned(), format!("({var}:{lbl})")]
}

proptest! {
    // Adversarial input must never panic the parser.
    #[test]
    fn parse_query_never_panics_on_arbitrary_input(text in ".*") {
        let _ = parse_query(&text);
    }

    // Arbitrary input parses deterministically.
    #[test]
    fn parse_query_is_deterministic(text in ".*") {
        prop_assert_eq!(parse_query(&text), parse_query(&text));
    }

    // Collapsing whitespace-equivalent forms yields identical ASTs. We only vary
    // whitespace at token boundaries we control (never inside literals), so the
    // canonical single-space form and a padded form must parse identically.
    #[test]
    fn parse_query_is_whitespace_idempotent(
        var in identifier(),
        lbl in label(),
        limit in small_int(),
        pads in proptest::collection::vec(1usize..4, 7),
    ) {
        let tokens = read_query_tokens(&var, &lbl, limit);
        let canonical = tokens.join(" ");

        // Re-join with variable-width whitespace runs between tokens, plus
        // leading/trailing padding.
        let mut padded = String::from("  ");
        for (index, token) in tokens.iter().enumerate() {
            if index > 0 {
                let width = pads.get(index - 1).copied().unwrap_or(1);
                padded.push_str(&" ".repeat(width));
            }
            padded.push_str(token);
        }
        padded.push_str("   ");

        prop_assert_eq!(parse_query(&canonical), parse_query(&padded));
    }

    // Generated valid read queries expose the promised clause structure.
    #[test]
    fn generated_read_query_parses_with_expected_structure(
        var in identifier(),
        lbl in label(),
        limit in small_int(),
    ) {
        let query = read_query_tokens(&var, &lbl, limit).join(" ");
        let ast = parse_query(&query).expect("generated read query should parse");

        prop_assert_eq!(ast.kind, QueryKind::Read);
        prop_assert!(ast.clauses.contains(&ClauseKind::Match));
        prop_assert!(ast.clauses.contains(&ClauseKind::Where));
        prop_assert!(ast.clauses.contains(&ClauseKind::Return));
    }

    // Generated valid create queries are classified as mutations with a CREATE
    // clause.
    #[test]
    fn generated_create_query_parses_as_mutation(
        var in identifier(),
        lbl in label(),
    ) {
        let query = create_query_tokens(&var, &lbl).join(" ");
        let ast = parse_query(&query).expect("generated create query should parse");

        prop_assert!(ast.clauses.contains(&ClauseKind::Create));
        prop_assert_eq!(ast.kind, QueryKind::Mutation);
    }
}
