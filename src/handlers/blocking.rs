use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::blocking::{BlockRequest, BlocklistResponse};
use crate::models::common::SuccessResponse;
use crate::state::AppState;

/// Get the current blocklist
#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/blocking/list",
    tag = "blocking",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Blocklist", body = BlocklistResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_blocklist(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<BlocklistResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let blocklist = client
        .blocking()
        .get_blocklist()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let blocked: Vec<String> = blocklist.iter().map(|entry| entry.jid.to_string()).collect();
    let count = blocked.len();

    Ok(Json(BlocklistResponse { blocked, count }))
}

/// Block a contact
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/blocking/block",
    tag = "blocking",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = BlockRequest,
    responses(
        (status = 200, description = "Contact blocked", body = SuccessResponse),
        (status = 400, description = "Invalid JID"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn block_contact(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<BlockRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = request
        .jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.jid.clone()))?;

    client
        .blocking()
        .block(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Contact blocked")))
}

/// Unblock a contact
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/blocking/unblock",
    tag = "blocking",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = BlockRequest,
    responses(
        (status = 200, description = "Contact unblocked", body = SuccessResponse),
        (status = 400, description = "Invalid JID"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn unblock_contact(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<BlockRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid: Jid = request
        .jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.jid.clone()))?;

    client
        .blocking()
        .unblock(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Contact unblocked")))
}

/// Check if a contact is blocked
#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/blocking/check/{jid}",
    tag = "blocking",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("jid" = String, Path, description = "Contact JID to check")
    ),
    responses(
        (status = 200, description = "Block status", body = BlockStatusResponse),
        (status = 400, description = "Invalid JID"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn is_blocked(
    State(state): State<AppState>,
    Path((session_id, jid)): Path<(String, String)>,
) -> Result<Json<BlockStatusResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let jid_parsed: Jid = jid
        .parse()
        .map_err(|_| ApiError::InvalidJid(jid.clone()))?;

    let blocked = client
        .blocking()
        .is_blocked(&jid_parsed)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(BlockStatusResponse { jid, is_blocked: blocked }))
}

/// Block status response
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct BlockStatusResponse {
    /// JID that was checked
    pub jid: String,
    /// Whether the contact is blocked
    pub is_blocked: bool,
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
