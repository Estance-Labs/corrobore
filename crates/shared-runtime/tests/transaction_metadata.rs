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
use std::collections::HashMap;

use graph_core::{ActorId, SessionId, TransactionId, WorkspaceId};
use shared_runtime::{
    ActorKind, ActorRef, CreateTransactionMetadataRequest, RuntimeError, RuntimeTimestamp,
    RuntimeTransactionMetadata, SessionRegistry, StartSessionRequest,
};

fn actor_ref(value: &str) -> ActorRef {
    ActorRef::new(
        ActorId::new(value).expect("actor id should be valid"),
        ActorKind::Agent,
    )
}

fn register_session(registry: &mut SessionRegistry, actor: ActorRef) -> SessionId {
    let session_id = SessionId::new("session--tx-meta").expect("session id should be valid");

    registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor),
            workspace_id: WorkspaceId::new("workspace--tx-meta")
                .expect("workspace id should be valid"),
            started_at: RuntimeTimestamp::from_millis(10),
            metadata: HashMap::new(),
        })
        .expect("session start should succeed");

    session_id
}

#[test]
fn create_transaction_metadata_from_session_carries_runtime_context() {
    let mut registry = SessionRegistry::default();
    let actor = actor_ref("actor--tx-meta");
    let session_id = register_session(&mut registry, actor.clone());

    let metadata = registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--001")
                .expect("transaction id should be valid"),
            session_id: session_id.clone(),
            started_at: RuntimeTimestamp::from_millis(1_728_000_000_000),
            policy_name: Some("default-policy".to_owned()),
        })
        .expect("transaction metadata creation should succeed");

    assert_eq!(
        metadata,
        RuntimeTransactionMetadata {
            transaction_id: TransactionId::new("transaction--001")
                .expect("transaction id should be valid"),
            workspace_id: WorkspaceId::new("workspace--tx-meta")
                .expect("workspace id should be valid"),
            session_id,
            actor,
            started_at: RuntimeTimestamp::from_millis(1_728_000_000_000),
            policy_name: "default-policy".to_owned(),
        }
    );
}

#[test]
fn create_transaction_metadata_from_session_rejects_missing_policy_name() {
    let mut registry = SessionRegistry::default();
    let session_id = register_session(&mut registry, actor_ref("actor--policy-missing"));

    let error = registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--policy-missing")
                .expect("transaction id should be valid"),
            session_id,
            started_at: RuntimeTimestamp::from_millis(20),
            policy_name: None,
        })
        .expect_err("missing policy name should be rejected");

    assert!(matches!(
    error,
    RuntimeError::MissingTransactionMetadata(field) if field == "policy_name"
    ));
}

#[test]
fn validate_transaction_metadata_for_mutation_rejects_missing_metadata() {
    let registry = SessionRegistry::default();

    let error = registry
        .validate_transaction_metadata_for_mutation(None)
        .expect_err("mutation validation should reject missing transaction metadata");

    assert!(matches!(
    error,
    RuntimeError::MissingTransactionMetadata(field) if field == "transaction_metadata"
    ));
}

#[test]
fn validate_transaction_metadata_for_mutation_rejects_workspace_mismatch() {
    let mut registry = SessionRegistry::default();
    let session_id = register_session(&mut registry, actor_ref("actor--workspace-mismatch"));

    let metadata = registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--workspace-mismatch")
                .expect("transaction id should be valid"),
            session_id,
            started_at: RuntimeTimestamp::from_millis(30),
            policy_name: Some("policy".to_owned()),
        })
        .expect("transaction metadata creation should succeed");

    let mismatched = RuntimeTransactionMetadata {
        workspace_id: WorkspaceId::new("workspace--unexpected")
            .expect("workspace id should be valid"),
        ..metadata
    };

    let error = registry
        .validate_transaction_metadata_for_mutation(Some(&mismatched))
        .expect_err("workspace mismatch should be rejected");

    assert!(matches!(
        error,
        RuntimeError::TransactionWorkspaceMismatch { .. }
    ));
}

#[test]
fn validate_transaction_metadata_for_mutation_rejects_actor_mismatch() {
    let mut registry = SessionRegistry::default();
    let session_id = register_session(&mut registry, actor_ref("actor--registry"));

    let metadata = registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--actor-mismatch")
                .expect("transaction id should be valid"),
            session_id,
            started_at: RuntimeTimestamp::from_millis(40),
            policy_name: Some("policy".to_owned()),
        })
        .expect("transaction metadata creation should succeed");

    let mismatched = RuntimeTransactionMetadata {
        actor: actor_ref("actor--other"),
        ..metadata
    };

    let error = registry
        .validate_transaction_metadata_for_mutation(Some(&mismatched))
        .expect_err("actor mismatch should be rejected");

    assert!(matches!(
        error,
        RuntimeError::TransactionActorMismatch { .. }
    ));
}

#[test]
fn validate_transaction_metadata_for_mutation_accepts_matching_metadata() {
    let mut registry = SessionRegistry::default();
    let session_id = register_session(&mut registry, actor_ref("actor--ok"));

    let metadata = registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--ok")
                .expect("transaction id should be valid"),
            session_id,
            started_at: RuntimeTimestamp::from_millis(50),
            policy_name: Some("policy".to_owned()),
        })
        .expect("transaction metadata creation should succeed");

    let validation = registry.validate_transaction_metadata_for_mutation(Some(&metadata));

    assert!(validation.is_ok());
}
