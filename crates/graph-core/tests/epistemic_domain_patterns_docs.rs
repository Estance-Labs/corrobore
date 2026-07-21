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
const EPISTEMIC_DOMAIN_PATTERNS_DOC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../project-documents/feature-0007-evidence-domain-model/artifacts/0005-epistemic-domain-claim-patterns.md"
));

fn assert_contains_all(document: &str, expected_fragments: &[&str]) {
    for expected_fragment in expected_fragments {
        assert!(
            document.contains(expected_fragment),
            "expected epistemic domain patterns documentation to contain fragment: {expected_fragment}"
        );
    }
}

#[test]
fn epistemic_patterns_doc_exists_and_references_epic_0005() {
    assert_contains_all(
        EPISTEMIC_DOMAIN_PATTERNS_DOC,
        &[
            "# Epistemic Domain Claim Patterns for CTI, FIMI, and Crisis",
            ": Epistemic Claim Graph Foundation",
            "CTI Claim Patterns",
            "FIMI Claim Patterns",
            "Crisis Claim Patterns",
        ],
    );
}

#[test]
fn cti_patterns_include_required_examples() {
    assert_contains_all(
        EPISTEMIC_DOMAIN_PATTERNS_DOC,
        &[
            "attribution",
            "infrastructure ownership",
            "malware usage",
            "indicator validity",
            "exploit targeting",
            "supporting evidence",
            "refuting evidence",
            "contradiction",
            "supersession",
            "agent stance",
        ],
    );
}

#[test]
fn fimi_patterns_include_required_examples() {
    assert_contains_all(
        EPISTEMIC_DOMAIN_PATTERNS_DOC,
        &[
            "narrative promotion",
            "coordinated amplification",
            "source attribution",
            "target audience",
            "campaign linkage",
            "supporting evidence",
            "refuting evidence",
            "contradiction",
            "supersession",
            "agent stance",
        ],
    );
}

#[test]
fn crisis_patterns_include_required_examples() {
    assert_contains_all(
        EPISTEMIC_DOMAIN_PATTERNS_DOC,
        &[
            "event occurrence",
            "location",
            "severity",
            "source reliability",
            "humanitarian impact",
            "supporting evidence",
            "refuting evidence",
            "contradiction",
            "supersession",
            "agent stance",
        ],
    );
}

#[test]
fn doc_covers_non_goals_open_questions_and_test_guidance() {
    assert_contains_all(
        EPISTEMIC_DOMAIN_PATTERNS_DOC,
        &[
            "## Non-Goals",
            "## Open Questions",
            "production ontology implementation",
            "guide acceptance tests",
        ],
    );
}
