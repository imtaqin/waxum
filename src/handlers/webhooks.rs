use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::webhooks::{
    RegisterWebhookRequest, WebhookConfig, WebhookConfigWithId, WebhookListResponse,
};
use crate::state::AppState;

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of webhooks", body = WebhookListResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn list_webhooks(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<WebhookListResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    let webhooks: Vec<WebhookConfigWithId> = state
        .get_webhooks(&session_id)
        .into_iter()
        .map(WebhookConfigWithId::from)
        .collect();
    let count = webhooks.len();

    Ok(Json(WebhookListResponse { webhooks, count }))
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = RegisterWebhookRequest,
    responses(
        (status = 200, description = "Webhook registered", body = WebhookConfig),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Webhook already exists")
    )
)]
pub async fn register_webhook(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<RegisterWebhookRequest>,
) -> Result<Json<WebhookConfig>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "Webhook URL must start with http:// or https://".to_string(),
        ));
    }

    let existing = state.get_webhooks(&session_id);
    if existing.iter().any(|(_, w)| w.url == request.url) {
        return Err(ApiError::WebhookAlreadyExists(request.url));
    }

    let id = Uuid::new_v4().to_string();

    let config = WebhookConfig {
        url: request.url,
        events: request.events,
        secret: request.secret,
        enabled: true,
    };

    state.register_webhook(&session_id, &id, config.clone());

    let _ = state
        .session_manager()
        .create_webhook(&id, &session_id, &config)
        .await;

    tracing::info!("Session {}: Registered webhook: {}", session_id, config.url);

    Ok(Json(config))
}

#[utoipa::path(
    delete,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks/{webhook_id}",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("webhook_id" = String, Path, description = "Webhook ID")
    ),
    responses(
        (status = 200, description = "Webhook unregistered", body = SuccessResponse),
        (status = 404, description = "Webhook not found")
    )
)]
pub async fn unregister_webhook(
    State(state): State<AppState>,
    Path((session_id, webhook_id)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let removed = state.remove_webhook(&session_id, &webhook_id);

    if removed.is_none() {
        return Err(ApiError::WebhookNotFound(webhook_id));
    }

    let _ = state.session_manager().delete_webhook(&webhook_id).await;

    tracing::info!(
        "Session {}: Unregistered webhook: {}",
        session_id,
        webhook_id
    );

    Ok(Json(SuccessResponse::with_message("Webhook unregistered")))
}
