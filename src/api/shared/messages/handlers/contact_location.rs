//! Location and contact-card sends.

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
    path = "/api/v1/sessions/{session_id}/messages/location",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendLocationRequest,
    responses(
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_location(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendLocationRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "location",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_location(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `location` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_location(
    state: &AppState,
    session_id: &str,
    request: SendLocationRequest,
) -> Result<MessageResponse, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let resp = cloud
            .send_location(
                &request.to,
                request.latitude,
                request.longitude,
                request.name.as_deref(),
                request.address.as_deref(),
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        location_message: MessageField::some(nx!(waproto::whatsapp::message::LocationMessage {
            degrees_latitude: Some(request.latitude),
            degrees_longitude: Some(request.longitude),
            name: request.name,
            address: request.address,
            context_info: reply_context(
                state,
                session_id,
                &client,
                &to_jid,
                request.reply_to.clone(),
            )
            .await
            .into(),
            ..Default::default()
        })),
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
    path = "/api/v1/sessions/{session_id}/messages/contact",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendContactRequest,
    responses(
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_contact(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendContactRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "contact",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_contact(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `contact` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_contact(
    state: &AppState,
    session_id: &str,
    request: SendContactRequest,
) -> Result<MessageResponse, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let contacts = serde_json::json!([{
            "name": {
                "formatted_name": request.contact.display_name,
                "first_name": request.contact.display_name,
            },
            "phones": request.contact.phones.iter().map(|p| serde_json::json!({
                "phone": p.number,
                "type": p.phone_type,
            })).collect::<Vec<_>>(),
            "org": request.contact.organization.as_ref().map(|c| serde_json::json!({ "company": c })),
        }]);
        let resp = cloud
            .send_contacts(&request.to, contacts)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let vcard = build_vcard(&request.contact);

    let message = nx!(waproto::whatsapp::Message {
        contact_message: MessageField::some(nx!(waproto::whatsapp::message::ContactMessage {
            display_name: Some(request.contact.display_name.clone()),
            vcard: Some(vcard),
            context_info: reply_context(
                state,
                session_id,
                &client,
                &to_jid,
                request.reply_to.clone(),
            )
            .await
            .into(),
            ..Default::default()
        })),
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
