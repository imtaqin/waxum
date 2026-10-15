//! Image, video, audio, document and sticker sends.

use axum::{
    extract::{Path, State},
    Json,
};
use base64::Engine;
use waproto::buffa::MessageField;

use super::common::*;
use crate::api::shared::messages::models::*;
use crate::api::shared::schedule::models::SendResponse;
use crate::error::ApiError;
use crate::state::AppState;

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
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
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
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let media = cloud_media_object(&cloud, &request.image).await?;
        let resp = cloud
            .send_media(
                &request.to,
                "image",
                media,
                request.caption.as_deref(),
                None,
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

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

    let message = nx!(waproto::whatsapp::Message {
        image_message: MessageField::some(nx!(waproto::whatsapp::message::ImageMessage {
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
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
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
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let media = cloud_media_object(&cloud, &request.video).await?;
        let resp = cloud
            .send_media(
                &request.to,
                "video",
                media,
                request.caption.as_deref(),
                None,
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

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

    let message = nx!(waproto::whatsapp::Message {
        video_message: MessageField::some(nx!(waproto::whatsapp::message::VideoMessage {
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
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
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
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let media = cloud_media_object(&cloud, &request.audio).await?;
        let resp = cloud
            .send_media(
                &request.to,
                "audio",
                media,
                None,
                None,
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let (data, mimetype) = get_media_data(&request.audio).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Audio,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = nx!(waproto::whatsapp::Message {
        audio_message: MessageField::some(nx!(waproto::whatsapp::message::AudioMessage {
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
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
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
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let media = cloud_media_object(&cloud, &request.document).await?;
        let resp = cloud
            .send_media(
                &request.to,
                "document",
                media,
                request.caption.as_deref(),
                Some(&request.filename),
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

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

    let message = nx!(waproto::whatsapp::Message {
        document_message: MessageField::some(nx!(waproto::whatsapp::message::DocumentMessage {
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
    if let Some(scheduled) = crate::api::shared::schedule::handlers::maybe_schedule(
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
    if let Some(creds) = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
    {
        let cloud =
            crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
        let media = cloud_media_object(&cloud, &request.sticker).await?;
        let resp = cloud
            .send_media(
                &request.to,
                "sticker",
                media,
                None,
                None,
                request.reply_to.as_deref(),
            )
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        return Ok(cloud_message_response(resp, request.to));
    }

    let client = get_client(state, session_id)?;
    let to_jid = resolve_recipient_jid(client.clone(), parse_jid(&request.to)?).await;
    crate::api::shared::new_chats::handlers::admit(state, session_id, &to_jid, &request.to).await?;

    let (data, _mimetype) = get_media_data(&request.sticker).await?;

    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Sticker,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    let message = nx!(waproto::whatsapp::Message {
        sticker_message: MessageField::some(nx!(waproto::whatsapp::message::StickerMessage {
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

/// Converts a whatsapp-rust-shaped [`MediaData`] into the Cloud API's own
/// `{"link": ...}`/`{"id": ...}` media object, uploading base64 payloads
/// through [`crate::cloud::client::CloudClient::upload_media`] first.
/// `MediaData::Uploaded` (whatsapp-rust's own pre-uploaded media pointer,
/// with its `direct_path`/`media_key`/... fields) has no Cloud API
/// equivalent, so it is rejected rather than silently dropped.
async fn cloud_media_object(
    cloud: &crate::cloud::client::CloudClient,
    media: &MediaData,
) -> Result<serde_json::Value, ApiError> {
    match media {
        MediaData::Url { url } => Ok(serde_json::json!({ "link": url })),
        MediaData::Base64 { data, mimetype } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| ApiError::BadRequest(format!("invalid base64 media data: {e}")))?;
            let resp = cloud
                .upload_media(bytes, mimetype, "upload")
                .await
                .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;
            let id = resp
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ApiError::MediaUploadFailed("cloud upload response missing id".to_string()))?;
            Ok(serde_json::json!({ "id": id }))
        }
        MediaData::Uploaded { .. } => Err(ApiError::BadRequest(
            "uploaded media pointers are whatsapp_web-specific; use image.url or image.data for whatsapp_cloud sessions".to_string(),
        )),
    }
}
