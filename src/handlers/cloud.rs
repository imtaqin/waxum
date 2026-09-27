//! WhatsApp Cloud API session management + webhook receiver.
//!
//! These routes are the Cloud API counterpart of the existing
//! QR/pair-based session lifecycle: `connect_cloud` attaches Meta
//! credentials to a session instead of scanning a QR code, and
//! `cloud_webhook_verify`/`cloud_webhook_receive` are the per-session
//! delivery endpoints Meta calls directly -- exempted from the normal
//! bearer-auth middleware in [`crate::middleware::jwt`] since Meta only
//! ever presents its own `X-Hub-Signature-256`, never a waxum token.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::collections::HashMap;

use crate::cloud::webhook;
use crate::error::ApiError;
use crate::models::cloud::{ConnectCloudRequest, ConnectCloudResponse};
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
    let session = manager
        .get_session(&session_id)
        .await?
        .ok_or(ApiError::SessionNotFound(session_id))?;
    Ok(Json(ConnectCloudResponse { session }))
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

    StatusCode::OK
}
