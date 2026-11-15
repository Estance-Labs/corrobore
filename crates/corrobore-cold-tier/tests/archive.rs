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

//! The Lance archive keeps each commit's exact view.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};

use corrobore_cold_tier::{ColdCommit, ColdTier};
use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};

static DIRS: AtomicUsize = AtomicUsize::new(0);

fn archive_dir() -> String {
    let dir = std::env::temp_dir().join(format!(
        "corrobore-cold-tier-{}-{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.display().to_string()
}

fn commit(id: &str, generation: u64) -> ColdCommit {
    ColdCommit {
        commit_id: id.to_owned(),
        branch: "main".to_owned(),
        generation,
        parents: Vec::new(),
        actor: Some("act-test".to_owned()),
        message: None,
        created_at: format!("2026-11-15T00:00:0{generation}.000Z"),
    }
}

/// Three views of a small graph: create, change, delete.
fn history() -> Vec<(ColdCommit, Graph)> {
    let mut graph = Graph::new();
    let alpha = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("alpha".into())),
        )
        .unwrap();
    let beta = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("beta".into())),
        )
        .unwrap();
    let link = graph
        .create_relationship(
            RelationshipInput::new(alpha.clone(), "RELATED_TO", beta.clone()).unwrap(),
        )
        .unwrap();
    let first = graph.clone();
    graph
        .update_node(
            &alpha,
            graph_core::NodePatch::default()
                .set_property("name", PropertyValue::String("alpha-2".into())),
        )
        .unwrap();
    let second = graph.clone();
    // A deleted node takes its relationships with it, as in the store.
    graph.tombstone_relationship(&link).unwrap();
    graph.tombstone_node(&beta).unwrap();
    let third = graph.clone();
    vec![
        (commit("gc01", 1), first),
        (commit("gc02", 2), second),
        (commit("gc03", 3), third),
    ]
}

fn summary(graph: &Graph) -> (BTreeMap<String, String>, usize) {
    let nodes = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .map(|node| {
            let name = match node.property("name") {
                Some(PropertyValue::String(name)) => name.clone(),
                _ => String::new(),
            };
            (node.id().as_str().to_owned(), name)
        })
        .collect();
    (nodes, graph.list_relationships().unwrap().len())
}

#[tokio::test]
async fn each_archived_commit_reads_back_exactly() {
    let tier = ColdTier::new(archive_dir(), HashMap::new());
    let history = history();
    let views: HashMap<String, Graph> = history
        .iter()
        .map(|(commit, graph)| (commit.commit_id.clone(), graph.clone()))
        .collect();
    let commits: Vec<ColdCommit> = history.iter().map(|(commit, _)| commit.clone()).collect();
    let report = tier
        .archive(commits.clone(), |commit| {
            Ok(views[&commit.commit_id].clone())
        })
        .await
        .unwrap();
    assert_eq!(report.archived, 3);
    // Keyframe of 2 nodes, a new alpha version; one relationship.
    assert_eq!(report.node_rows, 3);
    assert_eq!(report.relationship_rows, 1);

    for (commit, graph) in &history {
        let restored = tier.graph_at(&commit.commit_id).await.unwrap().unwrap();
        assert_eq!(summary(&restored), summary(graph), "{}", commit.commit_id);
        assert_eq!(
            restored.list_nodes().unwrap(),
            graph.list_nodes().unwrap(),
            "exact records"
        );
    }
    assert!(tier.graph_at("gc99").await.unwrap().is_none());
    assert_eq!(tier.commit("gc02").await.unwrap().unwrap().generation, 2);
    assert_eq!(
        tier.commits()
            .await
            .unwrap()
            .iter()
            .map(|c| c.commit_id.as_str())
            .collect::<Vec<_>>(),
        ["gc01", "gc02", "gc03"]
    );

    // A second run skips what is archived and adds the rest with its own keyframe.
    let mut later = Graph::new();
    later
        .create_node(
            NodeInput::new(["Note"]).with_property("name", PropertyValue::String("n".into())),
        )
        .unwrap();
    let report = tier
        .archive(vec![commits[2].clone(), commit("gc04", 4)], |commit| {
            assert_eq!(
                commit.commit_id, "gc04",
                "archived commits are not loaded again"
            );
            Ok(later.clone())
        })
        .await
        .unwrap();
    assert_eq!(report.archived, 1);
    assert_eq!(report.already_archived, 1);
    assert_eq!(
        summary(&tier.graph_at("gc04").await.unwrap().unwrap())
            .0
            .len(),
        1
    );
    assert_eq!(
        summary(&tier.graph_at("gc03").await.unwrap().unwrap()),
        summary(&history[2].1)
    );
}

#[tokio::test]
async fn an_empty_archive_has_nothing() {
    let tier = ColdTier::new(archive_dir(), HashMap::new());
    assert!(tier.commits().await.unwrap().is_empty());
    assert!(tier.graph_at("gc01").await.unwrap().is_none());
    let report = tier.archive(Vec::new(), |_| unreachable!()).await.unwrap();
    assert_eq!(report.archived, 0);
    assert!(
        tier.commit("bad'id").await.is_err(),
        "quotes are refused, not escaped"
    );
}
