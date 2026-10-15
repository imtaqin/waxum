//! Polls, buttons, lists, interactive, CTA, quick-reply and template sends and their responses.

use axum::{
    extract::{Path, State},
    Json,
};
use base64::Engine;
use waproto::buffa::{Enumeration, MessageField};

use super::common::*;
use crate::api::shared::messages::models::*;
use crate::api::shared::schedule::models::SendResponse;
use crate::error::ApiError;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/poll",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPollRequest,
    responses(
        (status = 200, description = "Poll sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_poll(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPollRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "poll",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_poll(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `poll` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_poll(
    state: &AppState,
    session_id: &str,
    request: SendPollRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let options: Vec<waproto::whatsapp::message::poll_creation_message::Option> = request
        .options
        .into_iter()
        .map(|name| {
            nx!(waproto::whatsapp::message::poll_creation_message::Option {
                option_name: Some(name),
                ..Default::default()
            })
        })
        .collect();

    let message = nx!(waproto::whatsapp::Message {
        poll_creation_message: MessageField::some(nx!(
            waproto::whatsapp::message::PollCreationMessage {
                name: Some(request.name),
                options,
                selectable_options_count: Some(request.selectable_count),
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
    path = "/api/v1/sessions/{session_id}/messages/buttons",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendButtonsRequest,
    responses(
        (status = 200, description = "Buttons message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_buttons(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendButtonsRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "buttons",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_buttons(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `buttons` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_buttons(
    state: &AppState,
    session_id: &str,
    request: SendButtonsRequest,
) -> Result<MessageResponse, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let buttons = serde_json::json!(request
            .buttons
            .iter()
            .map(|b| serde_json::json!({
                "type": "reply",
                "reply": { "id": b.button_id, "title": b.display_text },
            }))
            .collect::<Vec<_>>());
        let resp = cloud
            .send_interactive_buttons(
                &request.to,
                &request.content_text,
                buttons,
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let buttons: Vec<waproto::whatsapp::message::buttons_message::Button> = request
        .buttons
        .into_iter()
        .map(|b| {
            nx!(waproto::whatsapp::message::buttons_message::Button {
                button_id: Some(b.button_id),
                button_text: Some(nx!(
                    waproto::whatsapp::message::buttons_message::button::ButtonText {
                        display_text: Some(b.display_text),
                    }
                ),)
                .into(),
                r#type: Some(
                    waproto::whatsapp::message::buttons_message::button::Type::from_i32(1)
                        .unwrap_or_default(),
                ),
                ..Default::default()
            })
        })
        .collect();

    let header = request
        .header_text
        .map(waproto::whatsapp::message::buttons_message::Header::Text);

    let message = nx!(waproto::whatsapp::Message {
        buttons_message: MessageField::some(nx!(waproto::whatsapp::message::ButtonsMessage {
            content_text: Some(request.content_text),
            footer_text: request.footer,
            buttons,
            header_type: header.as_ref().map(|_| {
                waproto::whatsapp::message::buttons_message::HeaderType::from_i32(2)
                    .unwrap_or_default()
            }),
            header,
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
    path = "/api/v1/sessions/{session_id}/messages/list",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendListRequest,
    responses(
        (status = 200, description = "List message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendListRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "list",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_list(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `list` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_list(
    state: &AppState,
    session_id: &str,
    request: SendListRequest,
) -> Result<MessageResponse, ApiError> {
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let sections = serde_json::json!(request
            .sections
            .iter()
            .map(|s| serde_json::json!({
                "title": s.title,
                "rows": s.rows.iter().map(|r| serde_json::json!({
                    "id": r.row_id,
                    "title": r.title,
                    "description": r.description.as_deref().unwrap_or(""),
                })).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>());
        let resp = cloud
            .send_interactive_list(crate::cloud::client::SendInteractiveListRequest {
                to: &request.to,
                header_text: Some(&request.title),
                body_text: &request.description,
                footer_text: request.footer.as_deref(),
                button_text: &request.button_text,
                sections,
                reply_to: request.reply_to.as_deref(),
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let sections_json: Vec<serde_json::Value> = request
        .sections
        .iter()
        .map(|s| {
            let rows: Vec<serde_json::Value> = s
                .rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.row_id,
                        "title": r.title,
                        "description": r.description.as_deref().unwrap_or("")
                    })
                })
                .collect();
            serde_json::json!({
                "title": s.title,
                "rows": rows
            })
        })
        .collect();

    let list_params = serde_json::json!({
        "title": request.title,
        "button": request.button_text,
        "sections": sections_json
    });

    let native_flow = nx!(waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons: vec![
            nx!(waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("single_select".to_string()),
                button_params_json: Some(list_params.to_string()),
            }),
        ],
        ..Default::default()
    });

    let message = nx!(waproto::whatsapp::Message {
        interactive_message: MessageField::some(
            nx!(waproto::whatsapp::message::InteractiveMessage {
                body: Some(nx!(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.description),
                })).into(),
                footer: request.footer.map(|f| {
                    nx!(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            }),
        ),
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
    path = "/api/v1/sessions/{session_id}/messages/interactive",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendInteractiveRequest,
    responses(
        (status = 200, description = "Interactive message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_interactive(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInteractiveRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "interactive",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_interactive(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `interactive` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_interactive(
    state: &AppState,
    session_id: &str,
    request: SendInteractiveRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let buttons: Vec<
        waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton,
    > = request
        .buttons
        .into_iter()
        .map(|b| {
            nx!(waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some(b.name),
                button_params_json: Some(b.button_params_json),
            })
        })
        .collect();

    let native_flow = nx!(
        waproto::whatsapp::message::interactive_message::NativeFlowMessage {
            buttons,
            ..Default::default()
        }
    );

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
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

    let interactive = nx!(waproto::whatsapp::message::InteractiveMessage {
        body: Some(nx!(waproto::whatsapp::message::interactive_message::Body {
            text: Some(request.body_text),
        }))
        .into(),
        footer: request
            .footer_text
            .map(
                |f| nx!(waproto::whatsapp::message::interactive_message::Footer {
                    text: Some(f),
                    ..Default::default()
                }),
            )
            .into(),
        interactive_message: Some(
            waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(
                Box::new(native_flow),
            ),
        ),
        context_info,
        ..Default::default()
    });

    let inner = nx!(waproto::whatsapp::Message {
        interactive_message: MessageField::some(interactive),
        ..Default::default()
    });

    let message = if request.view_once {
        nx!(waproto::whatsapp::Message {
            view_once_message_v2: MessageField::some(nx!(
                waproto::whatsapp::message::FutureProofMessage {
                    message: MessageField::some(inner),
                }
            ),),
            ..Default::default()
        })
    } else {
        inner
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
    path = "/api/v1/sessions/{session_id}/messages/cta-url",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendCtaUrlRequest,
    responses(
        (status = 200, description = "CTA URL message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_cta_url(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendCtaUrlRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "cta-url",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_cta_url(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `cta-url` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_cta_url(
    state: &AppState,
    session_id: &str,
    request: SendCtaUrlRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let merchant_url = request
        .merchant_url
        .clone()
        .unwrap_or_else(|| request.url.clone());
    let params = serde_json::json!({
        "display_text": request.display_text,
        "url": request.url,
        "merchant_url": merchant_url,
    });

    let native_flow = nx!(waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons: vec![
            nx!(waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("cta_url".to_string()),
                button_params_json: Some(params.to_string()),
            }),
        ],
        message_params_json: Some("{\"tag\":\"cta_url\"}".to_string()),
        ..Default::default()
    });

    let header_media = if let Some(ref media_ref) = request.image {
        let (data, mimetype) = get_media_data(media_ref).await?;
        let upload = client
            .upload(
                data.clone(),
                wacore::download::MediaType::Image,
                Default::default(),
            )
            .await
            .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;
        Some(nx!(waproto::whatsapp::message::ImageMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            ..Default::default()
        }))
    } else {
        None
    };

    let header_title = request
        .header_text
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| request.display_text.clone());

    let header = nx!(waproto::whatsapp::message::interactive_message::Header {
        title: Some(header_title),
        has_media_attachment: Some(header_media.is_some()),
        media: header_media.map(|img| {
            waproto::whatsapp::__buffa::oneof::message::interactive_message::header::Media::ImageMessage(
                Box::new(img),
            )
        }),
        ..Default::default()
    });

    let message = nx!(waproto::whatsapp::Message {
        interactive_message: MessageField::some(
            nx!(waproto::whatsapp::message::InteractiveMessage {
                header: MessageField::some(header),
                body: Some(nx!(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                })).into(),
                footer: request.footer_text.map(|f| {
                    nx!(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            }),
        ),
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
    path = "/api/v1/sessions/{session_id}/messages/quick-reply",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendQuickReplyRequest,
    responses(
        (status = 200, description = "Quick reply message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_quick_reply(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendQuickReplyRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "quick-reply",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_quick_reply(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `quick-reply` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_quick_reply(
    state: &AppState,
    session_id: &str,
    request: SendQuickReplyRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    if request.buttons.is_empty() {
        return Err(ApiError::Internal(
            "buttons must contain 1 to 3 items".into(),
        ));
    }

    let buttons: Vec<
        waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton,
    > = request
        .buttons
        .into_iter()
        .map(|b| {
            let params = serde_json::json!({
                "display_text": b.display_text,
                "id": b.id,
            });
            nx!(waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("quick_reply".to_string()),
                button_params_json: Some(params.to_string()),
            })
        })
        .collect();

    let native_flow = nx!(
        waproto::whatsapp::message::interactive_message::NativeFlowMessage {
            buttons,
            ..Default::default()
        }
    );

    let message = nx!(waproto::whatsapp::Message {
        interactive_message: MessageField::some(
            nx!(waproto::whatsapp::message::InteractiveMessage {
                body: Some(nx!(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                })).into(),
                footer: request.footer_text.map(|f| {
                    nx!(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            }),
        ),
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
    path = "/api/v1/sessions/{session_id}/messages/poll-update",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPollUpdateRequest,
    responses(
        (status = 200, description = "Poll vote sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_poll_update(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPollUpdateRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "poll-update",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_poll_update(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `poll-update` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_poll_update(
    state: &AppState,
    session_id: &str,
    request: SendPollUpdateRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let enc_payload = request.enc_payload.map(|p| {
        base64::engine::general_purpose::STANDARD
            .decode(&p)
            .unwrap_or_default()
    });
    let enc_iv = request.enc_iv.map(|iv| {
        base64::engine::general_purpose::STANDARD
            .decode(&iv)
            .unwrap_or_default()
    });

    let message = nx!(waproto::whatsapp::Message {
        poll_update_message: MessageField::some(nx!(
            waproto::whatsapp::message::PollUpdateMessage {
                poll_creation_message_key: Some(nx!(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.poll_message_id),
                    from_me: Some(false),
                    ..Default::default()
                }))
                .into(),
                vote: Some(nx!(waproto::whatsapp::message::PollEncValue {
                    enc_payload,
                    enc_iv,
                }))
                .into(),
                sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            }
        )),
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
    path = "/api/v1/sessions/{session_id}/messages/buttons-response",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendButtonsResponseRequest,
    responses(
        (status = 200, description = "Buttons response sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_buttons_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendButtonsResponseRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "buttons-response",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_buttons_response(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `buttons-response` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_buttons_response(
    state: &AppState,
    session_id: &str,
    request: SendButtonsResponseRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = nx!(waproto::whatsapp::Message {
        buttons_response_message: MessageField::some(
            nx!(waproto::whatsapp::message::ButtonsResponseMessage {
            selected_button_id: Some(request.selected_button_id),
            r#type: Some(waproto::whatsapp::message::buttons_response_message::Type::DISPLAY_TEXT),
            context_info: request.reply_to.map(|id| {
                nx!(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            }).into(),
                response: Some(
                    waproto::whatsapp::message::buttons_response_message::Response::SelectedDisplayText(
                        request.selected_display_text,
                    ),
                ),
            }),
        ),
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
    path = "/api/v1/sessions/{session_id}/messages/list-response",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendListResponseRequest,
    responses(
        (status = 200, description = "List response sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_list_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendListResponseRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "list-response",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_list_response(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `list-response` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_list_response(
    state: &AppState,
    session_id: &str,
    request: SendListResponseRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = nx!(waproto::whatsapp::Message {
        list_response_message: MessageField::some(nx!(
            waproto::whatsapp::message::ListResponseMessage {
                title: Some(request.title),
                list_type: Some(
                    waproto::whatsapp::message::list_response_message::ListType::SINGLE_SELECT,
                ),
                single_select_reply: Some(nx!(
                    waproto::whatsapp::message::list_response_message::SingleSelectReply {
                        selected_row_id: Some(request.selected_row_id),
                    }
                ),)
                .into(),
                description: request.description,
                context_info: request
                    .reply_to
                    .map(|id| nx!(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    }))
                    .into(),
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
    path = "/api/v1/sessions/{session_id}/messages/interactive-response",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendInteractiveResponseRequest,
    responses(
        (status = 200, description = "Interactive response sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_interactive_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInteractiveResponseRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "interactive-response",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_interactive_response(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `interactive-response` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_interactive_response(
    state: &AppState,
    session_id: &str,
    request: SendInteractiveResponseRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let native_flow = nx!(
        waproto::whatsapp::message::interactive_response_message::NativeFlowResponseMessage {
            name: Some(request.name),
            params_json: Some(request.params_json),
            version: Some(request.version),
        }
    );

    let message = nx!(waproto::whatsapp::Message {
        interactive_response_message: MessageField::some(
            nx!(waproto::whatsapp::message::InteractiveResponseMessage {
                body: request.body_text.map(|text| {
                    nx!(waproto::whatsapp::message::interactive_response_message::Body {
                        text: Some(text),
                        ..Default::default()
                    })
                }).into(),
                context_info: request.reply_to.map(|id| {
                    nx!(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                }).into(),
                interactive_response_message: Some(
                    waproto::whatsapp::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(Box::new(native_flow)),
                ),
            }),
        ),
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
    path = "/api/v1/sessions/{session_id}/messages/highly-structured",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendHighlyStructuredRequest,
    responses(
        (status = 200, description = "HSM sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_highly_structured(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendHighlyStructuredRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "highly-structured",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_highly_structured(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `highly-structured` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_highly_structured(
    state: &AppState,
    session_id: &str,
    request: SendHighlyStructuredRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let message = nx!(waproto::whatsapp::Message {
        highly_structured_message: MessageField::some(nx!(
            waproto::whatsapp::message::HighlyStructuredMessage {
                namespace: Some(request.namespace),
                element_name: Some(request.element_name),
                params: request.params,
                fallback_lg: request.fallback_lg,
                fallback_lc: request.fallback_lc,
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
    path = "/api/v1/sessions/{session_id}/messages/template-button-reply",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendTemplateButtonReplyRequest,
    responses(
        (status = 200, description = "Template button reply sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_template_button_reply(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendTemplateButtonReplyRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
        &state,
        &session_id,
        "template-button-reply",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_template_button_reply(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `template-button-reply` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_template_button_reply(
    state: &AppState,
    session_id: &str,
    request: SendTemplateButtonReplyRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = nx!(waproto::whatsapp::Message {
        template_button_reply_message: MessageField::some(nx!(
            waproto::whatsapp::message::TemplateButtonReplyMessage {
                selected_id: Some(request.selected_id),
                selected_display_text: Some(request.selected_display_text),
                selected_index: request.selected_index,
                context_info: request
                    .reply_to
                    .map(|id| nx!(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    }))
                    .into(),
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
