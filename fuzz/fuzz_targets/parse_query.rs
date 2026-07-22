#![no_main]
//! Fuzz target: the agent-facing Cypher parser.
//!
//! `parse_query` is the untrusted-input boundary for agent-supplied queries. It
//! must return a typed `Result` for *any* input and never panic, since a panic
//! would be a denial-of-service vector. This target feeds arbitrary UTF-8 text
//! and asserts (via libFuzzer) that the parser never crashes.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = cypher_parser::parse_query(text);
    }
});
