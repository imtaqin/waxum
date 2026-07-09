use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;
use waproto::buffa::MessageField;
use waproto::whatsapp as wa;

use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::status::StatusReactionRequest;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/status/react",
    tag = "status",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = StatusReactionRequest,
    responses(
        (status = 200, description = "Status reaction sent", body = SuccessResponse),
        (status = 400, description = "Invalid JID"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_status_reaction(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<StatusReactionRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let owner: Jid = request
        .status_owner
        .parse()
        .map_err(|_| ApiError::InvalidJid(request.status_owner.clone()))?;

    let status_broadcast = Jid::status_broadcast();
    let now_ms = chrono::Utc::now().timestamp_millis();

    let message = wa::Message {
        reaction_message: MessageField::some(wa::message::ReactionMessage {
            key: Some(wa::MessageKey {
                remote_jid: Some(status_broadcast.to_string()),
                from_me: Some(false),
                id: Some(request.message_id.clone()),
                participant: Some(owner.to_string()),
            })
            .into(),
            text: Some(request.reaction.clone()),
            grouping_key: None,
            sender_timestamp_ms: Some(now_ms),
        }),
        ..Default::default()
    };

    client
        .send_message(status_broadcast, message)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse::with_message("Status reaction sent")))
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
