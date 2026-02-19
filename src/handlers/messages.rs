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
    let to_jid = parse_jid(&request.to)?;

    let message = waproto::whatsapp::Message {
        extended_text_message: Some(Box::new(waproto::whatsapp::message::ExtendedTextMessage {
            text: Some(request.text),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
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
    let to_jid = parse_jid(&request.to)?;

    let (data, mimetype) = get_media_data(&request.image).await?;

    let upload = client
        .upload(data.clone(), wacore::download::MediaType::Image)
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        image_message: Some(Box::new(waproto::whatsapp::message::ImageMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key),
            file_sha256: Some(upload.file_sha256),
            file_enc_sha256: Some(upload.file_enc_sha256),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            caption: request.caption,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
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
    let to_jid = parse_jid(&request.to)?;

    let (data, mimetype) = get_media_data(&request.video).await?;

    let upload = client
        .upload(data.clone(), wacore::download::MediaType::Video)
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        video_message: Some(Box::new(waproto::whatsapp::message::VideoMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key),
            file_sha256: Some(upload.file_sha256),
            file_enc_sha256: Some(upload.file_enc_sha256),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            caption: request.caption,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
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
    let to_jid = parse_jid(&request.to)?;

    let (data, mimetype) = get_media_data(&request.audio).await?;

    let upload = client
        .upload(data.clone(), wacore::download::MediaType::Audio)
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        audio_message: Some(Box::new(waproto::whatsapp::message::AudioMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key),
            file_sha256: Some(upload.file_sha256),
            file_enc_sha256: Some(upload.file_enc_sha256),
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
    let to_jid = parse_jid(&request.to)?;

    let (data, mimetype) = get_media_data(&request.document).await?;

    let upload = client
        .upload(data.clone(), wacore::download::MediaType::Document)
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        document_message: Some(Box::new(waproto::whatsapp::message::DocumentMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key),
            file_sha256: Some(upload.file_sha256),
            file_enc_sha256: Some(upload.file_enc_sha256),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            file_name: Some(request.filename),
            caption: request.caption,
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
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
    let to_jid = parse_jid(&request.to)?;

    let (data, _mimetype) = get_media_data(&request.sticker).await?;

    let upload = client
        .upload(data.clone(), wacore::download::MediaType::Sticker)
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = waproto::whatsapp::Message {
        sticker_message: Some(Box::new(waproto::whatsapp::message::StickerMessage {
            url: Some(upload.url),
            direct_path: Some(upload.direct_path),
            media_key: Some(upload.media_key),
            file_sha256: Some(upload.file_sha256),
            file_enc_sha256: Some(upload.file_enc_sha256),
            file_length: Some(data.len() as u64),
            mimetype: Some("image/webp".to_string()),
            ..Default::default()
        })),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
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
    let to_jid = parse_jid(&request.to)?;

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
    let to_jid = parse_jid(&request.to)?;

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
    let to_jid = parse_jid(&request.to)?;

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
    let to_jid = parse_jid(&request.to)?;

    let message = waproto::whatsapp::Message {
        reaction_message: Some(waproto::whatsapp::message::ReactionMessage {
            key: Some(waproto::whatsapp::MessageKey {
                remote_jid: Some(request.to.clone()),
                id: Some(request.message_id),
                from_me: Some(false),
                ..Default::default()
            }),
            text: Some(request.emoji),
            ..Default::default()
        }),
        ..Default::default()
    };

    let message_id = client
        .send_message(to_jid.clone(), message)
        .await
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
    let to_jid = parse_jid(&request.to)?;

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

    client
        .mark_as_read(&chat_jid, sender.as_ref(), request.message_ids)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(SuccessResponse { success: true }))
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

fn parse_jid(jid_str: &str) -> Result<Jid, ApiError> {
    if jid_str.contains('@') {
        jid_str
            .parse()
            .map_err(|_| ApiError::InvalidJid(jid_str.to_string()))
    } else {
        Ok(Jid::pn(jid_str))
    }
}

async fn get_media_data(media: &MediaData) -> Result<(Vec<u8>, String), ApiError> {
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
