use axum::{
    extract::{Path, State},
    Json,
};
use base64::Engine;
use wacore_binary::jid::Jid;

use crate::error::ApiError;
use crate::models::messages::*;
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
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_text(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendTextRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    // Build context_info: fake_reply has priority over reply_to.
    let context_info: Option<Box<waproto::whatsapp::ContextInfo>> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).map(Box::new)
        } else {
            request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            })
        };

    let message = waproto::whatsapp::Message {
        extended_text_message: Some(Box::new(waproto::whatsapp::message::ExtendedTextMessage {
            text: Some(request.text),
            context_info,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/image",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendImageRequest,
    responses(
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_image(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendImageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let (data, mimetype) = get_media_data(&request.image).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Image,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let context_info: Option<Box<waproto::whatsapp::ContextInfo>> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).map(Box::new)
        } else {
            request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            })
        };

    let message = waproto::whatsapp::Message {
        image_message: Some(Box::new(waproto::whatsapp::message::ImageMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            caption: request.caption,
            context_info,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/video",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendVideoRequest,
    responses(
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_video(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendVideoRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let (data, mimetype) = get_media_data(&request.video).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Video,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let context_info: Option<Box<waproto::whatsapp::ContextInfo>> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).map(Box::new)
        } else {
            request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            })
        };

    let message = waproto::whatsapp::Message {
        video_message: Some(Box::new(waproto::whatsapp::message::VideoMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            caption: request.caption,
            context_info,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/audio",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendAudioRequest,
    responses(
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_audio(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendAudioRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let (data, mimetype) = get_media_data(&request.audio).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Audio,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        audio_message: Some(Box::new(waproto::whatsapp::message::AudioMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            ptt: Some(request.ptt),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/document",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendDocumentRequest,
    responses(
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_document(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendDocumentRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let (data, mimetype) = get_media_data(&request.document).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Document,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let context_info: Option<Box<waproto::whatsapp::ContextInfo>> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).map(Box::new)
        } else {
            request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            })
        };

    let message = waproto::whatsapp::Message {
        document_message: Some(Box::new(waproto::whatsapp::message::DocumentMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            file_name: Some(request.filename),
            caption: request.caption,
            context_info,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/sticker",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendStickerRequest,
    responses(
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_sticker(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendStickerRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let (data, _mimetype) = get_media_data(&request.sticker).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Sticker,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        sticker_message: Some(Box::new(waproto::whatsapp::message::StickerMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some("image/webp".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

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
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_location(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendLocationRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        location_message: Some(Box::new(waproto::whatsapp::message::LocationMessage {
            degrees_latitude: Some(request.latitude),
            degrees_longitude: Some(request.longitude),
            name: request.name,
            address: request.address,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
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
        (status = 200, description = "Message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_contact(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendContactRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let vcard = build_vcard(&request.contact);

    let message = waproto::whatsapp::Message {
        contact_message: Some(Box::new(waproto::whatsapp::message::ContactMessage {
            display_name: Some(request.contact.display_name.clone()),
            vcard: Some(vcard),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
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

    let new_content = waproto::whatsapp::Message {
        extended_text_message: Some(Box::new(waproto::whatsapp::message::ExtendedTextMessage {
            text: Some(request.text),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .edit_message(to_jid.clone(), request.message_id, new_content)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
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
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    // Build the target MessageKey. The upstream send_reaction transparently
    // picks the right wire shape — encrypted CAG addon for community-announce
    // / channel chats, regular ReactionMessage otherwise.
    let key = waproto::whatsapp::MessageKey {
        remote_jid: Some(request.to.clone()),
        id: Some(request.message_id),
        from_me: Some(false),
        ..Default::default()
    };

    let message_id = client
        .send_reaction(to_jid.clone(), key, &request.emoji)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Poll ---

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
        (status = 200, description = "Poll sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_poll(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPollRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let options: Vec<waproto::whatsapp::message::poll_creation_message::Option> = request
        .options
        .into_iter()
        .map(
            |name| waproto::whatsapp::message::poll_creation_message::Option {
                option_name: Some(name),
                ..Default::default()
            },
        )
        .collect();

    let message = waproto::whatsapp::Message {
        poll_creation_message: Some(Box::new(waproto::whatsapp::message::PollCreationMessage {
            name: Some(request.name),
            options,
            selectable_options_count: Some(request.selectable_count),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Buttons ---

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
        (status = 200, description = "Buttons message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_buttons(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendButtonsRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let buttons: Vec<waproto::whatsapp::message::buttons_message::Button> = request
        .buttons
        .into_iter()
        .map(|b| waproto::whatsapp::message::buttons_message::Button {
            button_id: Some(b.button_id),
            button_text: Some(
                waproto::whatsapp::message::buttons_message::button::ButtonText {
                    display_text: Some(b.display_text),
                },
            ),
            r#type: Some(1), // RESPONSE type
            ..Default::default()
        })
        .collect();

    let header = request
        .header_text
        .map(waproto::whatsapp::message::buttons_message::Header::Text);

    let message = waproto::whatsapp::Message {
        buttons_message: Some(Box::new(waproto::whatsapp::message::ButtonsMessage {
            content_text: Some(request.content_text),
            footer_text: request.footer,
            buttons,
            header_type: header.as_ref().map(|_| 2), // TEXT header type
            header,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- List (via Interactive NativeFlow) ---

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
        (status = 200, description = "List message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendListRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    // Build list params JSON for native flow
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

    let native_flow = waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons: vec![
            waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("single_select".to_string()),
                button_params_json: Some(list_params.to_string()),
            },
        ],
        ..Default::default()
    };

    let message = waproto::whatsapp::Message {
        interactive_message: Some(Box::new(
            waproto::whatsapp::message::InteractiveMessage {
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.description),
                }),
                footer: request.footer.map(|f| {
                    Box::new(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(native_flow),
                ),
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Interactive (Generic Native Flow) ---

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
        (status = 200, description = "Interactive message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_interactive(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInteractiveRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let buttons: Vec<
        waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton,
    > = request
        .buttons
        .into_iter()
        .map(|b| {
            waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some(b.name),
                button_params_json: Some(b.button_params_json),
            }
        })
        .collect();

    let native_flow = waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons,
        ..Default::default()
    };

    let context_info: Option<Box<waproto::whatsapp::ContextInfo>> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).map(Box::new)
        } else {
            request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            })
        };

    let interactive = waproto::whatsapp::message::InteractiveMessage {
        body: Some(waproto::whatsapp::message::interactive_message::Body {
            text: Some(request.body_text),
        }),
        footer: request.footer_text.map(|f| {
            Box::new(waproto::whatsapp::message::interactive_message::Footer {
                text: Some(f),
                ..Default::default()
            })
        }),
        interactive_message: Some(
            waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(
                native_flow,
            ),
        ),
        context_info,
        ..Default::default()
    };

    let inner = waproto::whatsapp::Message {
        interactive_message: Some(Box::new(interactive)),
        ..Default::default()
    };

    let message = if request.view_once {
        waproto::whatsapp::Message {
            view_once_message_v2: Some(Box::new(waproto::whatsapp::message::FutureProofMessage {
                message: Some(Box::new(inner)),
            })),
            ..Default::default()
        }
    } else {
        inner
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- CTA URL button (single call-to-action that opens a URL) ---

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
        (status = 200, description = "CTA URL message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_cta_url(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendCtaUrlRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let merchant_url = request
        .merchant_url
        .clone()
        .unwrap_or_else(|| request.url.clone());
    let params = serde_json::json!({
        "display_text": request.display_text,
        "url": request.url,
        "merchant_url": merchant_url,
    });

    let native_flow = waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons: vec![
            waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("cta_url".to_string()),
                button_params_json: Some(params.to_string()),
            },
        ],
        ..Default::default()
    };

    let message = waproto::whatsapp::Message {
        interactive_message: Some(Box::new(
            waproto::whatsapp::message::InteractiveMessage {
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                }),
                footer: request.footer_text.map(|f| {
                    Box::new(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(native_flow),
                ),
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Quick Reply buttons (modern native-flow alternative to ButtonsMessage) ---

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
        (status = 200, description = "Quick reply message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_quick_reply(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendQuickReplyRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

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
            waproto::whatsapp::message::interactive_message::native_flow_message::NativeFlowButton {
                name: Some("quick_reply".to_string()),
                button_params_json: Some(params.to_string()),
            }
        })
        .collect();

    let native_flow = waproto::whatsapp::message::interactive_message::NativeFlowMessage {
        buttons,
        ..Default::default()
    };

    let message = waproto::whatsapp::Message {
        interactive_message: Some(Box::new(
            waproto::whatsapp::message::InteractiveMessage {
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                }),
                footer: request.footer_text.map(|f| {
                    Box::new(waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    })
                }),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(native_flow),
                ),
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Newsletter Admin Invite ---

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
        (status = 200, description = "Newsletter admin invite sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_admin_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterAdminInviteRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        newsletter_admin_invite_message: Some(Box::new(
            waproto::whatsapp::message::NewsletterAdminInviteMessage {
                newsletter_jid: Some(request.newsletter_jid),
                newsletter_name: Some(request.newsletter_name),
                caption: request.caption,
                invite_expiration: request.invite_expiration,
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Newsletter Follower Invite ---

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
        (status = 200, description = "Newsletter follower invite sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_follower_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterFollowerInviteRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        newsletter_follower_invite_message_v2: Some(Box::new(
            waproto::whatsapp::message::NewsletterFollowerInviteMessage {
                newsletter_jid: Some(request.newsletter_jid),
                newsletter_name: Some(request.newsletter_name),
                caption: request.caption,
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Order Message ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/order",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendOrderRequest,
    responses(
        (status = 200, description = "Order message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_order(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendOrderRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let status = request.status.as_deref().and_then(|s| match s {
        "inquiry" => Some(1),
        "accepted" => Some(2),
        "declined" => Some(3),
        _ => None,
    });

    let message = waproto::whatsapp::Message {
        order_message: Some(Box::new(waproto::whatsapp::message::OrderMessage {
            order_id: Some(request.order_id),
            item_count: request.item_count,
            status,
            message: request.message,
            order_title: request.order_title,
            seller_jid: request.seller_jid,
            token: request.token,
            total_amount1000: request.total_amount_1000,
            total_currency_code: request.total_currency_code,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Invoice Message ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/invoice",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendInvoiceRequest,
    responses(
        (status = 200, description = "Invoice message sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_invoice(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInvoiceRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let attachment_type = request.attachment_type.as_deref().and_then(|t| match t {
        "image" => Some(0),
        "pdf" => Some(1),
        _ => None,
    });

    let message = waproto::whatsapp::Message {
        invoice_message: Some(waproto::whatsapp::message::InvoiceMessage {
            note: request.note,
            token: request.token,
            attachment_type,
            attachment_mimetype: request.attachment_mimetype,
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Payment Invite ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/payment-invite",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPaymentInviteRequest,
    responses(
        (status = 200, description = "Payment invite sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentInviteRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        payment_invite_message: Some(waproto::whatsapp::message::PaymentInviteMessage {
            service_type: request.service_type,
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Pin Message ---

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

    let pin_type = if request.duration_seconds > 0 { 1 } else { 2 }; // 1 = PIN, 2 = UNPIN

    let message = waproto::whatsapp::Message {
        pin_in_chat_message: Some(waproto::whatsapp::message::PinInChatMessage {
            key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.chat.clone()),
                id: Some(request.message_id),
                from_me: Some(false),
                ..Default::default()
            }),
            r#type: Some(pin_type),
            sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(chat_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: chat_jid.to_string(),
    }))
}

// --- Forward Message ---

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
        (status = 200, description = "Message forwarded", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn forward_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ForwardMessageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        extended_text_message: Some(Box::new(waproto::whatsapp::message::ExtendedTextMessage {
            text: Some(request.text),
            context_info: Some(Box::new(waproto::whatsapp::ContextInfo {
                is_forwarded: Some(true),
                forwarding_score: Some(1),
                ..Default::default()
            })),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Poll Update (Vote) ---

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
        (status = 200, description = "Poll vote sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_poll_update(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPollUpdateRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
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

    let message = waproto::whatsapp::Message {
        poll_update_message: Some(waproto::whatsapp::message::PollUpdateMessage {
            poll_creation_message_key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.to.clone()),
                id: Some(request.poll_message_id),
                from_me: Some(false),
                ..Default::default()
            }),
            vote: Some(waproto::whatsapp::message::PollEncValue {
                enc_payload,
                enc_iv,
            }),
            sender_timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Buttons Response ---

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
        (status = 200, description = "Buttons response sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_buttons_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendButtonsResponseRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        buttons_response_message: Some(Box::new(
            waproto::whatsapp::message::ButtonsResponseMessage {
            selected_button_id: Some(request.selected_button_id),
            r#type: Some(1), // DisplayText
            context_info: request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            }),
                response: Some(
                    waproto::whatsapp::message::buttons_response_message::Response::SelectedDisplayText(
                        request.selected_display_text,
                    ),
                ),
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- List Response ---

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
        (status = 200, description = "List response sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_list_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendListResponseRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        list_response_message: Some(Box::new(waproto::whatsapp::message::ListResponseMessage {
            title: Some(request.title),
            list_type: Some(1), // SingleSelect
            single_select_reply: Some(
                waproto::whatsapp::message::list_response_message::SingleSelectReply {
                    selected_row_id: Some(request.selected_row_id),
                },
            ),
            description: request.description,
            context_info: request.reply_to.map(|id| {
                Box::new(waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
            }),
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Interactive Response ---

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
        (status = 200, description = "Interactive response sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_interactive_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInteractiveResponseRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let native_flow =
        waproto::whatsapp::message::interactive_response_message::NativeFlowResponseMessage {
            name: Some(request.name),
            params_json: Some(request.params_json),
            version: Some(request.version),
        };

    let message = waproto::whatsapp::Message {
        interactive_response_message: Some(Box::new(
            waproto::whatsapp::message::InteractiveResponseMessage {
                body: request.body_text.map(|text| {
                    waproto::whatsapp::message::interactive_response_message::Body {
                        text: Some(text),
                        ..Default::default()
                    }
                }),
                context_info: request.reply_to.map(|id| {
                    Box::new(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                }),
                interactive_response_message: Some(
                    waproto::whatsapp::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(native_flow),
                ),
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Highly Structured Message ---

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
        (status = 200, description = "HSM sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_highly_structured(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendHighlyStructuredRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        highly_structured_message: Some(Box::new(
            waproto::whatsapp::message::HighlyStructuredMessage {
                namespace: Some(request.namespace),
                element_name: Some(request.element_name),
                params: request.params,
                fallback_lg: request.fallback_lg,
                fallback_lc: request.fallback_lc,
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Template Button Reply ---

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
        (status = 200, description = "Template button reply sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_template_button_reply(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendTemplateButtonReplyRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        template_button_reply_message: Some(Box::new(
            waproto::whatsapp::message::TemplateButtonReplyMessage {
                selected_id: Some(request.selected_id),
                selected_display_text: Some(request.selected_display_text),
                selected_index: request.selected_index,
                context_info: request.reply_to.map(|id| {
                    Box::new(waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                }),
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Comment Message (Groups) ---

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
        (status = 200, description = "Comment sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_comment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendCommentRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    // Builds an EncCommentMessage envelope and ships it as a top-level
    // enc_comment_message; receivers decrypt with the parent's messageSecret.
    let target_jid = request
        .target_chat_jid
        .unwrap_or_else(|| request.to.clone());
    let parent_key = waproto::whatsapp::MessageKey {
        remote_jid: Some(target_jid),
        id: Some(request.target_message_id),
        from_me: Some(false),
        participant: request.target_participant,
    };

    let message_id = client
        .comments()
        .send_text(to_jid.clone(), parent_key, &request.text)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Scheduled Call Creation ---

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
        (status = 200, description = "Scheduled call created", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_scheduled_call(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendScheduledCallRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let call_type = match request.call_type.to_lowercase().as_str() {
        "video" => 2,
        _ => 1, // voice
    };

    let message = waproto::whatsapp::Message {
        scheduled_call_creation_message: Some(
            waproto::whatsapp::message::ScheduledCallCreationMessage {
                scheduled_timestamp_ms: Some(request.scheduled_timestamp_ms),
                call_type: Some(call_type),
                title: request.title,
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Scheduled Call Edit ---

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
        (status = 200, description = "Scheduled call edited", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_scheduled_call_edit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendScheduledCallEditRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let edit_type = match request.edit_type.to_lowercase().as_str() {
        "cancel" => 1,
        _ => 0, // unknown
    };

    let message = waproto::whatsapp::Message {
        scheduled_call_edit_message: Some(waproto::whatsapp::message::ScheduledCallEditMessage {
            key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.to.clone()),
                id: Some(request.scheduled_call_message_id),
                from_me: Some(true),
                ..Default::default()
            }),
            edit_type: Some(edit_type),
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Send Payment ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/send-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendPaymentRequest,
    responses(
        (status = 200, description = "Payment sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let note_message = request.note.map(|text| {
        Box::new(waproto::whatsapp::Message {
            extended_text_message: Some(Box::new(
                waproto::whatsapp::message::ExtendedTextMessage {
                    text: Some(text),
                    ..Default::default()
                },
            )),
            ..Default::default()
        })
    });

    let request_message_key = request
        .request_message_id
        .map(|id| waproto::whatsapp::MessageKey {
            remote_jid: Some(request.to.clone()),
            id: Some(id),
            from_me: Some(false),
            ..Default::default()
        });

    let message = waproto::whatsapp::Message {
        send_payment_message: Some(Box::new(waproto::whatsapp::message::SendPaymentMessage {
            note_message,
            request_message_key,
            transaction_data: request.transaction_data,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Request Payment ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/request-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RequestPaymentRequest,
    responses(
        (status = 200, description = "Payment request sent", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn request_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RequestPaymentRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let note_message = request.note.map(|text| {
        Box::new(waproto::whatsapp::Message {
            extended_text_message: Some(Box::new(
                waproto::whatsapp::message::ExtendedTextMessage {
                    text: Some(text),
                    ..Default::default()
                },
            )),
            ..Default::default()
        })
    });

    let message = waproto::whatsapp::Message {
        request_payment_message: Some(Box::new(
            waproto::whatsapp::message::RequestPaymentMessage {
                note_message,
                currency_code_iso4217: Some(request.currency_code),
                amount1000: Some(request.amount1000),
                request_from: Some(request.to.clone()),
                expiry_timestamp: request.expiry_timestamp,
                ..Default::default()
            },
        )),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Cancel Payment Request ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/cancel-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = CancelPaymentRequestRequest,
    responses(
        (status = 200, description = "Payment request cancelled", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn cancel_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CancelPaymentRequestRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        cancel_payment_request_message: Some(
            waproto::whatsapp::message::CancelPaymentRequestMessage {
                key: Some(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                }),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Decline Payment Request ---

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/decline-payment",
    tag = "messages",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = DeclinePaymentRequestRequest,
    responses(
        (status = 200, description = "Payment request declined", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn decline_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<DeclinePaymentRequestRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        decline_payment_request_message: Some(
            waproto::whatsapp::message::DeclinePaymentRequestMessage {
                key: Some(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                }),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

// --- Newsletter Forward ---

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
        (status = 200, description = "Newsletter message forwarded", body = MessageResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_newsletter_forward(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendNewsletterForwardRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let content_type = match request.content_type.as_deref() {
        Some("update_card") => Some(2),
        Some("link_card") => Some(3),
        _ => Some(1), // update
    };

    let message = waproto::whatsapp::Message {
        extended_text_message: Some(Box::new(waproto::whatsapp::message::ExtendedTextMessage {
            text: Some(request.text),
            context_info: Some(Box::new(waproto::whatsapp::ContextInfo {
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
                ),
                ..Default::default()
            })),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    }))
}

#[allow(dead_code)]
pub async fn send_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    send_text(
        State(state),
        Path(session_id),
        Json(SendTextRequest {
            to: request.to,
            text: request.text,
            reply_to: None,
            fake_reply: None,
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
        .revoke_message(to_jid, &request.message_id, revoke_type)
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

pub(crate) fn get_client(
    state: &AppState,
    session_id: &str,
) -> Result<std::sync::Arc<whatsapp_rust::Client>, ApiError> {
    let runtime = state
        .get_session(session_id)
        .ok_or(ApiError::NotConnected)?;

    runtime.get_client().ok_or(ApiError::NotConnected)
}

pub(crate) fn parse_jid(jid_str: &str) -> Result<Jid, ApiError> {
    if jid_str.contains('@') {
        jid_str
            .parse()
            .map_err(|_| ApiError::InvalidJid(jid_str.to_string()))
    } else {
        Ok(Jid::pn(jid_str))
    }
}

/// Resolve a recipient JID to its actual deliverable address.
///
/// WhatsApp has migrated most contacts to LID-only privacy mode: sending
/// to legacy `phone@s.whatsapp.net` is accepted by the server but never
/// delivered. For any `@s.whatsapp.net` recipient we query the contact
/// directory and substitute the LID when one exists. Already-resolved
/// JIDs (`@lid`, `@g.us`, etc.) and unknown contacts pass through
/// unchanged.
pub(crate) async fn resolve_recipient_jid(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jid: Jid,
) -> Jid {
    use wacore_binary::jid::SERVER_JID;
    if jid.server != SERVER_JID {
        return jid;
    }
    let probe = vec![jid.clone()];
    match do_get_user_info_lite(client, probe).await {
        Ok(map) => map
            .get(&jid)
            .and_then(|info| info.lid.clone())
            .unwrap_or(jid),
        Err(_) => jid,
    }
}

async fn do_get_user_info_lite(
    client: std::sync::Arc<whatsapp_rust::Client>,
    jids: Vec<Jid>,
) -> Result<std::collections::HashMap<Jid, whatsapp_rust::UserInfo>, ()> {
    struct AssertSend<F>(F);
    unsafe impl<F: std::future::Future> Send for AssertSend<F> {}
    impl<F: std::future::Future> std::future::Future for AssertSend<F> {
        type Output = F::Output;
        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll(cx)
        }
    }
    AssertSend(async move { client.contacts().get_user_info(&jids).await.map_err(|_| ()) }).await
}

pub(crate) async fn get_media_data(media: &MediaData) -> Result<(Vec<u8>, String), ApiError> {
    match media {
        MediaData::Url { url } => {
            let response = reqwest::get(url)
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to fetch URL: {}", e)))?;

            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();

            let data = response
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to read response: {}", e)))?
                .to_vec();

            Ok((data, content_type))
        }
        MediaData::Base64 { data, mimetype } => {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| ApiError::BadRequest(format!("Invalid base64: {}", e)))?;
            Ok((decoded, mimetype.clone()))
        }
        MediaData::Uploaded { mimetype: _, .. } => Err(ApiError::BadRequest(
            "Pre-uploaded media not supported in this context".to_string(),
        )),
    }
}

fn build_vcard(contact: &ContactCard) -> String {
    let mut vcard = String::new();
    vcard.push_str("BEGIN:VCARD\n");
    vcard.push_str("VERSION:3.0\n");
    vcard.push_str(&format!("FN:{}\n", contact.display_name));
    vcard.push_str(&format!("N:;{};;;\n", contact.display_name));

    if let Some(org) = &contact.organization {
        vcard.push_str(&format!("ORG:{}\n", org));
    }

    for phone in &contact.phones {
        vcard.push_str(&format!("TEL;type={}:{}\n", phone.phone_type, phone.number));
    }

    vcard.push_str("END:VCARD");
    vcard
}
