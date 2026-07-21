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
use std::{fs, path::PathBuf};

fn repository_file(path: &str) -> String {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(crate_dir.join("../..").join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn epic_0024_report_links_implementation_and_complete_acceptance_evidence() {
    let report = repository_file(
        "project-documents/feature-0024-temporal-graph-explorer/artifacts/0024-3d-temporal-graph-explorer.md",
    );

    for issue in [
        "#293", "#296", "#297", "#298", "#299", "#300", "#301", "#302",
    ] {
        assert!(report.contains(issue), "Epic report must link {issue}");
    }
    for heading in [
        "## Status",
        "## Acceptance evidence",
        "## Validated ceilings",
        "## Reproducibility",
        "## Accessibility",
        "## Complete validation matrix",
    ] {
        assert!(
            report.contains(heading),
            "Epic report must contain {heading}"
        );
    }
    assert!(report.contains("Implemented and accepted"));
}

#[test]
fn public_explorer_docs_define_backend_contract_and_external_repo_location() {
    let public_guide = repository_file("docs/user-guide/http-server.md");

    for required in [
        "Noetance-Labs/corrobore-web",
        "GET /v1/explorer/sessions",
        "GET /v1/explorer/sessions/{session_id}/timeline",
        "GET /v1/explorer/sessions/{session_id}/graph",
        "TEMPORAL_BOUNDARY_NOT_FOUND",
    ] {
        assert!(
            public_guide.contains(required),
            "public explorer documentation must describe {required}"
        );
    }
}
