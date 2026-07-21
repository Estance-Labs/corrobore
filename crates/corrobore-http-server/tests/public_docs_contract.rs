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
use std::{fs, path::Path};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn public_http_docs_cover_registered_routes_and_configuration() {
    let root = workspace_root();
    let app = read(&root.join("crates/corrobore-http-server/src/app.rs"));
    let config = read(&root.join("crates/corrobore-http-server/src/config.rs"));
    let guide = read(&root.join("docs/user-guide/http-server.md"));
    let openapi = read(&root.join("docs/api/openapi.yaml"));

    for line in app.lines() {
        let Some(route) = line
            .split(".route(\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
        else {
            continue;
        };
        assert!(
            guide.contains(route),
            "HTTP guide does not document registered route {route}"
        );
        assert!(
            openapi.contains(&format!("  {route}:")),
            "OpenAPI does not document registered route {route}"
        );
    }

    let mut variables = config
        .split('"')
        .filter(|value| value.starts_with("CORROBORE_HTTP_"))
        .collect::<Vec<_>>();
    variables.sort_unstable();
    variables.dedup();

    for variable in variables {
        assert!(
            guide.contains(variable),
            "HTTP guide does not document configuration variable {variable}"
        );
    }
}

#[test]
fn public_docs_use_current_identity_and_resolve_internal_links() {
    let root = workspace_root();
    let docs = root.join("docs");
    let mut pending = vec![docs.clone()];

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        {
            let entry = entry.expect("documentation entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("md" | "yaml" | "yml")
            ) {
                continue;
            }

            let content = read(&path);
            assert!(
                !content.contains("Agentic Intelligence Graph Engine"),
                "{} still uses the former product name",
                path.display()
            );
            assert!(
                !content.contains("AreDee-Bangs/intelligence-graph-engine"),
                "{} still uses the former repository URL",
                path.display()
            );

            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }

            for target in markdown_link_targets(&content) {
                if target.is_empty()
                    || target.starts_with('#')
                    || target.starts_with("http://")
                    || target.starts_with("https://")
                    || target.starts_with("mailto:")
                {
                    continue;
                }
                let target = target
                    .split('#')
                    .next()
                    .expect("split always yields one item");
                let resolved = path
                    .parent()
                    .expect("documentation file has a parent")
                    .join(target);
                assert!(
                    resolved.exists(),
                    "{} links to missing target {target}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn public_docs_load_the_corrobore_brand_system() {
    let root = workspace_root();
    let config = read(&root.join("mkdocs.yml"));
    let stylesheet = read(&root.join("docs/stylesheets/corrobore.css"));

    for expected in [
        "logo: assets/corrobore-mark.svg",
        "favicon: assets/corrobore-mark.svg",
        "extra_css:",
        "- stylesheets/corrobore.css",
        "scheme: corrobore-dark",
        "scheme: corrobore-light",
    ] {
        assert!(
            config.contains(expected),
            "MkDocs configuration should include {expected}"
        );
    }

    for expected in [
        "--corrobore-acid: #b8f45c",
        "--corrobore-mint: #75e3b7",
        "--corrobore-bg: #07110e",
        "--corrobore-font-sans: \"Manrope\"",
        "--corrobore-font-mono: \"DM Mono\"",
        "prefers-reduced-motion: reduce",
        "@media screen and (max-width: 76.234375em)",
    ] {
        assert!(
            stylesheet.contains(expected),
            "brand stylesheet should include {expected}"
        );
    }

    assert!(root.join("docs/assets/corrobore-mark.svg").is_file());
}

#[test]
fn public_repository_validates_provider_abi_without_publishing_ee_binaries() {
    let root = workspace_root();
    let workflow = read(&root.join(".github/workflows/ee-domain-binaries.yml"));

    assert!(workflow.contains("cargo test -p domain-provider-abi --locked"));
    assert!(workflow.contains("corrobore_domain_provider.h"));
    assert!(!workflow.contains("gh release"));
    assert!(!workflow.contains("corrobore-domain-cti"));
    assert!(!workflow.contains("corrobore-domain-fimi"));
    assert!(!workflow.contains("corrobore-domain-crisis"));
}

fn markdown_link_targets(content: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("](") {
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find(')') else {
            break;
        };
        targets.push(&remaining[..end]);
        remaining = &remaining[end + 1..];
    }
    targets
}
