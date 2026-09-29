//! WhatsApp Cloud API session management + webhook receiver.
//!
//! These routes are the Cloud API counterpart of the existing
//! QR/pair-based session lifecycle: `connect_cloud` attaches Meta
//! credentials to a session instead of scanning a QR code, and
//! `cloud_webhook_verify`/`cloud_webhook_receive` are the per-session
//! delivery endpoints Meta calls directly -- exempted from the normal
//! bearer-auth middleware in [`crate::middleware::jwt`] since Meta only
//! ever presents its own `X-Hub-Signature-256`, never a waxum token.
//!
//! `POST /sessions` starts a multi-device client right away, so a session
//! created and then attached to the Cloud API would otherwise keep that
//! client running (and asking for a QR scan) next to its Cloud
//! credentials. `connect_cloud` disconnects and drops any such runtime,
//! and `connect_client` re-checks the provider before opening a socket
//! in case the client was still being built.

use axum::{
    body::Bytes,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::collections::HashMap;

use crate::cloud::{embedded_signup, webhook};
use crate::error::ApiError;
use crate::models::cloud::{
    ConnectCloudRequest, ConnectCloudResponse, EmbeddedSignupExchangeRequest,
    EmbeddedSignupExchangeResponse, SendTemplateRequest,
};
use crate::models::messages::MessageResponse;
use crate::state::AppState;

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/connect",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = ConnectCloudRequest,
    responses(
        (status = 200, description = "Cloud credentials attached", body = ConnectCloudResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn connect_cloud(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ConnectCloudRequest>,
) -> Result<Json<ConnectCloudResponse>, ApiError> {
    let manager = state.session_manager();
    if manager.get_session(&session_id).await?.is_none() {
        return Err(ApiError::SessionNotFound(session_id));
    }
    manager.connect_cloud(&session_id, &request).await?;
    if let Some(runtime) = state.remove_session(&session_id) {
        if let Some(client) = runtime.get_client() {
            client.disconnect().await;
        }
        runtime.set_client(None);
    }
    let session = manager
        .get_session(&session_id)
        .await?
        .ok_or(ApiError::SessionNotFound(session_id))?;
    Ok(Json(ConnectCloudResponse { session }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/cloud/embedded-signup/exchange",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = EmbeddedSignupExchangeRequest,
    responses(
        (status = 200, description = "Exchanged token + WABA phone numbers", body = EmbeddedSignupExchangeResponse),
        (status = 502, description = "Meta rejected the exchange or a follow-up call")
    )
)]
pub async fn embedded_signup_exchange(
    Path(_session_id): Path<String>,
    Json(request): Json<EmbeddedSignupExchangeRequest>,
) -> Result<Json<EmbeddedSignupExchangeResponse>, ApiError> {
    let token_resp =
        embedded_signup::exchange_code(&request.app_id, &request.app_secret, &request.code)
            .await
            .map_err(|e| {
                ApiError::Internal(format!("embedded signup token exchange failed: {e}"))
            })?;
    let access_token = token_resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError::Internal("token exchange response had no access_token".to_string())
        })?
        .to_string();

    embedded_signup::subscribe_app(&request.waba_id, &access_token)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to subscribe app to WABA: {e}")))?;

    let phone_numbers = embedded_signup::list_phone_numbers(&request.waba_id, &access_token)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to list WABA phone numbers: {e}")))?;

    Ok(Json(EmbeddedSignupExchangeResponse {
        access_token,
        waba_id: request.waba_id,
        phone_numbers,
    }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/messages/template",
    tag = "cloud",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SendTemplateRequest,
    responses(
        (status = 200, description = "Template message sent", body = MessageResponse),
        (status = 400, description = "Not supported for whatsapp_web sessions"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn send_template(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<SendTemplateRequest>,
) -> Result<Json<MessageResponse>, ApiError> {
    let creds = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest(
                "messages/template is only supported for whatsapp_cloud sessions".to_string(),
            )
        })?;

    let cloud = crate::cloud::client::CloudClient::new(&creds.phone_number_id, &creds.access_token);
    let resp = cloud
        .send_template(
            &request.to,
            &request.name,
            &request.language_code,
            request.components,
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
    Ok(Json(MessageResponse {
        message_id,
        timestamp: chrono::Utc::now().timestamp(),
        to: request.to,
    }))
}

/// `GET /sessions/{session_id}/cloud/webhook` -- Meta's verification
/// handshake, sent once when the webhook URL is registered on the app
/// dashboard and re-sent whenever it's changed.
pub async fn cloud_webhook_verify(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Ok(Some(creds)) = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await
    else {
        return (StatusCode::NOT_FOUND, "unknown cloud session".to_string()).into_response();
    };

    match webhook::verify_challenge(
        &creds.webhook_verify_token,
        params.get("hub.mode").map(String::as_str),
        params.get("hub.verify_token").map(String::as_str),
        params.get("hub.challenge").map(String::as_str),
    ) {
        Some(challenge) => (StatusCode::OK, challenge.to_string()).into_response(),
        None => (StatusCode::FORBIDDEN, "verification failed".to_string()).into_response(),
    }
}

/// `POST /sessions/{session_id}/cloud/webhook` -- an actual message/status
/// delivery. Verifies `X-Hub-Signature-256` against the session's own
/// `cloud_app_secret` before touching the body, then normalizes any
/// inbound `messages[]` into the same `message` webhook event Web
/// sessions emit and fans it out through the existing
/// `broadcast_to_webhooks`/NATS pipeline.
pub async fn cloud_webhook_receive(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Ok(Some(creds)) = state
        .session_manager()
        .get_cloud_credentials(&session_id)
        .await
    else {
        return StatusCode::NOT_FOUND;
    };

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok());
    if !webhook::verify_signature(&creds.app_secret, &body, signature) {
        tracing::warn!(session_id = %session_id, "cloud webhook: signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };

    let timestamp = chrono::Utc::now().timestamp();
    for data in webhook::normalize_messages(&payload) {
        let envelope = serde_json::json!({
            "session_id": session_id,
            "event": "message",
            "timestamp": timestamp,
            "offline": false,
            "data": data,
        });
        if let Ok(payload_str) = serde_json::to_string(&envelope) {
            state
                .broadcast_to_webhooks(&session_id, "message", &payload_str)
                .await;
            state
                .publish_to_nats(&session_id, "message", &payload_str)
                .await;
        }
    }
    for data in webhook::normalize_statuses(&payload) {
        let envelope = serde_json::json!({
            "session_id": session_id,
            "event": "receipt",
            "timestamp": timestamp,
            "offline": false,
            "data": data,
        });
        if let Ok(payload_str) = serde_json::to_string(&envelope) {
            state
                .broadcast_to_webhooks(&session_id, "receipt", &payload_str)
                .await;
            state
                .publish_to_nats(&session_id, "receipt", &payload_str)
                .await;
        }
    }

    StatusCode::OK
}

async fn cloud_client_for(
    state: &AppState,
    session_id: &str,
) -> Result<crate::cloud::client::CloudClient, ApiError> {
    let creds = state
        .session_manager()
        .get_cloud_credentials(session_id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest("session is not a whatsapp_cloud session".to_string())
        })?;
    Ok(crate::cloud::client::CloudClient::new(
        &creds.phone_number_id,
        &creds.access_token,
    ))
}

/// `POST /sessions/{session_id}/cloud/media` (multipart, field `file`) --
/// uploads a media file to the Cloud API and returns its media ID, per
/// "Upload Image"/"Upload Sticker"/"Upload Audio". Cloud-only: there is
/// no shared schema with the whatsapp-rust `/media/upload` endpoint,
/// whose response carries `direct_path`/`media_key`/`file_sha256` --
/// concepts the Cloud API has no equivalent for, since it hands back a
/// single opaque media ID instead of a raw upload pointer.
pub async fn cloud_upload_media(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cloud = cloud_client_for(&state, &session_id).await?;

    let mut file_data: Option<Vec<u8>> = None;
    let mut mime_type: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            mime_type = field.content_type().map(str::to_string);
            file_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?
                    .to_vec(),
            );
        }
    }
    let file_data =
        file_data.ok_or_else(|| ApiError::BadRequest("No file provided".to_string()))?;
    let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_string());

    let resp = cloud
        .upload_media(file_data, &mime_type, "upload")
        .await
        .map_err(|e| ApiError::MediaUploadFailed(e.to_string()))?;
    Ok(Json(resp))
}

/// `GET /sessions/{session_id}/cloud/media/{media_id}` -- resolves a
/// media ID to its metadata + short-lived download URL, per "Retrieve
/// Media URL".
pub async fn cloud_get_media(
    State(state): State<AppState>,
    Path((session_id, media_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cloud = cloud_client_for(&state, &session_id).await?;
    let resp = cloud
        .get_media_url(&media_id)
        .await
        .map_err(|e| ApiError::MediaDownloadFailed(e.to_string()))?;
    Ok(Json(resp))
}

/// `GET /sessions/{session_id}/cloud/media/{media_id}/download` --
/// resolves the media ID then fetches the bytes from the returned URL,
/// per "Download Media", and streams them back as the raw body.
pub async fn cloud_download_media(
    State(state): State<AppState>,
    Path((session_id, media_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let cloud = cloud_client_for(&state, &session_id).await?;
    let meta = cloud
        .get_media_url(&media_id)
        .await
        .map_err(|e| ApiError::MediaDownloadFailed(e.to_string()))?;
    let url = meta
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::MediaDownloadFailed("media metadata had no url".to_string()))?;
    let mime_type = meta
        .get("mime_type")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = cloud
        .download_media_bytes(url)
        .await
        .map_err(|e| ApiError::MediaDownloadFailed(e.to_string()))?;
    Ok(([("content-type", mime_type)], bytes))
}

/// `DELETE /sessions/{session_id}/cloud/media/{media_id}`, per "Delete
/// Media".
pub async fn cloud_delete_media(
    State(state): State<AppState>,
    Path((session_id, media_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cloud = cloud_client_for(&state, &session_id).await?;
    let resp = cloud
        .delete_media(&media_id)
        .await
        .map_err(|e| ApiError::MediaDownloadFailed(e.to_string()))?;
    Ok(Json(resp))
}
