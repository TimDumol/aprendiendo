//! Authenticated mobile access to the same canonical learning store as MCP.
use crate::{
    db::{RecordPracticeError, SharedStore},
    tutoring::{RecordTutoringSessionRequest, expand},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};

pub fn router(store: SharedStore) -> Router {
    Router::new()
        .route("/api/practice/v1/learning/plan", get(plan))
        .route("/api/practice/v1/learning/sessions", post(record))
        .route("/api/practice/v1/learning/handoffs", post(create_handoff))
        .route("/api/practice/v1/learning/handoffs/{id}", get(get_handoff))
        .route(
            "/api/practice/v1/learning/handoffs/{id}/repairs",
            post(save_repair),
        )
        .layer(DefaultBodyLimit::max(512 * 1024))
        .with_state(store)
}
async fn plan(State(store): State<SharedStore>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    store
        .production_action("study_plan", json!({}))
        .await
        .map(Json)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"Learning plan unavailable"})),
            )
        })
}
async fn record(
    State(store): State<SharedStore>,
    Json(request): Json<RecordTutoringSessionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let canonical =
        expand(request).map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error":e}))))?;
    let response = store.record_practice(canonical).await.map_err(|e| {
        let status = match e {
            RecordPracticeError::IdempotencyConflict => StatusCode::CONFLICT,
            RecordPracticeError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        (status, Json(json!({"error":e.to_string()})))
    })?;
    Ok(Json(json!(response)))
}

fn workflow_error(error: anyhow::Error) -> (StatusCode, Json<Value>) {
    let message = error.to_string();
    let (status, detail) = if message.starts_with("not_found:") {
        (StatusCode::NOT_FOUND, message)
    } else if message.starts_with("conflict:") {
        (StatusCode::CONFLICT, message)
    } else if message.starts_with("invalid:") {
        (StatusCode::BAD_REQUEST, message)
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Tutor workflow unavailable".into(),
        )
    };
    (status, Json(json!({"error":detail})))
}
async fn create_handoff(
    State(store): State<SharedStore>,
    Json(request): Json<crate::workflow::CreateHandoff>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    store
        .production_action("handoff_create", json!(request))
        .await
        .map(Json)
        .map_err(workflow_error)
}
async fn get_handoff(
    State(store): State<SharedStore>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    store
        .production_action("handoff_get", json!({"handoff_id":id}))
        .await
        .map(Json)
        .map_err(workflow_error)
}
async fn save_repair(
    State(store): State<SharedStore>,
    Path(id): Path<String>,
    Json(request): Json<crate::workflow::SaveRepair>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    store
        .production_action("handoff_repair", json!({"handoff_id":id,"request":request}))
        .await
        .map(Json)
        .map_err(workflow_error)
}
