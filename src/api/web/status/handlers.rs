use axum::{
    extract::{Path, State},
    Json,
};
use wacore_binary::jid::Jid;
use waproto::buffa::MessageField;
use waproto::whatsapp as wa;

use crate::api::common::SuccessResponse;
use crate::api::web::status::models::StatusReactionRequest;
use crate::error::ApiError;
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

    let message = nx!(wa::Message {
        reaction_message: MessageField::some(nx!(wa::message::ReactionMessage {
            key: Some(nx!(wa::MessageKey {
                remote_jid: Some(status_broadcast.to_string()),
                from_me: Some(false),
                id: Some(request.message_id.clone()),
                participant: Some(owner.to_string()),
            }))
            .into(),
            text: Some(request.reaction.clone()),
            grouping_key: None,
            sender_timestamp_ms: Some(now_ms),
        })),
        ..Default::default()
    });

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

/// Everyone a status goes to: the session's known contacts, by phone JID.
async fn status_recipients(state: &AppState, session_id: &str) -> Vec<Jid> {
    let Some(store) = crate::api::shared::messages::handlers::chat_store(state, session_id) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    store
        .contacts()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.jid.to_non_ad())
        .filter(|j| matches!(j.server, wacore_binary::jid::Server::Pn))
        .filter(|j| seen.insert(j.to_string()))
        .collect()
}

fn status_jid() -> Jid {
    Jid::new(
        wacore_binary::jid::STATUS_BROADCAST_USER,
        wacore_binary::jid::Server::Broadcast,
    )
}

async fn record_own_status(state: &AppState, session_id: &str, message: &wa::Message, id: &str) {
    crate::api::shared::search::handlers::record_outgoing(
        state,
        session_id,
        &status_jid(),
        message,
        id,
    )
    .await;
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/status/text",
    tag = "status",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = crate::api::web::status::models::TextStatusRequest,
    responses(
        (status = 200, description = "Text status posted to all contacts", body = crate::api::web::status::models::StatusPostResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn post_text_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<crate::api::web::status::models::TextStatusRequest>,
) -> Result<Json<crate::api::web::status::models::StatusPostResponse>, ApiError> {
    use waproto::buffa::Enumeration;
    let client = get_client(&state, &session_id)?;
    let recipients = status_recipients(&state, &session_id).await;
    let argb = request.background_argb.unwrap_or(0xFF00_7C6C);
    let font = request
        .font
        .and_then(wa::message::extended_text_message::FontType::from_i32)
        .unwrap_or(wa::message::extended_text_message::FontType::SYSTEM);
    let result = client
        .status()
        .send_text(&request.text, argb, font, &recipients, Default::default())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let message = nx!(wa::Message {
        extended_text_message: MessageField::some(nx!(wa::message::ExtendedTextMessage {
            text: Some(request.text),
            background_argb: Some(argb),
            font: Some(font),
            ..Default::default()
        })),
        ..Default::default()
    });
    record_own_status(&state, &session_id, &message, result.message_id.as_str()).await;
    Ok(Json(crate::api::web::status::models::StatusPostResponse {
        message_id: result.message_id.to_string(),
        recipients: recipients.len(),
    }))
}

/// A small JPEG preview of an image, which WhatsApp shows before the full
/// status downloads.
fn jpeg_thumbnail(bytes: &[u8]) -> Vec<u8> {
    let Ok(image) = image::load_from_memory(bytes) else {
        return Vec::new();
    };
    let thumb = image.thumbnail(96, 96).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = image::DynamicImage::ImageRgb8(thumb).write_to(&mut out, image::ImageFormat::Jpeg);
    out.into_inner()
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/status/image",
    tag = "status",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = crate::api::web::status::models::ImageStatusRequest,
    responses(
        (status = 200, description = "Image status posted to all contacts", body = crate::api::web::status::models::StatusPostResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn post_image_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<crate::api::web::status::models::ImageStatusRequest>,
) -> Result<Json<crate::api::web::status::models::StatusPostResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let recipients = status_recipients(&state, &session_id).await;
    let (data, mimetype) =
        crate::api::shared::messages::handlers::get_media_data(&request.image).await?;
    let thumbnail = jpeg_thumbnail(&data);
    let upload = client
        .upload(
            data.clone(),
            wacore::download::MediaType::Image,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;
    let message = nx!(wa::Message {
        image_message: MessageField::some(nx!(wa::message::ImageMessage {
            url: Some(upload.url.clone()),
            direct_path: Some(upload.direct_path.clone()),
            media_key: Some(upload.media_key.to_vec()),
            file_sha256: Some(upload.file_sha256.to_vec()),
            file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
            file_length: Some(data.len() as u64),
            mimetype: Some(mimetype),
            caption: request.caption.clone(),
            jpeg_thumbnail: Some(thumbnail.clone()),
            ..Default::default()
        })),
        ..Default::default()
    });
    let result = client
        .status()
        .send_image(
            upload,
            thumbnail,
            request.caption.as_deref(),
            &recipients,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    record_own_status(&state, &session_id, &message, result.message_id.as_str()).await;
    Ok(Json(crate::api::web::status::models::StatusPostResponse {
        message_id: result.message_id.to_string(),
        recipients: recipients.len(),
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/status/{message_id}/revoke",
    tag = "status",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("message_id" = String, Path, description = "Id of one of your own statuses")
    ),
    responses(
        (status = 200, description = "Status deleted for everyone", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn revoke_status(
    State(state): State<AppState>,
    Path((session_id, message_id)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;
    let recipients = status_recipients(&state, &session_id).await;
    client
        .status()
        .revoke(
            whatsapp_rust::MessageId::new(&message_id)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?,
            &recipients,
            Default::default(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(store) = crate::api::shared::messages::handlers::chat_store(&state, &session_id) {
        let _ = store.record_revoke(&status_jid(), &message_id, chrono::Utc::now());
    }
    Ok(Json(SuccessResponse::new()))
}
