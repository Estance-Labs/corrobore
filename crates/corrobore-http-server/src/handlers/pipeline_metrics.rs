//! Authenticated stage instrumentation. It never creates or changes epistemic evidence.
use crate::{app::AppState, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use graph_core::{PipelineStageReport, StageMeasurement, StageMetricError};

async fn run(
    state: AppState,
    run_id: String,
    measurement: Option<StageMeasurement>,
) -> Result<Json<PipelineStageReport>, ApiError> {
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            let report = match measurement {
                Some(sample) => engine.record_pipeline_stage(&run_id, sample),
                None => engine.pipeline_stage_report(&run_id),
            };
            report.map_err(|error| match error {
                StageMetricError::UnknownRun => {
                    ApiError::not_found("STAGE_METRIC_RUN_NOT_FOUND", error.to_string())
                }
                StageMetricError::Capacity => {
                    ApiError::service_unavailable("STAGE_METRIC_CAPACITY", error.to_string())
                }
                StageMetricError::Conflict => {
                    ApiError::bad_request("STAGE_METRIC_CONFLICT", error.to_string())
                }
                StageMetricError::Invalid(_) => {
                    ApiError::bad_request("INVALID_STAGE_METRIC", error.to_string())
                }
            })
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "stage telemetry request timeout"))?
    .map_err(|e| ApiError::internal("TASK_JOIN_FAILED", e.to_string()))??;
    Ok(Json(result))
}
pub async fn read(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<PipelineStageReport>, ApiError> {
    run(state, run_id, None).await
}
pub async fn record(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(sample): Json<StageMeasurement>,
) -> Result<Json<PipelineStageReport>, ApiError> {
    run(state, run_id, Some(sample)).await
}
