use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::webhooks::{
    RegisterWebhookRequest, WebhookConfig, WebhookConfigWithId, WebhookDlqListResponse,
    WebhookListResponse,
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

    crate::net_guard::validate_public_url(&request.url)
        .await
        .map_err(ApiError::BadRequest)?;

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

/// Manually flip a webhook row back to enabled=true after the auto-disable
/// circuit muted it. Used by operators once the downstream target has
/// been fixed. Also clears `disabled_at` / `disabled_reason` so the
/// listing reflects the recovered state.
#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks/{webhook_id}/enable",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("webhook_id" = String, Path, description = "Webhook ID")
    ),
    responses(
        (status = 200, description = "Webhook re-enabled", body = SuccessResponse),
        (status = 404, description = "Webhook not found")
    )
)]
pub async fn reenable_webhook(
    State(state): State<AppState>,
    Path((session_id, webhook_id)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let flipped = state
        .session_manager()
        .enable_webhook(&webhook_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if !flipped {
        return Err(ApiError::WebhookNotFound(webhook_id));
    }
    state.set_webhook_enabled(&session_id, &webhook_id, true);

    tracing::info!(
        "Session {}: Webhook {} manually re-enabled",
        session_id,
        webhook_id
    );

    Ok(Json(SuccessResponse::with_message("Webhook re-enabled")))
}

/// List the session's in-memory dead-letter queue: deliveries that
/// exhausted every retry attempt. Newest first. The queue is bounded
/// (`WEBHOOK_DLQ_CAPACITY`, default 100 per session) and volatile — it
/// is lost on restart.
#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks/dlq",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Dead-lettered webhook deliveries, newest first", body = WebhookDlqListResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn list_webhook_dlq(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<WebhookDlqListResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    let entries = state.webhook_dlq_list(&session_id);
    let count = entries.len();

    Ok(Json(WebhookDlqListResponse { entries, count }))
}

/// Replay one DLQ entry: re-delivers the stored payload verbatim through
/// the same signing and retry path as a fresh event. The entry is
/// removed up front; if the re-delivery also exhausts its retries it
/// lands back in the DLQ as a new entry.
#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/sessions/{session_id}/webhooks/dlq/{entry_id}/replay",
    tag = "webhooks",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("entry_id" = String, Path, description = "DLQ entry ID")
    ),
    responses(
        (status = 200, description = "Replay scheduled", body = SuccessResponse),
        (status = 400, description = "Circuit open for this webhook URL"),
        (status = 404, description = "DLQ entry not found")
    )
)]
pub async fn replay_webhook_dlq_entry(
    State(state): State<AppState>,
    Path((session_id, entry_id)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let entry = state
        .webhook_dlq_take(&session_id, &entry_id)
        .ok_or_else(|| ApiError::WebhookNotFound(entry_id.clone()))?;

    if !state.webhook_circuit_allows(&entry.webhook_url) {
        state.webhook_dlq_push(entry.clone());
        return Err(ApiError::BadRequest(format!(
            "webhook circuit is OPEN for {} — replay skipped, try again later",
            entry.webhook_url
        )));
    }

    tracing::info!(
        "Session {}: Replaying webhook DLQ entry {} to {}",
        session_id,
        entry_id,
        entry.webhook_url
    );

    let state_for_task = state.clone();
    tokio::spawn(async move {
        state_for_task
            .deliver_webhook(
                &entry.session_id,
                &entry.event,
                &entry.webhook_url,
                entry.secret,
                &entry.payload,
            )
            .await;
    });

    Ok(Json(SuccessResponse::with_message(
        "Webhook DLQ entry replay scheduled",
    )))
}
