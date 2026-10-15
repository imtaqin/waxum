//! Create, list, read and delete sessions, plus status, diagnostics and device info.

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::api::common::SuccessResponse;
use crate::api::shared::sessions::models::{
    ConnectRequest, CreateSessionRequest, CreateSessionResponse, DeviceInfo, SessionInfo,
    SessionListResponse, SessionStatus, SessionStatusResponse,
};
use crate::api::shared::webhooks::models::{WebhookConfig, WebhookEvent};
use crate::error::ApiError;
use crate::state::AppState;

use super::*;

#[utoipa::path(
    post,
    path = "/api/v1/sessions",
    tag = "sessions",
    security(("bearer_auth" = [])),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created and connecting", body = CreateSessionResponse),
        (status = 200, description = "Existing session reused (re-scan started on the same slot)", body = CreateSessionResponse),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Session ID already exists (set `reuse: true` to re-scan the same slot)")
    )
)]
pub async fn create_session(
    State(state): State<AppState>,
    scope: Option<axum::Extension<crate::middleware::jwt::TokenScope>>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, ApiError> {
    if let Some(axum::Extension(scope)) = &scope {
        let allowed = request.id.as_deref().is_some_and(|id| scope.0.covers(id));
        if !allowed {
            return Err(ApiError::Forbidden(
                "a scoped token can only create a session whose id starts with one of its prefixes; pass that id explicitly".to_string(),
            ));
        }
    }
    let session_id = request.id.unwrap_or_else(|| Uuid::new_v4().to_string());

    if let Some(existing) = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        if !request.reuse.unwrap_or(false) {
            return Err(ApiError::AlreadyConnected);
        }
        let connect_req = request
            .device
            .map(|d| Json(ConnectRequest { device: Some(d) }));
        let _ = connect_session(State(state), Path(session_id), connect_req).await?;
        return Ok(Json(CreateSessionResponse { session: existing }));
    }

    let storage_path = format!("{}/{}", state.base_storage_path(), session_id);
    tokio::fs::create_dir_all(&storage_path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let session = state
        .session_manager()
        .create_session(&session_id, request.name.as_deref(), &storage_path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Some(webhook_req) = request.webhook {
        let webhook_id = Uuid::new_v4().to_string();
        let events = if webhook_req.events.is_empty() {
            vec![WebhookEvent::All]
        } else {
            webhook_req.events
        };
        let config = WebhookConfig {
            url: webhook_req.url,
            events,
            secret: webhook_req.secret,
            enabled: true,
        };

        state
            .session_manager()
            .create_webhook(&webhook_id, &session_id, &config)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        state.register_webhook(&session_id, &webhook_id, config);
    }

    let runtime = state.get_or_create_session(&session_id, &storage_path);
    runtime.set_status(SessionStatus::Connecting);

    let device_override = request.device.as_ref().map(|d| {
        crate::device_props::resolve_with_override(
            d.os.as_deref(),
            d.platform.as_deref(),
            d.version.as_deref(),
        )
    });

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone, device_override).await {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
            }
        }
    });

    Ok(Json(CreateSessionResponse { session }))
}

#[derive(serde::Deserialize)]
pub struct ListSessionsQuery {
    pub tag: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions",
    tag = "sessions",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of sessions", body = SessionListResponse)
    )
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    scope: Option<axum::Extension<crate::middleware::jwt::TokenScope>>,
    axum::extract::Query(q): axum::extract::Query<ListSessionsQuery>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let tag_filter = q.tag.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty());
    let allowed: Option<std::collections::HashSet<String>> =
        tag_filter.map(|t| state.sessions_with_tag(t).into_iter().collect());

    let mut updated_sessions = Vec::with_capacity(sessions.len());
    for mut session in sessions {
        if let Some(ref set) = allowed {
            if !set.contains(&session.id) {
                continue;
            }
        }
        if let Some(axum::Extension(scope)) = &scope {
            if !scope.0.covers(&session.id) {
                continue;
            }
        }
        if let Some(runtime) = state.get_session(&session.id) {
            session.status = runtime.effective_status();
            session.is_logged_in = session.status == SessionStatus::LoggedIn;
        }
        updated_sessions.push(session);
    }

    let total = updated_sessions.len();
    Ok(Json(SessionListResponse {
        sessions: updated_sessions,
        total,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session info", body = SessionInfo),
        (status = 404, description = "Session not found")
    )
)]
pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionInfo>, ApiError> {
    let mut session = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        session.status = runtime.effective_status();
        session.is_logged_in = session.status == SessionStatus::LoggedIn;
    }

    Ok(Json(session))
}

#[utoipa::path(
    delete,
    path = "/api/v1/sessions/{session_id}",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session deleted", body = SuccessResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    tracing::warn!(session_id = %session_id, "session: delete requested (storage + registry will be purged)");

    if let Some(runtime) = state.get_session(&session_id) {
        if let Some(client) = runtime.get_client() {
            client.disconnect().await;
        }
    }

    state.remove_session(&session_id);
    state.purge_webhooks_for_session(&session_id);
    state.drop_tags_for(&session_id).await;
    crate::db::new_chats::purge(state.session_manager().pool(), &session_id).await;

    if let Some(storage_path) = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        let _ = tokio::fs::remove_dir_all(&storage_path).await;
    }

    let deleted = state
        .session_manager()
        .delete_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if deleted {
        Ok(Json(SuccessResponse::with_message("Session deleted")))
    } else {
        Err(ApiError::SessionNotFound(session_id))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/status",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session status", body = SessionStatusResponse),
        (status = 404, description = "Session not found")
    )
)]
pub async fn get_session_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionStatusResponse>, ApiError> {
    let session = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    let (status, is_logged_in, socket_alive, paused, pair, reachability) =
        if is_cloud_session(&session) {
            (
                SessionStatus::LoggedIn,
                true,
                false,
                false,
                crate::api::shared::sessions::models::PairStatus::default(),
                None,
            )
        } else if let Some(runtime) = state.get_session(&session_id) {
            let ps = runtime.get_pair_state();
            let pair = crate::api::shared::sessions::models::PairStatus {
                last_qr_at: ps.last_qr_at,
                last_pair_code_at: ps.last_pair_code_at,
                pair_code_expires_at: ps.pair_code_expires_at,
                last_error: ps.last_error,
                attempts: ps.attempts,
            };
            let socket_alive = runtime.socket_alive();
            let client = runtime.get_client();
            let paused = client.as_ref().map(|c| c.is_paused()).unwrap_or(false);
            let reachability = client.as_ref().map(|c| reachability_str(c.reachability()));
            if runtime.is_alive() {
                (
                    SessionStatus::LoggedIn,
                    true,
                    socket_alive,
                    paused,
                    pair,
                    reachability,
                )
            } else {
                let s = runtime.get_status();
                let (status, is_logged_in) = if s == SessionStatus::LoggedIn {
                    (SessionStatus::Connecting, true)
                } else {
                    (s, false)
                };
                (
                    status,
                    is_logged_in,
                    socket_alive,
                    paused,
                    pair,
                    reachability,
                )
            }
        } else {
            (
                session.status,
                session.is_logged_in,
                false,
                false,
                crate::api::shared::sessions::models::PairStatus::default(),
                None,
            )
        };

    let mut diagnostics = session_diagnostics(&state, &session_id);
    diagnostics.new_chats =
        crate::api::shared::new_chats::handlers::counts(&state, &session_id).await;
    Ok(Json(SessionStatusResponse {
        status,
        is_logged_in,
        socket_alive,
        paused,
        phone_number: session.phone_number,
        push_name: session.push_name,
        pair,
        reachability,
        diagnostics,
    }))
}

pub(super) fn session_diagnostics(
    state: &AppState,
    session_id: &str,
) -> crate::api::shared::sessions::models::SessionDiagnostics {
    use std::sync::atomic::Ordering;
    let mut d = crate::api::shared::sessions::models::SessionDiagnostics::default();
    if let Some(runtime) = state.get_session(session_id) {
        d.messages_forwarded = runtime.messages_forwarded.load(Ordering::Relaxed);
        let last = runtime.last_message_forwarded_at.load(Ordering::Relaxed);
        d.last_message_forwarded_at = (last > 0).then_some(last);
        if let Some(client) = runtime.get_client() {
            let s = client.stats();
            d.last_data_received_at =
                (s.last_data_received_ms > 0).then_some((s.last_data_received_ms / 1000) as i64);
            d.client_frames_received = Some(s.frames_received);
            d.client_messages_received = Some(s.messages_received);
            d.client_messages_suppressed_duplicate = Some(s.messages_suppressed_duplicate);
            d.client_events_dropped = Some(s.events_dropped);
            d.client_reconnects = Some(s.reconnects);
            d.client_reconnect_errors = Some(s.reconnect_errors);
        }
    }
    d.webhooks = state
        .get_webhooks(session_id)
        .into_iter()
        .map(|(id, config)| {
            let circuit = state.webhook_circuit_snapshot(&config.url);
            crate::api::shared::sessions::models::WebhookHealth {
                id,
                receives_messages: config.events.iter().any(|e| e.matches("message")),
                enabled: config.enabled,
                url: config.url,
                circuit_open: circuit.open,
                consecutive_failures: circuit.failures,
                retry_in_seconds: circuit.retry_in_secs,
                last_error: circuit.last_error,
                skipped_while_suspended: circuit.skipped,
            }
        })
        .collect();
    d
}

/// `true` for a session attached to Meta's WhatsApp Cloud API.
pub(crate) fn is_cloud_session(session: &SessionInfo) -> bool {
    session.provider == "whatsapp_cloud"
}

/// Looks the session up and reports whether it is a `whatsapp_cloud`
/// session. A lookup failure counts as "not cloud", so a transient DB
/// error never blocks a normal multi-device connect.
pub(super) async fn session_is_cloud(state: &AppState, session_id: &str) -> bool {
    matches!(
        state.session_manager().get_session(session_id).await,
        Ok(Some(s)) if is_cloud_session(&s)
    )
}

/// Refuses a multi-device lifecycle operation (connect, pair, QR wait) on
/// a `whatsapp_cloud` session. Such a session has no socket to open: its
/// credentials are its connection, so starting a whatsapp-rust client
/// there would just try to pair a second, unrelated WhatsApp account.
pub(super) fn reject_cloud_session(session: &SessionInfo, op: &str) -> Result<(), ApiError> {
    if is_cloud_session(session) {
        return Err(ApiError::BadRequest(format!(
            "{op} is not applicable to a whatsapp_cloud session; it is connected through its Cloud API credentials"
        )));
    }
    Ok(())
}

/// `whatsapp_rust::Reachability` carries no `Serialize`/`Display` of its own
/// (see its upstream doc: reported by `Client::reachability`, waited out by
/// `Client::wait_until_reachable`) -- own the string mapping here so a
/// renamed/added upstream variant is a compile error, not a silently missing
/// value on the wire.
pub(super) fn reachability_str(r: whatsapp_rust::Reachability) -> String {
    use whatsapp_rust::Reachability;
    match r {
        Reachability::Reachable => "reachable",
        Reachability::Reconnecting => "reconnecting",
        Reachability::Paused => "paused",
        Reachability::Unsupervised => "unsupervised",
        Reachability::Finished => "finished",
        _ => "unknown",
    }
    .to_string()
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/device",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Device info", body = DeviceInfo),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_device_info(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<DeviceInfo>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    let runtime = state
        .get_session(&session_id)
        .ok_or(ApiError::NotConnected)?;

    let client = runtime.get_client().ok_or(ApiError::NotConnected)?;

    let push_name_str = client.push_name();
    let push_name = if push_name_str.is_empty() {
        None
    } else {
        Some(push_name_str)
    };
    let pn = client.pn().map(|j| j.to_string());
    let lid = client.lid().map(|j| j.to_string());

    Ok(Json(DeviceInfo {
        device_id: None,
        phone_number: pn,
        lid,
        push_name,
    }))
}

/// Loads a session's webhooks from the database into the in-memory
/// registry the fan-out reads.
///
/// Runs for every session at boot, whatever its state. It used to run
/// only for sessions that were about to reconnect, so a session that was
/// disconnected, logged out or on the Cloud API when waxum restarted came
/// back with no webhooks until they were registered again.
pub(super) async fn load_session_webhooks(state: &AppState, session_id: &str) {
    match state.session_manager().get_webhooks(session_id).await {
        Ok(rows) => {
            for (webhook_id, config) in rows {
                state.register_webhook(session_id, &webhook_id, config);
            }
        }
        Err(e) => tracing::warn!("[startup] get_webhooks failed for {}: {}", session_id, e),
    }
}
