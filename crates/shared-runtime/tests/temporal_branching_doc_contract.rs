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
use std::fs;
use std::path::PathBuf;

fn epic_0014_doc_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("project-documents")
        .join("feature-0014-temporal-cypher-branching")
        .join("artifacts")
        .join("0014-advanced-temporal-cypher-snapshot-branching-and-gql-alignment.md")
}

#[test]
fn epic_0014_document_includes_required_sections_for_issue_121_contract() {
    let contents = fs::read_to_string(epic_0014_doc_path())
        .expect("epic 0014 markdown artifact should be readable");

    let required_sections = [
        "## Status",
        "## GitHub tracker",
        "## PRD coverage",
        "## Summary",
        "## Goals",
        "## Non-goals",
        "## Acceptance criteria",
        "## Related epics",
        "## Candidate issue breakdown",
        "## Definition of done",
    ];

    for section in required_sections {
        assert!(
            contents.contains(section),
            "missing required section in artifact: {section}"
        );
    }
}
