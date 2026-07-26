// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Standalone adapter for the engine-owned high-level memory contract.

use axum::{
    Extension, Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
};
use corrobore_engine::{
    MemoryContractVersion, MemoryError, MemoryErrorCode, MemoryRequest, MemoryResponse,
    MemoryServiceContext,
};
use serde::Serialize;

use crate::{app::AppState, correlation::RequestCorrelationId, error::ApiError};

#[derive(Serialize)]
pub struct MemoryTransportResponse {
    contract_version: MemoryContractVersion,
    result: MemoryResponse,
}

/// Execute the same versioned operation contract exposed by the embedded API.
///
/// Authentication middleware establishes this standalone runtime's trusted
/// context. JSON fields cannot replace workspace or policy identity because the
/// public [`MemoryRequest`] schema denies unknown context fields.
pub async fn execute_memory_operation(
    State(state): State<AppState>,
    Extension(correlation): Extension<RequestCorrelationId>,
    payload: Result<Json<MemoryRequest>, JsonRejection>,
) -> Result<Json<MemoryTransportResponse>, ApiError> {
    let Json(request) = payload.map_err(|rejection| {
        ApiError::bad_request(
            "INVALID_REQUEST",
            format!("invalid versioned memory request: {rejection}"),
        )
    })?;
    let context = MemoryServiceContext::new(
        state.config.memory_workspace_id.clone(),
        state.config.memory_actor_id.clone(),
        state.config.memory_agent_id.clone(),
        state.config.memory_session_id.clone(),
        state.config.memory_permissions,
        correlation.0.clone(),
        correlation.0,
    )
    .map_err(map_memory_error)?;
    let result = state
        .engine
        .lock()
        .map_err(|_| {
            ApiError::service_unavailable("OVERLOADED", "memory engine is temporarily unavailable")
        })?
        .execute_memory(&context, &request)
        .map_err(map_memory_error)?;
    Ok(Json(MemoryTransportResponse {
        contract_version: request.contract_version,
        result,
    }))
}

fn map_memory_error(error: MemoryError) -> ApiError {
    let (status, code) = match error.code {
        MemoryErrorCode::InvalidRequest => (StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
        MemoryErrorCode::InvalidBudget => (StatusCode::UNPROCESSABLE_ENTITY, "INVALID_BUDGET"),
        MemoryErrorCode::PermissionDenied => (StatusCode::FORBIDDEN, "PERMISSION_DENIED"),
        MemoryErrorCode::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        MemoryErrorCode::VersionConflict => (StatusCode::CONFLICT, "VERSION_CONFLICT"),
        MemoryErrorCode::IdempotencyConflict => (StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT"),
        MemoryErrorCode::IdempotencyKeyRequired => {
            (StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED")
        }
        MemoryErrorCode::BudgetExceeded => (StatusCode::UNPROCESSABLE_ENTITY, "BUDGET_EXCEEDED"),
        MemoryErrorCode::Cancelled => (StatusCode::REQUEST_TIMEOUT, "CANCELLED"),
        MemoryErrorCode::Overloaded => (StatusCode::SERVICE_UNAVAILABLE, "OVERLOADED"),
        MemoryErrorCode::SemanticProviderUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "SEMANTIC_PROVIDER_UNAVAILABLE",
        ),
        MemoryErrorCode::DurabilityFailed => (StatusCode::SERVICE_UNAVAILABLE, "DURABILITY_FAILED"),
        MemoryErrorCode::PolicyApprovalRequired => {
            (StatusCode::FORBIDDEN, "POLICY_APPROVAL_REQUIRED")
        }
        MemoryErrorCode::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    };
    ApiError {
        status,
        code,
        message: error.message,
    }
}
