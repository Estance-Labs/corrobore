// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use corrobore_engine::{
    AccessContext, AccessDecisionReason, AccessMetadata, ConsistencyLevel, ContractVersion,
    CorroboreEngine, CorroboreKnowledgeDataProvider, CountRequest, DivergenceBaseline,
    EnginePersistence, GetByIdRequest, GraphDirection, GraphReadPolicy, KnowledgeDataErrorCode,
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse,
    ListRequest, OpenCtiAccessPolicy, PaginateRequest, PreparedKnowledgeDataProjection,
    ProviderDescriptor, ProviderExecution, QueryClass, RecordPage, RequestContext,
    ShadowComparisonGate, SortDirection, TraverseRequest, compare_shadow_read,
};
use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};
use serde_json::{Value, json};

const PAGINATION_KEY: &[u8] = b"issue-45-opencti-authorization-pagination-key";
type MetadataGrant = (AccessDecisionReason, fn(&mut AccessMetadata));

#[derive(Debug)]
struct StaticPersistence {
    graph: Graph,
}

impl EnginePersistence for StaticPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self.graph.clone())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug)]
struct PushdownDenyPersistence;

impl EnginePersistence for PushdownDenyPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }

    fn prepare_knowledge_data_operation(
        &mut self,
        _operation: &KnowledgeDataOperation,
        _access: &AccessContext,
    ) -> Result<Option<PreparedKnowledgeDataProjection>, String> {
        Ok(Some(PreparedKnowledgeDataProjection {
            graph: Graph::new(),
            page_ins: 0,
            cache_hits: 0,
            authorization_denials: 1,
            full_text_page: None,
        }))
    }
}

fn access(policy_version: &str) -> AccessContext {
    AccessContext {
        subject_id: "user--alpha".to_owned(),
        organization_ids: vec!["organization--alpha".to_owned()],
        marking_ids: vec!["marking--amber".to_owned(), "marking--green".to_owned()],
        tenant_id: Some("tenant--alpha".to_owned()),
        roles: vec!["analyst".to_owned()],
        attributes: BTreeMap::from([
            ("policy_version".to_owned(), policy_version.to_owned()),
            ("authority_ids".to_owned(), "authority--alpha".to_owned()),
            ("sharing_grants".to_owned(), "sharing--alpha".to_owned()),
        ]),
    }
}

fn metadata() -> AccessMetadata {
    AccessMetadata {
        marking_ids: vec!["marking--amber".to_owned()],
        organization_ids: vec!["organization--alpha".to_owned()],
        authorized_members: Vec::new(),
        tenant_ids: vec!["tenant--alpha".to_owned()],
        creator_ids: Vec::new(),
        owner_ids: Vec::new(),
        sharing_policy: None,
        authorized_authorities: Vec::new(),
    }
}

#[test]
fn compiled_policy_covers_markings_tenants_and_every_identity_grant() {
    let policy = OpenCtiAccessPolicy::compile(&access("policy--v1")).unwrap();
    assert!(policy.evaluate(&metadata()).allowed());
    let mut equivalent = access(" policy--v1 ");
    equivalent.subject_id = " user--alpha ".to_owned();
    equivalent.organization_ids = vec![
        " organization--alpha ".to_owned(),
        "organization--alpha".to_owned(),
    ];
    equivalent.marking_ids = vec!["marking--green".to_owned(), " marking--amber ".to_owned()];
    equivalent.tenant_id = Some(" tenant--alpha ".to_owned());
    equivalent.roles = vec![" analyst ".to_owned(), "analyst".to_owned()];
    equivalent.attributes.insert(
        "authority_ids".to_owned(),
        " authority--alpha,authority--alpha ".to_owned(),
    );
    equivalent
        .attributes
        .insert("sharing_grants".to_owned(), " sharing--alpha ".to_owned());
    assert_eq!(
        policy.fingerprint(),
        OpenCtiAccessPolicy::compile(&equivalent)
            .unwrap()
            .fingerprint()
    );

    let mut denied_marking = metadata();
    denied_marking.marking_ids.push("marking--red".to_owned());
    assert_eq!(
        policy.evaluate(&denied_marking).reason(),
        AccessDecisionReason::MissingMarking
    );

    let mut denied_tenant = metadata();
    denied_tenant.tenant_ids = vec!["tenant--beta".to_owned()];
    assert_eq!(
        policy.evaluate(&denied_tenant).reason(),
        AccessDecisionReason::TenantMismatch
    );

    let mut sharing_denied = metadata();
    sharing_denied.sharing_policy = Some(json!({"deny": ["user--alpha"]}));
    assert_eq!(
        policy.evaluate(&sharing_denied).reason(),
        AccessDecisionReason::SharingDenied
    );

    let identity_grants: [MetadataGrant; 5] = [
        (
            AccessDecisionReason::AuthorizedMember,
            |metadata: &mut AccessMetadata| {
                metadata.organization_ids = vec!["organization--beta".to_owned()];
                metadata.authorized_members = vec![json!({"id": "user--alpha"})];
            },
        ),
        (
            AccessDecisionReason::Creator,
            |metadata: &mut AccessMetadata| {
                metadata.organization_ids = vec!["organization--beta".to_owned()];
                metadata.creator_ids = vec!["user--alpha".to_owned()];
            },
        ),
        (
            AccessDecisionReason::Owner,
            |metadata: &mut AccessMetadata| {
                metadata.organization_ids = vec!["organization--beta".to_owned()];
                metadata.owner_ids = vec!["user--alpha".to_owned()];
            },
        ),
        (
            AccessDecisionReason::Authority,
            |metadata: &mut AccessMetadata| {
                metadata.organization_ids = vec!["organization--beta".to_owned()];
                metadata.authorized_authorities = vec!["authority--alpha".to_owned()];
            },
        ),
        (
            AccessDecisionReason::SharingPolicy,
            |metadata: &mut AccessMetadata| {
                metadata.organization_ids = vec!["organization--beta".to_owned()];
                metadata.sharing_policy = Some(json!({"grants": ["sharing--alpha"]}));
            },
        ),
    ];
    for (reason, mutate) in identity_grants {
        let mut candidate = metadata();
        mutate(&mut candidate);
        let decision = policy.evaluate(&candidate);
        assert!(decision.allowed(), "{reason:?} should grant access");
        assert_eq!(decision.reason(), reason);
    }
}

fn node(id: &str, name: &str, access: Value) -> NodeInput {
    NodeInput::new(vec![
        "OpenCtiObject".to_owned(),
        "OpenCtiType_indicator".to_owned(),
    ])
    .with_property("opencti.canonical_id", PropertyValue::String(id.to_owned()))
    .with_property(
        "opencti.field.type",
        PropertyValue::String("indicator".to_owned()),
    )
    .with_property("opencti.field.name", PropertyValue::String(name.to_owned()))
    .with_property(
        "opencti.raw",
        PropertyValue::Json(json!({"id": id, "type": "indicator", "name": name})),
    )
    .with_property("opencti.access", PropertyValue::Json(access))
}

fn secure_graph() -> Graph {
    let mut graph = Graph::new();
    let seed = graph
        .create_node(node(
            "indicator--seed",
            "Visible seed",
            json!({
                "marking_ids": ["marking--amber"],
                "organization_ids": ["organization--alpha"],
                "tenant_ids": ["tenant--alpha"]
            }),
        ))
        .unwrap();
    let visible = graph
        .create_node(node(
            "indicator--visible",
            "Visible neighbor",
            json!({
                "marking_ids": ["marking--amber"],
                "owner_ids": ["user--alpha"],
                "tenant_ids": ["tenant--alpha"]
            }),
        ))
        .unwrap();
    let hidden = graph
        .create_node(node(
            "indicator--hidden",
            "Classified payload",
            json!({
                "marking_ids": ["marking--red"],
                "organization_ids": ["organization--beta"],
                "tenant_ids": ["tenant--beta"]
            }),
        ))
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed.clone(), "related-to", visible.clone())
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--visible".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "id": "relationship--visible",
                        "type": "relationship",
                        "relationship_type": "related-to"
                    })),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--amber"],
                        "organization_ids": ["organization--alpha"],
                        "tenant_ids": ["tenant--alpha"]
                    })),
                ),
        )
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed.clone(), "related-to", hidden)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--hidden-endpoint".to_owned()),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--amber"],
                        "organization_ids": ["organization--alpha"],
                        "tenant_ids": ["tenant--alpha"]
                    })),
                ),
        )
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed, "derived-from", visible)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--hidden-policy".to_owned()),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--red"],
                        "organization_ids": ["organization--beta"],
                        "tenant_ids": ["tenant--beta"]
                    })),
                ),
        )
        .unwrap();
    graph
}

fn request(
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{correlation_id}"),
            correlation_id: correlation_id.to_owned(),
            access,
            consistency: ConsistencyLevel::Snapshot,
            ..RequestContext::default()
        },
        operation,
    }
}

fn execute(
    engine: &mut CorroboreEngine,
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> corrobore_engine::KnowledgeDataResponseEnvelope {
    CorroboreKnowledgeDataProvider::new(engine, PAGINATION_KEY)
        .unwrap()
        .execute(request(operation, access, correlation_id))
}

#[test]
fn denied_point_reads_are_indistinguishable_from_missing_records_and_audits_are_redacted() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence {
            graph: secure_graph(),
        }))
        .build()
        .unwrap();
    let denied = execute(
        &mut engine,
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--hidden".to_owned(),
        }),
        access("policy--v1"),
        "denied",
    );
    let missing = execute(
        &mut engine,
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--absent".to_owned(),
        }),
        access("policy--v1"),
        "missing",
    );
    assert_eq!(denied.outcome, missing.outcome);
    assert_eq!(
        denied.outcome,
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(None)
        }
    );

    let audit = serde_json::to_string(engine.security_audit_events()).unwrap();
    assert!(audit.contains("policy--v1"));
    assert!(audit.contains("missing_marking"));
    assert!(!audit.contains("indicator--hidden"));
    assert!(!audit.contains("Classified payload"));
}

#[test]
fn persistent_pushdown_denials_produce_a_payload_free_denied_audit() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(PushdownDenyPersistence))
        .build()
        .unwrap();
    let response = execute(
        &mut engine,
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--hidden".to_owned(),
        }),
        access("policy--v1"),
        "persistent-denied",
    );

    assert_eq!(
        response.outcome,
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(None)
        }
    );
    let event = engine.security_audit_events().last().unwrap();
    assert!(!event.allowed);
    assert_eq!(event.authorization_denials, 1);
    assert_eq!(event.reason, AccessDecisionReason::PolicyApplied);
    assert!(
        !serde_json::to_string(event)
            .unwrap()
            .contains("indicator--hidden")
    );
}

#[test]
fn paths_require_access_to_every_node_and_relationship() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence {
            graph: secure_graph(),
        }))
        .build()
        .unwrap();
    let response = execute(
        &mut engine,
        KnowledgeDataOperation::Traverse(TraverseRequest {
            start_ids: vec!["indicator--seed".to_owned()],
            max_depth: 1,
            direction: GraphDirection::Outgoing,
            constraints: Value::Null,
            policy: GraphReadPolicy {
                max_results: 10,
                max_expansions: 10,
                supernode_threshold: 10,
                ..GraphReadPolicy::default()
            },
        }),
        access("policy--v1"),
        "paths",
    );
    let KnowledgeDataOutcome::Success {
        response: KnowledgeDataResponse::Graph(result),
    } = response.outcome
    else {
        panic!("expected graph result");
    };
    assert_eq!(
        result
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["indicator--seed", "indicator--visible"]
    );
    assert_eq!(result.relationships.len(), 1);
    assert_eq!(result.relationships[0]["id"], "relationship--visible");
    assert!(result.paths.iter().all(|path| {
        path.steps
            .iter()
            .all(|step| step.node_id != "indicator--hidden")
    }));
}

#[test]
fn list_count_order_and_page_boundaries_use_only_authorized_records() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence {
            graph: secure_graph(),
        }))
        .build()
        .unwrap();
    let query = ListRequest {
        kinds: vec!["indicator".to_owned()],
        filters: Vec::new(),
        order_by: vec![corrobore_engine::ReadOrder {
            field: "name".to_owned(),
            direction: SortDirection::Ascending,
        }],
        limit: 10,
    };
    let listed = execute(
        &mut engine,
        KnowledgeDataOperation::List(query.clone()),
        access("policy--v1"),
        "authorized-list",
    );
    let KnowledgeDataOutcome::Success {
        response: KnowledgeDataResponse::Records(listed),
    } = listed.outcome
    else {
        panic!("expected list response");
    };
    assert_eq!(
        listed
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["indicator--visible", "indicator--seed"]
    );

    let counted = execute(
        &mut engine,
        KnowledgeDataOperation::Count(CountRequest {
            filter: Value::Null,
            kinds: vec!["indicator".to_owned()],
            filters: Vec::new(),
        }),
        access("policy--v1"),
        "authorized-count",
    );
    assert_eq!(
        counted.outcome,
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Count(corrobore_engine::CountResult { count: 2 })
        }
    );

    let first = execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query: query.clone(),
            page_size: 1,
            token: None,
        }),
        access("policy--v1"),
        "authorized-page-one",
    );
    let KnowledgeDataOutcome::Success {
        response:
            KnowledgeDataResponse::Records(RecordPage {
                records: first_records,
                next_token: Some(token),
            }),
    } = first.outcome
    else {
        panic!("expected first page");
    };
    let second = execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query,
            page_size: 1,
            token: Some(token),
        }),
        access("policy--v1"),
        "authorized-page-two",
    );
    let KnowledgeDataOutcome::Success {
        response:
            KnowledgeDataResponse::Records(RecordPage {
                records: second_records,
                next_token: None,
            }),
    } = second.outcome
    else {
        panic!("expected final page");
    };
    assert_eq!(
        first_records
            .iter()
            .chain(&second_records)
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["indicator--visible", "indicator--seed"]
    );
    assert!(engine.security_audit_events().iter().all(|event| {
        event.policy_version == "policy--v1"
            && !serde_json::to_string(event)
                .unwrap()
                .contains("indicator--hidden")
    }));
}

#[test]
fn pagination_tokens_are_bound_to_policy_version_and_access_fingerprint() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence {
            graph: secure_graph(),
        }))
        .build()
        .unwrap();
    let query = ListRequest {
        kinds: vec!["indicator".to_owned()],
        filters: Vec::new(),
        order_by: vec![corrobore_engine::ReadOrder {
            field: "name".to_owned(),
            direction: SortDirection::Ascending,
        }],
        limit: 10,
    };
    let first = execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query: query.clone(),
            page_size: 1,
            token: None,
        }),
        access("policy--v1"),
        "page-one",
    );
    let KnowledgeDataOutcome::Success {
        response:
            KnowledgeDataResponse::Records(RecordPage {
                next_token: Some(token),
                ..
            }),
    } = first.outcome
    else {
        panic!("expected continuation token");
    };

    let stale = execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query,
            page_size: 1,
            token: Some(token),
        }),
        access("policy--v2"),
        "page-stale-policy",
    );
    assert_eq!(
        stale.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::StalePaginationToken)
    );
}

fn execution(correlation_id: &str, response: KnowledgeDataResponse) -> ProviderExecution {
    ProviderExecution {
        provider: ProviderDescriptor {
            name: "provider".to_owned(),
            version: "1".to_owned(),
            release: "test".to_owned(),
        },
        latency_ms: 10,
        envelope: corrobore_engine::KnowledgeDataResponseEnvelope {
            contract_version: ContractVersion::CURRENT,
            correlation_id: correlation_id.to_owned(),
            outcome: KnowledgeDataOutcome::Success { response },
        },
    }
}

#[test]
fn policy_scoped_shadow_divergences_block_even_with_a_matching_baseline() {
    let request = request(
        KnowledgeDataOperation::List(ListRequest {
            limit: 10,
            ..ListRequest::default()
        }),
        access("policy--v1"),
        "shadow-security",
    );
    let reference = execution(
        "shadow-security",
        KnowledgeDataResponse::Records(RecordPage {
            records: Vec::new(),
            next_token: None,
        }),
    );
    let shadow = execution(
        "shadow-security",
        KnowledgeDataResponse::Records(RecordPage {
            records: Vec::new(),
            next_token: Some("different-page-state".to_owned()),
        }),
    );
    let initial = compare_shadow_read(&request, reference.clone(), shadow.clone(), &[], 1);
    assert_eq!(initial.gate, ShadowComparisonGate::Blocked);
    assert_eq!(
        initial.security_differences,
        vec!["authorization_result_mismatch"]
    );

    let baselined = compare_shadow_read(
        &request,
        reference,
        shadow,
        &[DivergenceBaseline {
            id: "baseline--security-must-not-apply".to_owned(),
            query_class: QueryClass::Collection,
            fingerprint: initial.divergence_fingerprint,
            owner: "security".to_owned(),
            expires_at_unix_ms: 10,
        }],
        1,
    );
    assert_eq!(baselined.gate, ShadowComparisonGate::Blocked);
    assert!(baselined.baseline.is_none());
}
