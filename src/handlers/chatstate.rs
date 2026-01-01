use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::chatstate::{ChatStateType, SendChatStateRequest};
use crate::models::common::SuccessResponse;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chatstate/send",
    tag = "chatstate",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendChatStateRequest,
    responses(
        (status = 200, description = "Chat state sent", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_chatstate(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendChatStateRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid: Jid = request
        .to
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.to.clone()))?;

    match request.state {
        ChatStateType::Composing => {
            client
                .chatstate()
                .send_composing(&to_jid)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        ChatStateType::Recording => {
            client
                .chatstate()
                .send_recording(&to_jid)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        ChatStateType::Paused => {
            client
                .chatstate()
                .send_paused(&to_jid)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
    }

    Ok(Json(SuccessResponse::with_message("Chat state sent")))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/chatstate/typing",
    tag = "chatstate",
    params(
        ("session_id" = String, Path, description = "Session ID"),
    ),
    request_body = TypingRequest,
    responses(
        (status = 200, description = "Typing indicator sent", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_typing(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<TypingRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid: Jid = request
        .to
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.to.clone()))?;

    client
        .chatstate()
        .send_composing(&to_jid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Typing indicator sent")))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct TypingRequest {
    #[schema(example = "559999999999@s.whatsapp.net")]
    pub to: String,
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
