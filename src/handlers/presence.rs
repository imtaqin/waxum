use axum::{
    extract::{Path, State},
    Json,
};

use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::presence::{PresenceStatus, SetPresenceRequest, SubscribePresenceRequest};
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/presence/set",
    tag = "presence",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SetPresenceRequest,
    responses(
        (status = 200, description = "Presence set", body = SuccessResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn set_presence(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SetPresenceRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    match request.status {
        PresenceStatus::Available => {
            client
                .presence()
                .set_available()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        PresenceStatus::Unavailable => {
            client
                .presence()
                .set_unavailable()
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
    }

    Ok(Json(SuccessResponse::with_message(format!(
        "Presence set to {:?}",
        request.status
    ))))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/presence/subscribe",
    tag = "presence",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SubscribePresenceRequest,
    responses(
        (status = 200, description = "Subscribed to presence", body = SuccessResponse),
        (status = 400, description = "Invalid JID"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn subscribe_presence(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SubscribePresenceRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let jid: Jid = if request.jid.contains('@') {
        request
            .jid
            .parse()
            .map_err(|_| ApiError::InvalidJid(request.jid.clone()))?
    } else {
        Jid::pn(&request.jid)
    };

    client
        .presence()
        .subscribe(&jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message(format!(
        "Subscribed to presence updates for {}",
        jid
    ))))
}

fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}
