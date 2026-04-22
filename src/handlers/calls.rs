use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::calls::RejectCallRequest;
use crate::models::common::SuccessResponse;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/calls/reject",
    tag = "calls",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RejectCallRequest,
    responses(
        (status = 200, description = "Call rejected", body = SuccessResponse),
        (status = 400, description = "Invalid JID or empty call_id"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn reject_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RejectCallRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let from: Jid = request
        .from
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.from.clone()))?;

    client
        .reject_call(&request.call_id, &from)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Call rejected")))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}
