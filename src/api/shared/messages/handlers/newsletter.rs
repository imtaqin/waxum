//! Newsletter admin and follower invites and newsletter forwards.

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
    path = "/api/v1/sessions/{session_id}/messages/newsletter-admin-invite",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendNewsletterAdminInviteRequest,
    responses(
        (status = 200, description = "Newsletter admin invite sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_admin_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterAdminInviteRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "newsletter-admin-invite",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_newsletter_admin_invite(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `newsletter-admin-invite` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_newsletter_admin_invite(
    state: &AppState,
    session_id: &str,
    request: SendNewsletterAdminInviteRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = waproto::whatsapp::Message {
        newsletter_admin_invite_message: MessageField::some(
            waproto::whatsapp::message::NewsletterAdminInviteMessage {
                newsletter_jid: Some(request.newsletter_jid),
                newsletter_name: Some(request.newsletter_name),
                caption: request.caption,
                invite_expiration: request.invite_expiration,
                ..Default::default()
            },
        ),
        ..Default::default()
    };

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
        &message_id,
    )
    .await;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/newsletter-follower-invite",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendNewsletterFollowerInviteRequest,
    responses(
        (status = 200, description = "Newsletter follower invite sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_follower_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterFollowerInviteRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "newsletter-follower-invite",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_newsletter_follower_invite(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `newsletter-follower-invite` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_newsletter_follower_invite(
    state: &AppState,
    session_id: &str,
    request: SendNewsletterFollowerInviteRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = waproto::whatsapp::Message {
        newsletter_follower_invite_message_v2: MessageField::some(
            waproto::whatsapp::message::NewsletterFollowerInviteMessage {
                newsletter_jid: Some(request.newsletter_jid),
                newsletter_name: Some(request.newsletter_name),
                caption: request.caption,
                ..Default::default()
            },
        ),
        ..Default::default()
    };

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
        &message_id,
    )
    .await;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/newsletter-forward",
    tag = "newsletter",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendNewsletterForwardRequest,
    responses(
        (status = 200, description = "Newsletter message forwarded", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_forward(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterForwardRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "newsletter-forward",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_newsletter_forward(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `newsletter-forward` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_newsletter_forward(
    state: &AppState,
    session_id: &str,
    request: SendNewsletterForwardRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let content_type = match request.content_type.as_deref() {
        Some("update_card") => Some(
            waproto::whatsapp::context_info::forwarded_newsletter_message_info::ContentType::UPDATE_CARD,
        ),
        Some("link_card") => Some(
            waproto::whatsapp::context_info::forwarded_newsletter_message_info::ContentType::LINK_CARD,
        ),
        _ => Some(
            waproto::whatsapp::context_info::forwarded_newsletter_message_info::ContentType::UPDATE,
        ),
    };

    let message = waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                context_info: MessageField::some(waproto::whatsapp::ContextInfo {
                    is_forwarded: Some(true),
                    forwarding_score: Some(1),
                    forwarded_newsletter_message_info: Some(
                        waproto::whatsapp::context_info::ForwardedNewsletterMessageInfo {
                            newsletter_jid: Some(request.newsletter_jid),
                            server_message_id: Some(request.server_message_id),
                            newsletter_name: request.newsletter_name,
                            content_type,
                            ..Default::default()
                        },
                    )
                    .into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

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
        &message_id,
    )
    .await;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}
