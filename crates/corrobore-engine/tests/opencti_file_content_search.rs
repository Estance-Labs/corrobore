// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, fs, path::PathBuf};

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CorroboreEngine,
    CorroboreKnowledgeDataProvider, EnginePersistence, KnowledgeDataOperation,
    KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse,
    PreparedKnowledgeDataProjection, RequestContext, SearchRequest,
    file_content_query_from_search_request,
};
use graph_core::Graph;
use opencti_access::AccessMetadata;
use opencti_file_search::{
    ExtractionLimits, FileContentIndex, FileContentIndexSettings, FileDescriptor,
    FileExtractionRequest, extract_file,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const CURSOR_KEY: &[u8] = b"issue-48-kde-file-content-key-v1";

#[derive(Debug)]
struct FileContentPersistence {
    index: FileContentIndex,
}

impl EnginePersistence for FileContentPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }

    fn prepare_knowledge_data_operation(
        &mut self,
        operation: &KnowledgeDataOperation,
        access: &AccessContext,
    ) -> Result<Option<PreparedKnowledgeDataProjection>, String> {
        let KnowledgeDataOperation::Search(request) = operation else {
            return Ok(None);
        };
        let Some(query) =
            file_content_query_from_search_request(request).map_err(|error| error.message)?
        else {
            return Ok(None);
        };
        let page = self
            .index
            .search(&query, access)
            .map_err(|error| error.to_string())?;
        Ok(Some(PreparedKnowledgeDataProjection {
            graph: Graph::new(),
            page_ins: 0,
            cache_hits: 0,
            authorization_denials: page.authorization_denials,
            full_text_page: Some(page),
        }))
    }
}

fn root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "corrobore-issue-48-kde-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn knowledge_data_search_routes_file_content_without_graph_hydration_or_access_leakage() {
    let root = root();
    let content = b"Synthetic report referencing malware.example.org".to_vec();
    let artifact = extract_file(
        FileExtractionRequest {
            descriptor: FileDescriptor {
                file_id: "file--00000000-0000-4000-8000-000000000070".to_owned(),
                source_object_id: "report--synthetic".to_owned(),
                blob_key: "opencti/report.txt".to_owned(),
                name: "synthetic-report.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                content_hash: format!("{:x}", Sha256::digest(&content)),
                version: 1,
                access: AccessMetadata {
                    marking_ids: vec!["marking--clear".to_owned()],
                    ..AccessMetadata::default()
                },
            },
            content,
        },
        &ExtractionLimits {
            max_input_bytes: 100_000,
            max_extracted_bytes: 100_000,
            max_pages: 10,
            max_sheets: 10,
            max_rows_per_sheet: 100,
            max_cells: 1_000,
            max_chunks: 100,
            max_chunk_chars: 4_096,
        },
    )
    .unwrap();
    let second_content =
        b"<html><body>Synthetic report referencing malware.example.org</body></html>".to_vec();
    let second_artifact = extract_file(
        FileExtractionRequest {
            descriptor: FileDescriptor {
                file_id: "file--00000000-0000-4000-8000-000000000071".to_owned(),
                source_object_id: "report--synthetic".to_owned(),
                blob_key: "opencti/report.html".to_owned(),
                name: "synthetic-report.html".to_owned(),
                mime_type: "text/html".to_owned(),
                content_hash: format!("{:x}", Sha256::digest(&second_content)),
                version: 1,
                access: AccessMetadata {
                    marking_ids: vec!["marking--clear".to_owned()],
                    ..AccessMetadata::default()
                },
            },
            content: second_content,
        },
        &ExtractionLimits {
            max_input_bytes: 100_000,
            max_extracted_bytes: 100_000,
            max_pages: 10,
            max_sheets: 10,
            max_rows_per_sheet: 100,
            max_cells: 1_000,
            max_chunks: 100,
            max_chunk_chars: 4_096,
        },
    )
    .unwrap();
    let denied_content = b"Hidden malware.example.org source".to_vec();
    let denied_artifact = extract_file(
        FileExtractionRequest {
            descriptor: FileDescriptor {
                file_id: "file--inaccessible".to_owned(),
                source_object_id: "report--inaccessible".to_owned(),
                blob_key: "opencti/hidden.txt".to_owned(),
                name: "hidden.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                content_hash: format!("{:x}", Sha256::digest(&denied_content)),
                version: 1,
                access: AccessMetadata {
                    marking_ids: vec!["marking--amber".to_owned()],
                    ..AccessMetadata::default()
                },
            },
            content: denied_content,
        },
        &ExtractionLimits::default(),
    )
    .unwrap();
    let index = FileContentIndex::open(
        root.clone(),
        FileContentIndexSettings::testing(CURSOR_KEY.to_vec()),
    )
    .unwrap();
    index
        .rebuild(vec![artifact, second_artifact, denied_artifact])
        .unwrap();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(FileContentPersistence { index }))
        .build()
        .unwrap();
    let response = CorroboreKnowledgeDataProvider::new(&mut engine, CURSOR_KEY)
        .unwrap()
        .execute(KnowledgeDataRequest {
            contract_version: ContractVersion::CURRENT,
            context: RequestContext {
                request_id: "request--file-content".to_owned(),
                correlation_id: "correlation--file-content".to_owned(),
                idempotency_key: None,
                deadline_unix_ms: None,
                cancellation_id: None,
                access: AccessContext {
                    subject_id: "user--clear".to_owned(),
                    marking_ids: vec!["marking--clear".to_owned()],
                    attributes: BTreeMap::from([(
                        "policy_version".to_owned(),
                        "policy--v1".to_owned(),
                    )]),
                    ..AccessContext::default()
                },
                consistency: ConsistencyLevel::Snapshot,
            },
            operation: KnowledgeDataOperation::Search(SearchRequest {
                expression: json!({
                    "text": "malware.example.org",
                    "content": true
                }),
                limit: 10,
            }),
        });
    match response.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Search(page),
        } => {
            let reference: serde_json::Value = serde_json::from_str(include_str!(
                "../../../compatibility/opencti/7.260722.0/reference-results.json"
            ))
            .unwrap();
            let expected = reference["captures"]
                .as_array()
                .unwrap()
                .iter()
                .find(|capture| capture["id"] == "file-full-text")
                .unwrap();
            let expected_ids = expected["expected"]["ordered_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                page.hits
                    .iter()
                    .map(|hit| hit.id.as_str())
                    .collect::<Vec<_>>(),
                expected_ids
            );
            assert_eq!(
                page.hits[0].record_class,
                opencti_search::FullTextRecordClass::FileContent
            );
            assert!(page.hits.iter().all(|hit| {
                hit.snippet.as_deref().unwrap().contains("<mark>")
                    && hit.highlights == vec!["malware.example.org"]
                    && hit.metadata["source_object_id"] == "report--synthetic"
            }));
            assert_eq!(page.authorization_denials, 0);
            assert!(
                !serde_json::to_string(&page)
                    .unwrap()
                    .contains("inaccessible")
            );
        }
        other => panic!("expected file-content search page, got {other:?}"),
    }
    assert!(engine.graph().list_nodes().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}
