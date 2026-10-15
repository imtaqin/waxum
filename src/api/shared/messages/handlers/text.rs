//! Text, edit, reaction, pin, forward, comment, revoke and mark-as-read sends.

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
    path = "/api/v1/sessions/{session_id}/messages/text",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendTextRequest,
    responses(
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_text(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendTextRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "text",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_text(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `text` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_text(
    state: &AppState,
    session_id: &str,
    request: SendTextRequest,
) -> Result<MessageResponse, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let resp = cloud
            .send_text(
                &request.to,
                &request.text,
                request.link_preview.unwrap_or(false),
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let message_id = resp
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|a| a.first())
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        return Ok(MessageResponse {
            message_id,
            timestamp: chrono::Utc::now().timestamp(),
            to: request.to,
        });
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let mut mentioned: Vec<String> = Vec::new();
    if let Some(mentions) = request.mentions {
        for entry in mentions {
            let s = parse_jid(&entry)?.to_string();
            if !mentioned.contains(&s) {
                mentioned.push(s);
            }
        }
    }
    let explicit_mentions = mentioned.len();
    if request.mention_all.unwrap_or(false) {
        if to_jid.server != wacore_binary::jid::GROUP_SERVER {
            return Err(ApiError::BadRequest(
                "mention_all is only supported for group chats".to_string(),
            ));
        }
        let metadata = client
            .groups()
            .fetch_metadata(&to_jid)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        for p in metadata.participants {
            let s = p.jid.to_string();
            if !mentioned.contains(&s) {
                mentioned.push(s);
            }
        }
    }

    let mut context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::api::web::fake_reply::handlers::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| {
                    nx!(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                })
                .into()
        };

    let mut text = request.text;
    if !mentioned.is_empty() {
        for jid_str in &mentioned[..explicit_mentions] {
            let at_form = format!("@{}", jid_str.split('@').next().unwrap_or(jid_str));
            if !text.contains(&at_form) {
                if !text.ends_with(' ') {
                    text.push(' ');
                }
                text.push_str(&at_form);
            }
        }
        context_info.get_or_insert_default().mentioned_jid = mentioned;
    }

    let preview = if request.link_preview == Some(true) {
        crate::link_preview::for_text(&text).await
    } else {
        None
    };
    let mut extended = nx!(waproto::whatsapp::message::ExtendedTextMessage {
        text: Some(text),
        context_info,
        ..Default::default()
    });
    if let Some(preview) = preview {
        crate::link_preview::apply(&mut extended, preview);
    }
    let message = nx!(waproto::whatsapp::Message {
        extended_text_message: MessageField::some(extended),
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
    path = "/api/v1/sessions/{session_id}/messages/edit",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = EditMessageRequest,
    responses(
        (status = 200, description = "Message edited", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn edit_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<EditMessageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let new_content = nx!(waproto::whatsapp::Message {
        extended_text_message: MessageField::some(nx!(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                ..Default::default()
            }
        ),),
        ..Default::default()
    });

    let result = client
        .edit_message_raw(
            to_jid.clone(),
            request.message_id,
            new_content,
            whatsapp_rust::EditOptions::default(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id: result.message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/react",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendReactionRequest,
    responses(
        (status = 200, description = "Reaction sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_reaction(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendReactionRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let resp = cloud
            .send_reaction(&request.to, &request.message_id, &request.emoji)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(Json(cloud_message_response(resp, request.to)));
    }

    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let key = nx!(waproto::whatsapp::MessageKey {
        remote_jid: Some(request.to.clone()),
        id: Some(request.message_id),
        from_me: Some(false),
        ..Default::default()
    });

    let message_id = client
        .send_reaction_raw(to_jid.clone(), key, &request.emoji)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/pin",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPinMessageRequest,
    responses(
        (status = 200, description = "Message pinned/unpinned", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_pin_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPinMessageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let chat_jid = parse_jid(&request.chat)?;

    let pin_type = if request.duration_seconds > 0 {
        waproto::whatsapp::message::pin_in_chat_message::Type::PIN_FOR_ALL
    } else {
        waproto::whatsapp::message::pin_in_chat_message::Type::UNPIN_FOR_ALL
    };

    let message = nx!(waproto::whatsapp::Message {
        pin_in_chat_message: MessageField::some(nx!(
            waproto::whatsapp::message::PinInChatMessage {
                key: Some(nx!(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.chat.clone()),
                    id: Some(request.message_id),
                    from_me: Some(false),
                    ..Default::default()
                }))
                .into(),
                r#type: Some(pin_type),
                sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
            }
        )),
        ..Default::default()
    });

    if auto_presence_on_send() {
        show_typing_before_send(&client, &chat_jid).await;
    }

    let message_id = client
        .send_message(chat_jid.clone(), message.clone())
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    crate::api::shared::search::handlers::record_outgoing(
        &state,
        &session_id,
        &chat_jid,
        &message,
        message_id.as_str(),
    )
    .await;

    Ok(Json(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: chat_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/forward",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = ForwardMessageRequest,
    responses(
        (status = 200, description = "Message forwarded", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn forward_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ForwardMessageRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "forward",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_forward_message(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `forward` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_forward_message(
    state: &AppState,
    session_id: &str,
    request: ForwardMessageRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        extended_text_message: MessageField::some(nx!(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                context_info: MessageField::some(nx!(waproto::whatsapp::ContextInfo {
                    is_forwarded: Some(true),
                    forwarding_score: Some(1),
                    ..Default::default()
                })),
                ..Default::default()
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
    path = "/api/v1/sessions/{session_id}/messages/comment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendCommentRequest,
    responses(
        (status = 200, description = "Comment sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_comment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendCommentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "comment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_comment(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `comment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_comment(
    state: &AppState,
    session_id: &str,
    request: SendCommentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let target_jid = request
        .target_chat_jid
        .unwrap_or_else(|| request.to.clone());
    let parent_key = nx!(waproto::whatsapp::MessageKey {
        remote_jid: Some(target_jid),
        id: Some(request.target_message_id),
        from_me: Some(false),
        participant: request.target_participant,
    });

    let message_id = client
        .comments()
        .send_text_raw(to_jid.clone(), parent_key, &request.text)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id: message_id.to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[allow(dead_code)]
pub async fn send_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    send_text(
        State(state),
        Path(session_id),
        Json(SendTextRequest {
            to: request.to,
            text: request.text,
            reply_to: None,
            fake_reply: None,
            mentions: None,
            mention_all: None,
            send_at: None,
            link_preview: None,
        }),
    )
    .await
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/revoke",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RevokeMessageRequest,
    responses(
        (status = 200, description = "Message revoked", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn revoke_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RevokeMessageRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let revoke_type = match request.original_sender {
        Some(sender) => {
            let sender_jid = parse_jid(&sender)?;
            whatsapp_rust::RevokeType::Admin {
                original_sender: sender_jid,
            }
        }
        None => whatsapp_rust::RevokeType::Sender,
    };

    client
        .revoke_message_raw(to_jid, request.message_id.as_str(), revoke_type)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/read",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = MarkAsReadRequest,
    responses(
        (status = 200, description = "Messages marked as read", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn mark_as_read(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<MarkAsReadRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        for id in &request.message_ids {
            cloud
                .mark_read(id)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        }
        return Ok(Json(SuccessResponse { success: true }));
    }

    let client = get_client(&state, &session_id)?;
    let chat_jid = parse_jid(&request.chat_jid)?;

    let sender = match &request.sender {
        Some(s) => Some(parse_jid(s)?),
        None => None,
    };

    let id_refs: Vec<&str> = request.message_ids.iter().map(|s| s.as_str()).collect();
    client
        .mark_as_read(&chat_jid, sender.as_ref(), &id_refs)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
}
