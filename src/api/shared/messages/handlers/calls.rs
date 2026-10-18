//! Scheduled-call sends and edits.

use axum::{
    extract::{Path, State},
    Json,
};
use waproto::buffa::MessageField;

use super::common::*;
use crate::api::shared::messages::models::*;
use crate::api::shared::schedule::models::SendResponse;
use crate::error::ApiError;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/scheduled-call",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendScheduledCallRequest,
    responses(
        (status = 200, description = "Scheduled call created", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_scheduled_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendScheduledCallRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "scheduled-call",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_scheduled_call(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `scheduled-call` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_scheduled_call(
    state: &AppState,
    session_id: &str,
    request: SendScheduledCallRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let call_type = match request.call_type.to_lowercase().as_str() {
        "video" => waproto::whatsapp::message::scheduled_call_creation_message::CallType::VIDEO,
        _ => waproto::whatsapp::message::scheduled_call_creation_message::CallType::VOICE,
    };

    let message = nx!(waproto::whatsapp::Message {
        scheduled_call_creation_message: MessageField::some(nx!(
            waproto::whatsapp::message::ScheduledCallCreationMessage {
                scheduled_timestamp_ms: Some(request.scheduled_timestamp_ms),
                call_type: Some(call_type),
                title: request.title,
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/scheduled-call-edit",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendScheduledCallEditRequest,
    responses(
        (status = 200, description = "Scheduled call edited", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_scheduled_call_edit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendScheduledCallEditRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "scheduled-call-edit",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_scheduled_call_edit(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `scheduled-call-edit` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_scheduled_call_edit(
    state: &AppState,
    session_id: &str,
    request: SendScheduledCallEditRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let edit_type = match request.edit_type.to_lowercase().as_str() {
        "cancel" => waproto::whatsapp::message::scheduled_call_edit_message::EditType::CANCEL,
        _ => waproto::whatsapp::message::scheduled_call_edit_message::EditType::UNKNOWN,
    };

    let message = nx!(waproto::whatsapp::Message {
        scheduled_call_edit_message: MessageField::some(nx!(
            waproto::whatsapp::message::ScheduledCallEditMessage {
                key: Some(nx!(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.scheduled_call_message_id),
                    from_me: Some(true),
                    ..Default::default()
                }))
                .into(),
                edit_type: Some(edit_type),
            }
        ),),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &to_jid).await;
    }

    let message_id = client
        .send_message(to_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &to_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}
