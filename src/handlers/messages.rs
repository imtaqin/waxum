use axum::{
    extract::{Path, State},
    Json,
};
use base64::Engine;
use wacore_binary::jid::Jid;
use waproto::buffa::{Enumeration, MessageField};

use crate::error::ApiError;
use crate::models::messages::*;
use crate::models::schedule::SendResponse;
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
                .into()
        };

    let message = waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                context_info,
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_image(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendImageRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "image",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_image(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `image` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_image(
    state: &AppState,
    session_id: &str,
    request: SendImageRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
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

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
                .into()
        };

    let message = waproto::whatsapp::Message {
        image_message: MessageField::some(waproto::whatsapp::message::ImageMessage {
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
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_video(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendVideoRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "video",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_video(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `video` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_video(
    state: &AppState,
    session_id: &str,
    request: SendVideoRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
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

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
                .into()
        };

    let message = waproto::whatsapp::Message {
        video_message: MessageField::some(waproto::whatsapp::message::VideoMessage {
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
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_audio(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendAudioRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "audio",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_audio(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `audio` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_audio(
    state: &AppState,
    session_id: &str,
    request: SendAudioRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
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
        audio_message: MessageField::some(waproto::whatsapp::message::AudioMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            ptt: Some(request.ptt),
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_document(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendDocumentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "document",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_document(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `document` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_document(
    state: &AppState,
    session_id: &str,
    request: SendDocumentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
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

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
                .into()
        };

    let message = waproto::whatsapp::Message {
        document_message: MessageField::some(waproto::whatsapp::message::DocumentMessage {
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
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
        (status = 200, description = "Message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_sticker(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendStickerRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "sticker",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_sticker(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `sticker` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_sticker(
    state: &AppState,
    session_id: &str,
    request: SendStickerRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
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
        sticker_message: MessageField::some(waproto::whatsapp::message::StickerMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some("image/webp".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        location_message: MessageField::some(waproto::whatsapp::message::LocationMessage {
            degrees_latitude: Some(request.latitude),
            degrees_longitude: Some(request.longitude),
            name: request.name,
            address: request.address,
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let vcard = build_vcard(&request.contact);

    let message = waproto::whatsapp::Message {
        contact_message: MessageField::some(waproto::whatsapp::message::ContactMessage {
            display_name: Some(request.contact.display_name.clone()),
            vcard: Some(vcard),
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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

    let new_content = waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                ..Default::default()
            },
        ),
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
        poll_creation_message: MessageField::some(
            waproto::whatsapp::message::PollCreationMessage {
                name: Some(request.name),
                options,
                selectable_options_count: Some(request.selectable_count),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
    let client = get_client(state, session_id)?;
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
            )
            .into(),
            r#type: Some(
                waproto::whatsapp::message::buttons_message::button::Type::from_i32(1)
                    .unwrap_or_default(),
            ),
            ..Default::default()
        })
        .collect();

    let header = request
        .header_text
        .map(waproto::whatsapp::message::buttons_message::Header::Text);

    let message = waproto::whatsapp::Message {
        buttons_message: MessageField::some(waproto::whatsapp::message::ButtonsMessage {
            content_text: Some(request.content_text),
            footer_text: request.footer,
            buttons,
            header_type: header.as_ref().map(|_| {
                waproto::whatsapp::message::buttons_message::HeaderType::from_i32(2)
                    .unwrap_or_default()
            }),
            header,
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

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
        interactive_message: MessageField::some(
            waproto::whatsapp::message::InteractiveMessage {
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.description),
                }).into(),
                footer: request.footer.map(|f| {
                    waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    }
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let context_info: MessageField<waproto::whatsapp::ContextInfo> =
        if let Some(ref fake) = request.fake_reply {
            crate::handlers::fake_reply::build_fake_reply_context_info(fake).into()
        } else {
            request
                .reply_to
                .map(|id| waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                })
                .into()
        };

    let interactive = waproto::whatsapp::message::InteractiveMessage {
        body: Some(waproto::whatsapp::message::interactive_message::Body {
            text: Some(request.body_text),
        })
        .into(),
        footer: request
            .footer_text
            .map(
                |f| waproto::whatsapp::message::interactive_message::Footer {
                    text: Some(f),
                    ..Default::default()
                },
            )
            .into(),
        interactive_message: Some(
            waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(
                Box::new(native_flow),
            ),
        ),
        context_info,
        ..Default::default()
    };

    let inner = waproto::whatsapp::Message {
        interactive_message: MessageField::some(interactive),
        ..Default::default()
    };

    let message = if request.view_once {
        waproto::whatsapp::Message {
            view_once_message_v2: MessageField::some(
                waproto::whatsapp::message::FutureProofMessage {
                    message: MessageField::some(inner),
                },
            ),
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

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
        message_params_json: Some("{\"tag\":\"cta_url\"}".to_string()),
        ..Default::default()
    };

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
        Some(waproto::whatsapp::message::ImageMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            ..Default::default()
        })
    } else {
        None
    };

    let header_title = request
        .header_text
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| request.display_text.clone());

    let header = waproto::whatsapp::message::interactive_message::Header {
        title: Some(header_title),
        has_media_attachment: Some(header_media.is_some()),
        media: header_media.map(|img| {
            waproto::whatsapp::__buffa::oneof::message::interactive_message::header::Media::ImageMessage(
                Box::new(img),
            )
        }),
        ..Default::default()
    };

    let message = waproto::whatsapp::Message {
        interactive_message: MessageField::some(
            waproto::whatsapp::message::InteractiveMessage {
                header: MessageField::some(header),
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                }).into(),
                footer: request.footer_text.map(|f| {
                    waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    }
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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
        interactive_message: MessageField::some(
            waproto::whatsapp::message::InteractiveMessage {
                body: Some(waproto::whatsapp::message::interactive_message::Body {
                    text: Some(request.body_text),
                }).into(),
                footer: request.footer_text.map(|f| {
                    waproto::whatsapp::message::interactive_message::Footer {
                        text: Some(f),
                        ..Default::default()
                    }
                }).into(),
                interactive_message: Some(
                    waproto::whatsapp::message::interactive_message::InteractiveMessage::NativeFlowMessage(Box::new(native_flow)),
                ),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Order message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_order(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendOrderRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "order",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_order(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `order` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_order(
    state: &AppState,
    session_id: &str,
    request: SendOrderRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let status = request.status.as_deref().and_then(|s| match s {
        "inquiry" => Some(waproto::whatsapp::message::order_message::OrderStatus::INQUIRY),
        "accepted" => Some(waproto::whatsapp::message::order_message::OrderStatus::ACCEPTED),
        "declined" => Some(waproto::whatsapp::message::order_message::OrderStatus::DECLINED),
        _ => None,
    });

    let message = waproto::whatsapp::Message {
        order_message: MessageField::some(waproto::whatsapp::message::OrderMessage {
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
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Invoice message sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_invoice(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendInvoiceRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "invoice",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_invoice(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `invoice` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_invoice(
    state: &AppState,
    session_id: &str,
    request: SendInvoiceRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let attachment_type = request.attachment_type.as_deref().and_then(|t| match t {
        "image" => Some(waproto::whatsapp::message::invoice_message::AttachmentType::IMAGE),
        "pdf" => Some(waproto::whatsapp::message::invoice_message::AttachmentType::PDF),
        _ => None,
    });

    let message = waproto::whatsapp::Message {
        invoice_message: MessageField::some(waproto::whatsapp::message::InvoiceMessage {
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

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Payment invite sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment_invite(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentInviteRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "payment-invite",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_payment_invite(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `payment-invite` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_payment_invite(
    state: &AppState,
    session_id: &str,
    request: SendPaymentInviteRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        payment_invite_message: MessageField::some(
            waproto::whatsapp::message::PaymentInviteMessage {
                service_type: request.service_type.and_then(
                    waproto::whatsapp::message::payment_invite_message::ServiceType::from_i32,
                ),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
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

    let message = waproto::whatsapp::Message {
        pin_in_chat_message: MessageField::some(waproto::whatsapp::message::PinInChatMessage {
            key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.chat.clone()),
                id: Some(request.message_id),
                from_me: Some(false),
                ..Default::default()
            })
            .into(),
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(request.text),
                context_info: MessageField::some(waproto::whatsapp::ContextInfo {
                    is_forwarded: Some(true),
                    forwarding_score: Some(1),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        poll_update_message: MessageField::some(waproto::whatsapp::message::PollUpdateMessage {
            poll_creation_message_key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.to.clone()),
                id: Some(request.poll_message_id),
                from_me: Some(false),
                ..Default::default()
            })
            .into(),
            vote: Some(waproto::whatsapp::message::PollEncValue {
                enc_payload,
                enc_iv,
            })
            .into(),
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

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        buttons_response_message: MessageField::some(
            waproto::whatsapp::message::ButtonsResponseMessage {
            selected_button_id: Some(request.selected_button_id),
            r#type: Some(waproto::whatsapp::message::buttons_response_message::Type::DISPLAY_TEXT),
            context_info: request.reply_to.map(|id| {
                waproto::whatsapp::ContextInfo {
                    stanza_id: Some(id),
                    ..Default::default()
                }
            }).into(),
                response: Some(
                    waproto::whatsapp::message::buttons_response_message::Response::SelectedDisplayText(
                        request.selected_display_text,
                    ),
                ),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        list_response_message: MessageField::some(
            waproto::whatsapp::message::ListResponseMessage {
                title: Some(request.title),
                list_type: Some(
                    waproto::whatsapp::message::list_response_message::ListType::SINGLE_SELECT,
                ),
                single_select_reply: Some(
                    waproto::whatsapp::message::list_response_message::SingleSelectReply {
                        selected_row_id: Some(request.selected_row_id),
                    },
                )
                .into(),
                description: request.description,
                context_info: request
                    .reply_to
                    .map(|id| waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                    .into(),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let native_flow =
        waproto::whatsapp::message::interactive_response_message::NativeFlowResponseMessage {
            name: Some(request.name),
            params_json: Some(request.params_json),
            version: Some(request.version),
        };

    let message = waproto::whatsapp::Message {
        interactive_response_message: MessageField::some(
            waproto::whatsapp::message::InteractiveResponseMessage {
                body: request.body_text.map(|text| {
                    waproto::whatsapp::message::interactive_response_message::Body {
                        text: Some(text),
                        ..Default::default()
                    }
                }).into(),
                context_info: request.reply_to.map(|id| {
                    waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    }
                }).into(),
                interactive_response_message: Some(
                    waproto::whatsapp::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(Box::new(native_flow)),
                ),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        highly_structured_message: MessageField::some(
            waproto::whatsapp::message::HighlyStructuredMessage {
                namespace: Some(request.namespace),
                element_name: Some(request.element_name),
                params: request.params,
                fallback_lg: request.fallback_lg,
                fallback_lc: request.fallback_lc,
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message = waproto::whatsapp::Message {
        template_button_reply_message: MessageField::some(
            waproto::whatsapp::message::TemplateButtonReplyMessage {
                selected_id: Some(request.selected_id),
                selected_display_text: Some(request.selected_display_text),
                selected_index: request.selected_index,
                context_info: request
                    .reply_to
                    .map(|id| waproto::whatsapp::ContextInfo {
                        stanza_id: Some(id),
                        ..Default::default()
                    })
                    .into(),
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let call_type = match request.call_type.to_lowercase().as_str() {
        "video" => waproto::whatsapp::message::scheduled_call_creation_message::CallType::VIDEO,
        _ => waproto::whatsapp::message::scheduled_call_creation_message::CallType::VOICE,
    };

    let message = waproto::whatsapp::Message {
        scheduled_call_creation_message: MessageField::some(
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

    Ok(MessageResponse {
        message_id,
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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let edit_type = match request.edit_type.to_lowercase().as_str() {
        "cancel" => waproto::whatsapp::message::scheduled_call_edit_message::EditType::CANCEL,
        _ => waproto::whatsapp::message::scheduled_call_edit_message::EditType::UNKNOWN,
    };

    let message = waproto::whatsapp::Message {
        scheduled_call_edit_message: MessageField::some(
            waproto::whatsapp::message::ScheduledCallEditMessage {
                key: Some(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.scheduled_call_message_id),
                    from_me: Some(true),
                    ..Default::default()
                })
                .into(),
                edit_type: Some(edit_type),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Payment sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn send_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendPaymentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "send-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_payment(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `send-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_payment(
    state: &AppState,
    session_id: &str,
    request: SendPaymentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let note_message = request.note.map(|text| waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(text),
                ..Default::default()
            },
        ),
        ..Default::default()
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
        send_payment_message: MessageField::some(waproto::whatsapp::message::SendPaymentMessage {
            note_message: note_message.into(),
            request_message_key: request_message_key.into(),
            transaction_data: request.transaction_data,
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Payment request sent", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn request_payment(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RequestPaymentRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "request-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_request_payment(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `request-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_request_payment(
    state: &AppState,
    session_id: &str,
    request: RequestPaymentRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let note_message = request.note.map(|text| waproto::whatsapp::Message {
        extended_text_message: MessageField::some(
            waproto::whatsapp::message::ExtendedTextMessage {
                text: Some(text),
                ..Default::default()
            },
        ),
        ..Default::default()
    });

    let message = waproto::whatsapp::Message {
        request_payment_message: MessageField::some(
            waproto::whatsapp::message::RequestPaymentMessage {
                note_message: note_message.into(),
                currency_code_iso4217: Some(request.currency_code),
                amount1000: Some(request.amount1000),
                request_from: Some(request.to.clone()),
                expiry_timestamp: request.expiry_timestamp,
                ..Default::default()
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Payment request cancelled", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn cancel_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<CancelPaymentRequestRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "cancel-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_cancel_payment_request(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `cancel-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_cancel_payment_request(
    state: &AppState,
    session_id: &str,
    request: CancelPaymentRequestRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        cancel_payment_request_message: MessageField::some(
            waproto::whatsapp::message::CancelPaymentRequestMessage {
                key: Some(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                })
                .into(),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: to_jid.to_string(),
    })
}

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
        (status = 200, description = "Payment request declined", body = SendResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn decline_payment_request(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<DeclinePaymentRequestRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
        &state,
        &session_id,
        "decline-payment",
        &request,
        request.send_at,
    )
    .await?
    {
        return Ok(Json(scheduled));
    }
    execute_decline_payment_request(&state, &session_id, request)
        .await
        .map(SendResponse::sent)
        .map(Json)
}

/// Core send logic for `decline-payment` messages, split out of the HTTP handler so
/// the background scheduler can dispatch a parked request without an
/// HTTP request in front of it.
pub async fn execute_decline_payment_request(
    state: &AppState,
    session_id: &str,
    request: DeclinePaymentRequestRequest,
) -> Result<MessageResponse, ApiError> {
    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;

    let message = waproto::whatsapp::Message {
        decline_payment_request_message: MessageField::some(
            waproto::whatsapp::message::DeclinePaymentRequestMessage {
                key: Some(waproto::whatsapp::MessageKey {
                    remote_jid: Some(request.to.clone()),
                    id: Some(request.request_message_id),
                    from_me: Some(false),
                    ..Default::default()
                })
                .into(),
            },
        ),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

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
    if let Some(scheduled) = crate::handlers::schedule::maybe_schedule(
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

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
        .map(|r| r.message_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(MessageResponse {
        message_id,
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
            send_at: None,
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

    runtime.get_live_client().ok_or(ApiError::NotConnected)
}

pub(crate) fn parse_jid(jid_str: &str) -> Result<Jid, ApiError> {
    let trimmed = jid_str.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidJid(jid_str.to_string()));
    }
    if trimmed.contains('@') {
        trimmed
            .parse()
            .map_err(|_| ApiError::InvalidJid(trimmed.to_string()))
    } else {
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return Err(ApiError::InvalidJid(trimmed.to_string()));
        }
        Ok(Jid::pn(&digits))
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
