use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::state::AppState;

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;
    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/bots",
    tag = "bots",
    params(("session_id" = String, Path, description = "Session ID")),
    responses((status = 200, description = "Bot directory", body = crate::api::web::bots::models::BotListResponse))
)]
pub async fn list_bots(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let list = client
        .bots()
        .list()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let val = serde_json::json!(format!("{:?}", list));
    Ok(Json(serde_json::json!({ "bots": val })))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/capping",
    tag = "bots",
    params(("session_id" = String, Path, description = "Session ID")),
    responses((status = 200, description = "New chat capping status", body = crate::api::web::bots::models::CappingResponse))
)]
pub async fn get_capping(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _client = get_client(&state, &session_id)?;
    Ok(Json(
        serde_json::json!({ "capping": null, "note": "capping query requires mex; use /mex/query with NewChatCapping doc" }),
    ))
}
