use axum::{
    extract::{Multipart, Path, State},
    Json,
};
use base64::Engine;

use crate::error::ApiError;
use crate::models::media::{MediaType, UploadMediaResponse};
use crate::state::AppState;

/// Upload media file
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/media/upload",
    tag = "media",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Media uploaded", body = UploadMediaResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn upload_media(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<UploadMediaResponse>, ApiError> {
    let client = get_client(&state, &session_id)?;

    let mut file_data: Option<Vec<u8>> = None;
    let mut media_type: Option<MediaType> = None;
    let mut mimetype: Option<String> = None;

    // Process multipart form
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                let content_type = field.content_type().map(|s| s.to_string());
                mimetype = content_type.clone();

                // Infer media type from content type if not specified
                if media_type.is_none() {
                    media_type = content_type.as_ref().and_then(|ct| infer_media_type(ct));
                }

                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?
                        .to_vec(),
                );
            }
            "media_type" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
                media_type = Some(parse_media_type(&value)?);
            }
            "mimetype" => {
                mimetype = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(e.to_string()))?,
                );
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| ApiError::BadRequest("No file provided".to_string()))?;
    let media_type =
        media_type.ok_or_else(|| ApiError::BadRequest("Media type not specified".to_string()))?;
    let mimetype = mimetype.unwrap_or_else(|| get_default_mimetype(&media_type));

    // Upload to WhatsApp servers
    let upload_result = client
        .upload(file_data.clone(), media_type.to_wacore_media_type())
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;

    Ok(Json(UploadMediaResponse {
        url: upload_result.url,
        direct_path: upload_result.direct_path,
        media_key: base64::engine::general_purpose::STANDARD.encode(&upload_result.media_key),
        file_sha256: base64::engine::general_purpose::STANDARD.encode(&upload_result.file_sha256),
        file_enc_sha256: base64::engine::general_purpose::STANDARD
            .encode(&upload_result.file_enc_sha256),
        file_length: file_data.len() as u64,
        media_type,
        mimetype,
    }))
}

fn infer_media_type(content_type: &str) -> Option<MediaType> {
    if content_type.starts_with("image/") {
        if content_type == "image/webp" {
            Some(MediaType::Sticker)
        } else {
            Some(MediaType::Image)
        }
    } else if content_type.starts_with("video/") {
        Some(MediaType::Video)
    } else if content_type.starts_with("audio/") {
        Some(MediaType::Audio)
    } else {
        Some(MediaType::Document)
    }
}

fn parse_media_type(s: &str) -> Result<MediaType, ApiError> {
    match s.to_lowercase().as_str() {
        "image" => Ok(MediaType::Image),
        "video" => Ok(MediaType::Video),
        "audio" => Ok(MediaType::Audio),
        "document" => Ok(MediaType::Document),
        "sticker" => Ok(MediaType::Sticker),
        _ => Err(ApiError::BadRequest(format!("Invalid media type: {}", s))),
    }
}

fn get_default_mimetype(media_type: &MediaType) -> String {
    match media_type {
        MediaType::Image => "image/jpeg".to_string(),
        MediaType::Video => "video/mp4".to_string(),
        MediaType::Audio => "audio/ogg; codecs=opus".to_string(),
        MediaType::Document => "application/octet-stream".to_string(),
        MediaType::Sticker => "image/webp".to_string(),
    }
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
