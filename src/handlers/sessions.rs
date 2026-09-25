use std::time::Duration;

use axum::{
    extract::{Multipart, Path, Query, State},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    response::Response as AxumResponse,
    Json,
};
use futures::stream::Stream;
use uuid::Uuid;

use crate::device_props::ResolvedDeviceProps;
use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::sessions::{
    ConnectRequest, CreateSessionRequest, CreateSessionResponse, DeviceInfo, PairCodeRequest,
    PairCodeResponse, QrCodeResponse, SessionInfo, SessionListResponse, SessionStatus,
    SessionStatusResponse,
};
use crate::models::webhooks::{WebhookConfig, WebhookEvent};
use crate::state::AppState;

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
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, ApiError> {
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
        if let Some(runtime) = state.get_session(&session_id) {
            let ps = runtime.get_pair_state();
            let pair = crate::models::sessions::PairStatus {
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
                crate::models::sessions::PairStatus::default(),
                None,
            )
        };

    Ok(Json(SessionStatusResponse {
        status,
        is_logged_in,
        socket_alive,
        paused,
        phone_number: session.phone_number,
        push_name: session.push_name,
        pair,
        reachability,
    }))
}

/// `whatsapp_rust::Reachability` carries no `Serialize`/`Display` of its own
/// (see its upstream doc: reported by `Client::reachability`, waited out by
/// `Client::wait_until_reachable`) -- own the string mapping here so a
/// renamed/added upstream variant is a compile error, not a silently missing
/// value on the wire.
fn reachability_str(r: whatsapp_rust::Reachability) -> String {
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
    path = "/api/v1/sessions/{session_id}/qr",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "QR codes", body = QrCodeResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn get_qr_code(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<QrCodeResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    let runtime = state
        .get_session(&session_id)
        .ok_or(ApiError::NotConnected)?;

    Ok(Json(QrCodeResponse {
        qr_codes: runtime.get_qr_codes(),
        timeout_seconds: 60,
        status: runtime.get_status(),
    }))
}

/// Forces a clean rebuild when the session isn't actually live: cleanly
/// disconnects whatever client is cached (if any) before rebuilding, rather
/// than only clearing the `Arc` slot and leaving an old `Client`'s
/// background task and socket running -- two live clients racing for the
/// same on-disk device store is a real failure mode, not a hypothetical
/// one. `Connecting` is deliberately *not* one of the refused statuses
/// below: it's exactly the state a session sits in for the whole duration
/// of whatsapp-rust's internal reconnect backoff after a drop, and an
/// operator needs a way to force a rebuild out of it instead of waiting
/// out however many backoff cycles remain.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/connect",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body(content = ConnectRequest, description = "Optional device override (first-pair only)"),
    responses(
        (status = 200, description = "Connection initiated", body = SuccessResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Already connected")
    )
)]
pub async fn connect_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: Option<Json<ConnectRequest>>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let device_override = body.and_then(|Json(req)| req.device).map(|d| {
        crate::device_props::resolve_with_override(
            d.os.as_deref(),
            d.platform.as_deref(),
            d.version.as_deref(),
        )
    });
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        if runtime.is_alive() {
            return Err(ApiError::AlreadyConnected);
        }
        let s = runtime.get_status();
        if matches!(
            s,
            SessionStatus::WaitingForQr | SessionStatus::WaitingForPairCode
        ) {
            return Err(ApiError::AlreadyConnected);
        }
        if let Some(old_client) = runtime.get_client() {
            old_client.disconnect().await;
        }
        runtime.set_client(None);
        runtime.set_status(SessionStatus::Disconnected);
        runtime.clear_reconnecting();
        runtime.clear_lock_cooldown();
    }

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    let runtime = state.get_or_create_session(&session_id, &storage_path);
    runtime.set_status(SessionStatus::Connecting);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    let dp_override = device_override.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone, dp_override).await {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            let msg = e.to_string();
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
                runtime.set_client(None);
                runtime.update_pair_state(|ps| ps.last_error = Some(msg));
            }
        }
    });

    Ok(Json(SuccessResponse::with_message("Connection initiated")))
}

#[derive(Debug, serde::Deserialize)]
pub struct ConnectWaitQuery {
    /// Session display name, only applied if the session doesn't exist yet.
    pub name: Option<String>,
    /// Device OS override, only applied if the session doesn't exist yet.
    pub os: Option<String>,
    /// Device platform override, only applied if the session doesn't exist yet.
    pub platform: Option<String>,
    /// Device version override, only applied if the session doesn't exist yet.
    pub version: Option<String>,
    /// Give up and close the stream after this many seconds with no
    /// terminal event. Clamped to 5-600, default 180.
    pub timeout_seconds: Option<u64>,
}

/// Creates the session if it doesn't exist, connects it if it isn't already
/// connecting/connected, and streams the pairing flow end-to-end as
/// server-sent events so a caller doesn't have to orchestrate
/// `POST /sessions` + poll `/qr` + poll `/status` + watch webhooks by hand.
///
/// Emitted event names (each `data:` is the same JSON payload shape as the
/// matching webhook, or a small inline object for the synthetic ones):
/// - `qr_code` / `pair_code` -- forwarded verbatim as WhatsApp rotates them.
/// - `connected` -- forwarded verbatim once paired.
/// - `ready` -- synthetic terminal event. Fires immediately if the session
///   was already logged in when the stream opened; otherwise fires after
///   `connected` if history sync is disabled for this session
///   (`skip_history_sync`), or after the upstream `offline_sync_completed`
///   event otherwise.
/// - `error` -- synthetic terminal event on `logged_out`.
/// - `timeout` -- synthetic terminal event if no terminal state is reached
///   before `timeout_seconds`.
///
/// The stream closes itself after any terminal event.
#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/connect/wait",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID (created if it doesn't already exist)"),
        ("name" = Option<String>, Query, description = "Session display name, only applied on first creation"),
        ("os" = Option<String>, Query, description = "Device OS override, only applied on first creation"),
        ("platform" = Option<String>, Query, description = "Device platform override, only applied on first creation"),
        ("version" = Option<String>, Query, description = "Device version override, only applied on first creation"),
        ("timeout_seconds" = Option<u64>, Query, description = "Give up after this many seconds with no terminal event (5-600, default 180)")
    ),
    responses(
        (status = 200, description = "text/event-stream of the pairing flow: qr_code/pair_code -> connected -> ready, or error/timeout")
    )
)]
pub async fn connect_and_wait(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<ConnectWaitQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>>, ApiError> {
    let timeout = Duration::from_secs(query.timeout_seconds.unwrap_or(180).clamp(5, 600));
    let runtime = ensure_connecting_for_wait(&state, &session_id, &query).await?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(32);

    if runtime.is_alive() && runtime.get_status() == SessionStatus::LoggedIn {
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(SseEvent::default()
                    .event("ready")
                    .data(r#"{"already_connected":true}"#)))
                .await;
        });
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        return Ok(Sse::new(stream).keep_alive(KeepAlive::default()));
    }

    let mut events_rx = runtime.subscribe_events();
    let skip_sync = runtime.skip_history_sync();

    tokio::spawn(async move {
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    let _ = tx
                        .send(Ok(SseEvent::default().event("timeout").data("{}")))
                        .await;
                    return;
                }
                recv = events_rx.recv() => {
                    let payload = match recv {
                        Ok(p) => p,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    };
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) else {
                        continue;
                    };
                    let event_name = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
                    match event_name {
                        #[allow(clippy::collapsible_match)]
                        "qr_code" | "pair_code" => {
                            if tx
                                .send(Ok(SseEvent::default().event(event_name).data(payload)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        "connected" => {
                            if tx
                                .send(Ok(SseEvent::default().event("connected").data(payload.clone())))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            if skip_sync {
                                let _ = tx
                                    .send(Ok(SseEvent::default().event("ready").data(payload)))
                                    .await;
                                return;
                            }
                        }
                        "offline_sync_completed" => {
                            let _ = tx
                                .send(Ok(SseEvent::default().event("ready").data(payload)))
                                .await;
                            return;
                        }
                        "logged_out" => {
                            let _ = tx
                                .send(Ok(SseEvent::default().event("error").data(payload)))
                                .await;
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// Idempotent connect trigger shared by [`connect_and_wait`]: creates the
/// session row if it doesn't exist yet, then kicks off a connect unless one
/// is already in flight or the session is already logged in (in which case
/// the caller short-circuits without touching the client at all).
async fn ensure_connecting_for_wait(
    state: &AppState,
    session_id: &str,
    query: &ConnectWaitQuery,
) -> Result<std::sync::Arc<crate::state::SessionState>, ApiError> {
    let existing = state
        .session_manager()
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let storage_path = if existing.is_some() {
        state
            .session_manager()
            .get_storage_path(session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id))
    } else {
        let storage_path = format!("{}/{}", state.base_storage_path(), session_id);
        tokio::fs::create_dir_all(&storage_path)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        state
            .session_manager()
            .create_session(session_id, query.name.as_deref(), &storage_path)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        storage_path
    };

    let runtime = state.get_or_create_session(session_id, &storage_path);

    if runtime.is_alive() && runtime.get_status() == SessionStatus::LoggedIn {
        return Ok(runtime);
    }

    let s = runtime.get_status();
    if matches!(
        s,
        SessionStatus::Connecting | SessionStatus::WaitingForQr | SessionStatus::WaitingForPairCode
    ) {
        return Ok(runtime);
    }

    if let Some(old_client) = runtime.get_client() {
        old_client.disconnect().await;
    }
    runtime.set_client(None);
    runtime.set_status(SessionStatus::Connecting);
    runtime.clear_reconnecting();
    runtime.clear_lock_cooldown();

    let device_override =
        if query.os.is_some() || query.platform.is_some() || query.version.is_some() {
            Some(crate::device_props::resolve_with_override(
                query.os.as_deref(),
                query.platform.as_deref(),
                query.version.as_deref(),
            ))
        } else {
            None
        };

    let state_clone = state.clone();
    let session_id_clone = session_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone, device_override).await {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            let msg = e.to_string();
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
                runtime.set_client(None);
                runtime.update_pair_state(|ps| ps.last_error = Some(msg));
            }
        }
    });

    Ok(runtime)
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/pair",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = PairCodeRequest,
    responses(
        (status = 200, description = "Pair code generated", body = PairCodeResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Already connected")
    )
)]
pub async fn pair_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<PairCodeRequest>,
) -> Result<Json<PairCodeResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        let status = runtime.get_status();
        if status == SessionStatus::LoggedIn {
            return Err(ApiError::AlreadyConnected);
        }
    }

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    let runtime = state.get_or_create_session(&session_id, &storage_path);

    use whatsapp_rust::pair_code::PairCodeOptions;
    let opts_for_client = PairCodeOptions {
        phone_number: request.phone_number.clone(),
        show_push_notification: request.show_push_notification,
        custom_code: None,
        platform_id: None,
        display_os: None,
    };

    if let Some(client) = runtime.get_client() {
        let code = client
            .pair_with_code(opts_for_client)
            .await
            .map_err(|e| ApiError::Internal(format!("pair_with_code failed: {e}")))?;
        runtime.set_pair_code(Some(code.clone()));
        return Ok(Json(PairCodeResponse {
            code,
            timeout_seconds: 180,
        }));
    }

    let existing_status = runtime.get_status();
    let spawn_needed = !matches!(
        existing_status,
        SessionStatus::WaitingForPairCode | SessionStatus::Connecting
    );

    if spawn_needed {
        runtime.set_status(SessionStatus::WaitingForPairCode);

        let state_clone = state.clone();
        let session_id_clone = session_id.clone();
        let phone_number = request.phone_number.clone();
        let show_notification = request.show_push_notification;
        let device_override = request.device.as_ref().map(|d| {
            crate::device_props::resolve_with_override(
                d.os.as_deref(),
                d.platform.as_deref(),
                d.version.as_deref(),
            )
        });

        tokio::spawn(async move {
            if let Err(e) = connect_client_with_pair_code(
                &state_clone,
                &session_id_clone,
                &phone_number,
                show_notification,
                device_override,
            )
            .await
            {
                tracing::error!(
                    "Session {} pair code connection failed: {}",
                    session_id_clone,
                    e
                );
                if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                    runtime.set_status(SessionStatus::Disconnected);
                }
            }
        });
    }

    let mut pair_code = String::new();
    for _ in 0..80 {
        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
        if let Some(c) = state
            .get_session(&session_id)
            .and_then(|r| r.get_pair_code())
        {
            if !c.is_empty() {
                pair_code = c;
                break;
            }
        }
    }

    if pair_code.is_empty() {
        if let Some(client) = state.get_session(&session_id).and_then(|r| r.get_client()) {
            let opts = PairCodeOptions {
                phone_number: request.phone_number.clone(),
                show_push_notification: request.show_push_notification,
                custom_code: None,
                platform_id: None,
                display_os: None,
            };
            if let Ok(code) = client.pair_with_code(opts).await {
                if let Some(runtime) = state.get_session(&session_id) {
                    runtime.set_pair_code(Some(code.clone()));
                }
                pair_code = code;
            }
        }
    }

    Ok(Json(PairCodeResponse {
        code: pair_code,
        timeout_seconds: 180,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/disconnect",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Disconnected", body = SuccessResponse),
        (status = 404, description = "Session not found"),
        (status = 503, description = "Not connected")
    )
)]
pub async fn disconnect_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
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

    client.disconnect().await;
    runtime.set_status(SessionStatus::Disconnected);
    runtime.set_client(None);
    runtime.clear_reconnecting();

    let _ = state
        .session_manager()
        .update_session_status(&session_id, SessionStatus::Disconnected, false)
        .await;

    Ok(Json(SuccessResponse::with_message("Disconnected")))
}

/// Package a session's local storage directory (device identity, Signal
/// protocol keys, noise handshake state — everything `whatsapp-rust`
/// itself persists) as a zip, so it can be moved to another waxum
/// instance. Disconnects the session first: the same device credentials
/// must never be live on two instances at once, so export always leaves
/// the source side stopped.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/export",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Zip archive of the session's local storage directory"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn export_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<AxumResponse, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        if let Some(client) = runtime.get_client() {
            client.disconnect().await;
        }
        runtime.set_status(SessionStatus::Disconnected);
        runtime.set_client(None);
        runtime.clear_reconnecting();
    }
    let _ = state
        .session_manager()
        .update_session_status(&session_id, SessionStatus::Disconnected, false)
        .await;

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    let sid = session_id.clone();
    let zip_bytes = tokio::task::spawn_blocking(move || zip_directory(&storage_path))
        .await
        .map_err(|e| ApiError::Internal(format!("export task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(format!("export failed: {e}")))?;

    AxumResponse::builder()
        .status(200)
        .header("content-type", "application/zip")
        .header(
            "content-disposition",
            format!("attachment; filename=\"{sid}.waxum-session.zip\""),
        )
        .body(axum::body::Body::from(zip_bytes))
        .map_err(|e| ApiError::Internal(format!("response build: {e}")))
}

/// Restore a session's local storage directory from an [`export_session`]
/// zip, e.g. after copying it to a different waxum instance. Refuses to
/// run over a session that is currently connected on this instance —
/// disconnect it first (or export it here, which does that
/// automatically). Does not reconnect automatically; call
/// `POST /sessions/{id}/connect` afterwards.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/import",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID — must already exist (create it first if needed)")
    ),
    responses(
        (status = 200, description = "Storage restored", body = SuccessResponse),
        (status = 400, description = "Invalid zip upload"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session is currently connected on this instance")
    )
)]
pub async fn import_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<SuccessResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        if runtime.is_alive() {
            return Err(ApiError::AlreadyConnected);
        }
    }

    let mut zip_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            zip_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?
                    .to_vec(),
            );
        }
    }
    let zip_bytes = zip_bytes.ok_or_else(|| ApiError::BadRequest("No file provided".into()))?;

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    tokio::task::spawn_blocking(move || unzip_directory(&storage_path, &zip_bytes))
        .await
        .map_err(|e| ApiError::Internal(format!("import task panicked: {e}")))?
        .map_err(|e| ApiError::BadRequest(format!("import failed: {e}")))?;

    Ok(Json(SuccessResponse::with_message(
        "Session storage imported — call /connect to bring it online",
    )))
}

/// Recursively zip a directory's contents, entry paths relative to
/// `dir`. Blocking (file I/O + deflate); run inside `spawn_blocking`.
fn zip_directory(dir: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options = zip::write::SimpleFileOptions::default();

    fn add_dir(
        writer: &mut zip::ZipWriter<&mut std::io::Cursor<Vec<u8>>>,
        options: zip::write::SimpleFileOptions,
        base: &std::path::Path,
        dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base)?.to_string_lossy().to_string();
            if path.is_dir() {
                add_dir(writer, options, base, &path)?;
            } else {
                writer.start_file(rel, options)?;
                std::io::Write::write_all(writer, &std::fs::read(&path)?)?;
            }
        }
        Ok(())
    }

    let base = std::path::Path::new(dir);
    if base.is_dir() {
        add_dir(&mut writer, options, base, base)?;
    }
    writer.finish()?;
    Ok(buf.into_inner())
}

/// Caps for [`unzip_directory`]: an import is caller-supplied and untrusted,
/// so a crafted archive with a tiny compressed size but a huge (or
/// unbounded) decompressed size -- a zip bomb -- must not be able to
/// exhaust disk. `MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES` is enforced twice:
/// once from the entry's declared size (fast rejection for the well-formed
/// case) and once as a hard `Read::take` cap on the actual bytes copied,
/// since the declared size in a zip's local/central header is
/// attacker-controlled metadata, not a guarantee.
const MAX_IMPORT_ENTRIES: usize = 20_000;
const MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Unzip into `dir`, creating it if needed. Rejects entries whose path
/// would escape `dir` (zip-slip) instead of writing them, and enforces the
/// zip-bomb caps above. Blocking; run inside `spawn_blocking`.
fn unzip_directory(dir: &str, zip_bytes: &[u8]) -> anyhow::Result<()> {
    let base = std::path::Path::new(dir);
    std::fs::create_dir_all(base)?;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    if archive.len() > MAX_IMPORT_ENTRIES {
        anyhow::bail!(
            "zip has {} entries, exceeding the {} entry limit",
            archive.len(),
            MAX_IMPORT_ENTRIES
        );
    }

    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel) = file.enclosed_name() else {
            anyhow::bail!(
                "zip entry {:?} has an unsafe path, refusing to extract",
                file.name()
            );
        };
        if file.is_dir() {
            continue;
        }

        if file.size() > MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip entry {:?} declares {} uncompressed bytes, exceeding the per-file limit",
                rel,
                file.size()
            );
        }
        total_uncompressed = total_uncompressed.saturating_add(file.size());
        if total_uncompressed > MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip's total declared uncompressed size exceeds the {} byte limit",
                MAX_IMPORT_TOTAL_UNCOMPRESSED_BYTES
            );
        }

        let out_path = base.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        let mut limited = std::io::Read::take(&mut file, MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut out)?;
        if copied > MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES {
            anyhow::bail!(
                "zip entry {:?} exceeded the per-file uncompressed size limit while extracting",
                rel
            );
        }
    }
    Ok(())
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

/// On engine boot, walk every previously-paired session and start a
/// reconnect attempt in the background. Sessions that have no stored
/// credentials (never paired or freshly logged out) are skipped — those
/// stay disconnected until the user re-pairs from the dashboard.
pub async fn reconnect_all_on_startup(state: AppState) {
    let sessions = match state.session_manager().list_sessions().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("[startup] list_sessions failed: {}", e);
            return;
        }
    };

    let stagger = crate::preflight::session_startup_stagger();
    tracing::info!(
        stagger_ms = stagger.as_millis() as u64,
        "[startup] auto-reconnect: found {} sessions in DB",
        sessions.len()
    );

    for session in sessions {
        let should_reconnect = matches!(
            session.status,
            SessionStatus::LoggedIn | SessionStatus::Connected | SessionStatus::Connecting
        ) || session.is_logged_in;
        if !should_reconnect {
            tracing::debug!(
                "[startup] skip session {} (status={:?})",
                session.id,
                session.status
            );
            continue;
        }

        let storage_path = match state.session_manager().get_storage_path(&session.id).await {
            Ok(Some(p)) => p,
            _ => {
                tracing::warn!("[startup] no storage path for session {}", session.id);
                continue;
            }
        };

        let runtime = state.get_or_create_session(&session.id, &storage_path);
        runtime.set_status(SessionStatus::Connecting);

        match state.session_manager().get_webhooks(&session.id).await {
            Ok(rows) => {
                if rows.is_empty() {
                    tracing::debug!("[startup] no webhooks for session {}", session.id);
                } else {
                    for (webhook_id, config) in rows {
                        state.register_webhook(&session.id, &webhook_id, config);
                    }
                    tracing::info!("[startup] reloaded webhooks for session {}", session.id);
                }
            }
            Err(e) => {
                tracing::warn!("[startup] get_webhooks failed for {}: {}", session.id, e);
            }
        }

        let state_clone = state.clone();
        let sid = session.id.clone();
        tokio::spawn(async move {
            tracing::info!("[startup] reconnecting session {}", sid);
            if let Err(e) = connect_client(&state_clone, &sid, None).await {
                tracing::warn!("[startup] reconnect failed for {}: {}", sid, e);
                if let Some(runtime) = state_clone.get_session(&sid) {
                    runtime.set_status(SessionStatus::Disconnected);
                }
            }
        });

        tokio::time::sleep(stagger).await;
    }
}

/// Self-heal for a session wedged in whatsapp-rust's own reconnect backoff.
///
/// whatsapp-rust's internal retry loop (inside `bot.run()`) has no attempt
/// cap of its own -- it backs off up to 15 minutes between tries and keeps
/// going forever, which in practice reads as "auto-reconnect enabled but
/// stuck" during a prolonged outage or a wedged handshake. This watchdog
/// ticks every `RECONNECT_WATCHDOG_POLL_MS` (default 30s) and, for any
/// session that has been in `Connecting` for longer than
/// `RECONNECT_MAX_STUCK_SECS` (default 600s) *or* whose
/// `client.stats().reconnect_errors` has crossed `RECONNECT_MAX_ATTEMPTS`
/// (default 10), forces the same full rebuild a manual `POST .../connect`
/// performs: cleanly disconnect the wedged client (stopping its internal
/// loop) and spawn [`connect_client`] fresh, rather than trusting the
/// crate to eventually recover on its own. Also broadcasts a synthetic
/// `disconnected` webhook/event as a safety net -- see the module docs on
/// `Event::Disconnected` for why the crate doesn't always dispatch one for
/// a socket that dies silently. Sessions paused by an account-lock
/// cooldown (`ACCOUNT_LOCK_BACKOFF_SECS`) are skipped outright.
pub async fn run_reconnect_watchdog(state: AppState) {
    let poll_ms: u64 = std::env::var("RECONNECT_WATCHDOG_POLL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30_000);
    let max_attempts: u32 = std::env::var("RECONNECT_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let max_stuck_secs: i64 = std::env::var("RECONNECT_MAX_STUCK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tracing::info!(
        poll_ms,
        max_attempts,
        max_stuck_secs,
        "reconnect watchdog started"
    );

    loop {
        ticker.tick().await;
        for (session_id, runtime) in state.session_iter_with_ids() {
            if runtime.get_status() != SessionStatus::Connecting {
                continue;
            }
            if let Some(remaining) = runtime.lock_cooldown_remaining() {
                tracing::debug!(
                    session_id = %session_id,
                    cooldown_remaining_secs = remaining,
                    "reconnect watchdog: skipping session in account-lock cooldown"
                );
                continue;
            }
            let Some(client) = runtime.get_client() else {
                continue;
            };
            let stuck_secs = runtime.reconnecting_for_secs().unwrap_or(0);
            let reconnect_errors = client.stats().reconnect_errors;
            if reconnect_errors < max_attempts && stuck_secs < max_stuck_secs {
                continue;
            }

            tracing::warn!(
                session_id = %session_id,
                reconnect_errors,
                stuck_secs,
                "reconnect watchdog: forcing full rebuild after a stuck retry window"
            );

            client.disconnect().await;
            runtime.set_client(None);
            runtime.set_status(SessionStatus::Disconnected);
            runtime.clear_reconnecting();
            let _ = state
                .session_manager()
                .update_session_status(&session_id, SessionStatus::Disconnected, false)
                .await;

            let payload = serde_json::json!({
                "session_id": session_id,
                "event": "disconnected",
                "timestamp": chrono::Utc::now().timestamp(),
                "data": { "forced_by": "reconnect_watchdog" },
            });
            if let Ok(payload_str) = serde_json::to_string(&payload) {
                state
                    .broadcast_to_webhooks(&session_id, "disconnected", &payload_str)
                    .await;
                state
                    .publish_to_nats(&session_id, "disconnected", &payload_str)
                    .await;
                runtime.broadcast_event(payload_str);
            }

            runtime.set_status(SessionStatus::Connecting);
            let state_clone = state.clone();
            let session_id_clone = session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = connect_client(&state_clone, &session_id_clone, None).await {
                    tracing::error!(
                        "reconnect watchdog: rebuild failed for {}: {}",
                        session_id_clone,
                        e
                    );
                    if let Some(rt) = state_clone.get_session(&session_id_clone) {
                        rt.set_status(SessionStatus::Disconnected);
                        rt.set_client(None);
                    }
                }
            });
        }
    }
}

async fn connect_client(
    state: &AppState,
    session_id: &str,
    device_props: Option<ResolvedDeviceProps>,
) -> Result<(), ApiError> {
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::TokioRuntime;
    use whatsapp_rust_sqlite_storage::SqliteStore;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;

    let storage_path = state
        .session_manager()
        .get_storage_path(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;

    let db_path = format!("{}/whatsapp.db", storage_path);

    let backend = SqliteStore::new(&db_path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let chat_store_backend = backend.clone();

    let transport_factory = TokioWebSocketTransportFactory::new();
    let http_client = crate::net::build_http_client();

    let state_for_events = state.clone();
    let session_id_for_events = session_id.to_string();

    let dp = device_props.unwrap_or_else(crate::device_props::resolve_from_env);

    let bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .with_runtime(TokioRuntime)
        .with_device_props({
            let mut o = wacore::store::DevicePropsOverride::new()
                .with_os(dp.os)
                .with_platform_type(dp.platform);
            if let Some(v) = dp.version {
                o = o.with_version(v);
            }
            o
        })
        .on_event(move |event, client| {
            let state = state_for_events.clone();
            let session_id = session_id_for_events.clone();
            async move {
                handle_event(event, &state, &session_id, client).await;
            }
        })
        .build()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Some(runtime) = state.get_session(session_id) {
        let c = bot.client();
        c.enable_auto_reconnect.store(
            runtime.auto_reconnect_enabled(),
            std::sync::atomic::Ordering::Relaxed,
        );
        c.set_skip_history_sync(runtime.skip_history_sync());
        match whatsapp_rust_chat_store::ChatStore::new(&chat_store_backend).await {
            Ok(store) => {
                let subscription = c.subscribe_handler(store.handler());
                runtime.set_chat_store(store, subscription);
            }
            Err(e) => {
                tracing::warn!("chat store unavailable for session {}: {}", session_id, e);
            }
        }
        runtime.set_enc_decrypt_failed_lease(c.acquire_enc_decrypt_failed_forwarding());
        runtime.set_client(Some(c));
        runtime.set_status(SessionStatus::WaitingForQr);
    }

    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::WaitingForQr, false)
        .await;

    bot.run().await;

    if let Some(runtime) = state.get_session(session_id) {
        runtime.set_status(SessionStatus::Disconnected);
        runtime.set_client(None);
    }

    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::Disconnected, false)
        .await;

    Ok(())
}

async fn connect_client_with_pair_code(
    state: &AppState,
    session_id: &str,
    phone_number: &str,
    show_notification: bool,
    device_props: Option<ResolvedDeviceProps>,
) -> Result<(), ApiError> {
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::pair_code::PairCodeOptions;
    use whatsapp_rust::TokioRuntime;
    use whatsapp_rust_sqlite_storage::SqliteStore;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;

    let storage_path = state
        .session_manager()
        .get_storage_path(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.to_string()))?;

    let db_path = format!("{}/whatsapp.db", storage_path);

    let backend = SqliteStore::new(&db_path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let chat_store_backend = backend.clone();

    let transport_factory = TokioWebSocketTransportFactory::new();
    let http_client = crate::net::build_http_client();

    let state_for_events = state.clone();
    let session_id_for_events = session_id.to_string();

    let dp = device_props.unwrap_or_else(crate::device_props::resolve_from_env);
    let pair_options = PairCodeOptions {
        phone_number: phone_number.to_string(),
        show_push_notification: show_notification,
        custom_code: None,
        platform_id: None,
        display_os: None,
    };

    let bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .with_runtime(TokioRuntime)
        .with_pair_code(pair_options)
        .with_device_props({
            let mut o = wacore::store::DevicePropsOverride::new()
                .with_os(dp.os.clone())
                .with_platform_type(dp.platform);
            if let Some(v) = dp.version {
                o = o.with_version(v);
            }
            o
        })
        .on_event(move |event, client| {
            let state = state_for_events.clone();
            let session_id = session_id_for_events.clone();
            async move {
                handle_event(event, &state, &session_id, client).await;
            }
        })
        .build()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Some(runtime) = state.get_session(session_id) {
        let c = bot.client();
        c.enable_auto_reconnect.store(
            runtime.auto_reconnect_enabled(),
            std::sync::atomic::Ordering::Relaxed,
        );
        c.set_skip_history_sync(runtime.skip_history_sync());
        match whatsapp_rust_chat_store::ChatStore::new(&chat_store_backend).await {
            Ok(store) => {
                let subscription = c.subscribe_handler(store.handler());
                runtime.set_chat_store(store, subscription);
            }
            Err(e) => {
                tracing::warn!("chat store unavailable for session {}: {}", session_id, e);
            }
        }
        runtime.set_enc_decrypt_failed_lease(c.acquire_enc_decrypt_failed_forwarding());
        runtime.set_client(Some(c));
    }

    bot.run().await;

    if let Some(runtime) = state.get_session(session_id) {
        runtime.set_status(SessionStatus::Disconnected);
        runtime.set_client(None);
    }

    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::Disconnected, false)
        .await;

    Ok(())
}

/// Whether incoming calls are auto-rejected instead of registered for
/// manual `POST .../calls/reject`. Read once from
/// `AUTO_REJECT_INCOMING_CALLS` (default false) and cached — see #114.
fn auto_reject_incoming_calls() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("AUTO_REJECT_INCOMING_CALLS")
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    })
}

async fn handle_event(
    event: std::sync::Arc<wacore::types::events::Event>,
    state: &AppState,
    session_id: &str,
    client: std::sync::Arc<whatsapp_rust::Client>,
) {
    use wacore::types::events::Event;

    let runtime = match state.get_session(session_id) {
        Some(r) => r,
        None => return,
    };

    match event.as_ref() {
        Event::PairingQrCode(wacore::types::events::PairingQrCode { code, .. }) => {
            tracing::info!("Session {}: QR code received", session_id);
            runtime.set_qr_codes(vec![code.clone()]);
            runtime.set_status(SessionStatus::WaitingForQr);
            let now = chrono::Utc::now().timestamp();
            runtime.update_pair_state(|ps| {
                ps.last_qr_at = Some(now);
                ps.attempts = ps.attempts.saturating_add(1);
                ps.last_error = None;
            });
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::WaitingForQr, false)
                .await;
        }
        Event::PairingCode(wacore::types::events::PairingCode { code, timeout, .. }) => {
            tracing::info!("Session {}: Pair code received: {}", session_id, code);
            runtime.set_pair_code(Some(code.clone()));
            let now = chrono::Utc::now().timestamp();
            let expires_at = now + timeout.as_secs() as i64;
            runtime.update_pair_state(|ps| {
                ps.last_pair_code_at = Some(now);
                ps.pair_code_expires_at = Some(expires_at);
                ps.last_error = None;
            });
        }
        Event::Connected(_) => {
            tracing::info!("Session {}: Connected", session_id);
            if runtime.reconnecting_for_secs().is_some() {
                crate::metrics::record_session_reconnect(session_id);
            }
            runtime.set_status(SessionStatus::LoggedIn);
            runtime.set_qr_codes(vec![]);
            runtime.set_pair_code(None);
            runtime.clear_pair_state();
            runtime.clear_reconnecting();

            let push_name_str = client.push_name();
            let push_name = if push_name_str.is_empty() {
                None
            } else {
                Some(push_name_str)
            };
            let phone = client.pn().map(|j| j.user.clone());

            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::LoggedIn, true)
                .await;
            let _ = state
                .session_manager()
                .update_session_info(session_id, phone.as_deref(), push_name.as_deref())
                .await;
            let _ = state
                .session_manager()
                .update_last_connected(session_id)
                .await;
        }
        Event::Disconnected(d) => {
            let reason = format!("{}", d.reason);
            let clean = d.reason.is_clean_shutdown();
            if clean {
                tracing::info!(
                    session_id = %session_id,
                    reason = %reason,
                    "socket dropped (clean recycle) — auto-reconnect in flight"
                );
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    reason = %reason,
                    "socket dropped (unexpected) — auto-reconnect in flight"
                );
            }
            runtime.set_status(SessionStatus::Connecting);
            runtime.mark_reconnecting_now();
            crate::metrics::record_session_drop(session_id);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Connecting, false)
                .await;
        }
        Event::LoggedOut(logged_out) => {
            let is_lock = matches!(
                logged_out.reason,
                wacore::types::events::ConnectFailureReason::AccountLocked
            );
            if let Some(client) = runtime.get_client() {
                if is_lock {
                    client
                        .enable_auto_reconnect
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                }
                client.disconnect().await;
            }
            runtime.set_status(SessionStatus::Disconnected);
            runtime.set_client(None);
            runtime.clear_reconnecting();
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Disconnected, false)
                .await;

            if is_lock {
                let (cooldown_secs, cooldown_until) =
                    runtime.record_lock_cooldown(&state.account_lock_backoff().schedule);
                tracing::warn!(
                    session_id = %session_id,
                    reason = ?logged_out.reason,
                    cooldown_secs,
                    "account_locked, cooling down — auto-reconnect paused until manual reconnect"
                );
                let payload = serde_json::json!({
                    "session_id": session_id,
                    "event": "account_locked",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "data": {
                        "reason": format!("{:?}", logged_out.reason),
                        "cooldown_secs": cooldown_secs,
                        "cooldown_until": cooldown_until,
                    },
                });
                if let Ok(payload_str) = serde_json::to_string(&payload) {
                    state
                        .broadcast_to_webhooks(session_id, "account_locked", &payload_str)
                        .await;
                    state
                        .publish_to_nats(session_id, "account_locked", &payload_str)
                        .await;
                    runtime.broadcast_event(payload_str);
                }
                return;
            }

            let should_purge = runtime.record_logout_and_should_purge();
            if !should_purge {
                tracing::warn!(
                    "Session {}: Logged out: {:?} — keeping storage (transient flap)",
                    session_id,
                    logged_out.reason
                );
                return;
            }

            tracing::warn!(
                "Session {}: Logged out: {:?} — purging after repeated flaps",
                session_id,
                logged_out.reason
            );
            let storage_path = state
                .session_manager()
                .get_storage_path(session_id)
                .await
                .ok()
                .flatten();
            state.remove_session(session_id);
            if let Some(path) = storage_path {
                let _ = tokio::fs::remove_dir_all(&path).await;
            }
            if let Err(e) = state.session_manager().delete_session(session_id).await {
                tracing::warn!(
                    "Session {}: failed to purge after logout: {}",
                    session_id,
                    e
                );
            }
        }
        Event::IncomingCall(call) => {
            let call_id = call.action.call_id().to_string();
            if auto_reject_incoming_calls() {
                if let Err(e) = client.voip().reject(call).await {
                    tracing::warn!(
                        session_id = %session_id,
                        call_id = %call_id,
                        "AUTO_REJECT_INCOMING_CALLS: failed to reject: {}",
                        e
                    );
                }
            } else if !call_id.is_empty() {
                state.incoming_calls().insert(call_id, *call.clone());
            }
        }
        _ => {}
    }

    persist_contact_event(state, session_id, event.as_ref()).await;

    if let Event::Messages(batch) = event.as_ref() {
        let timestamp = chrono::Utc::now().timestamp();
        let offline = matches!(
            batch.origin,
            wacore::types::events::BatchOrigin::OfflineDrain
        );
        if offline {
            tracing::info!(
                session_id = %session_id,
                count = batch.messages.len(),
                "offline sync: replaying messages received while disconnected"
            );
        }
        for im in batch.messages.iter() {
            crate::handlers::search::record_incoming(state, session_id, &im.message, &im.info)
                .await;
            let from_phone = resolve_sender_phone(&client, &im.info.source.sender).await;
            let data = message_event_data(&im.message, &im.info, from_phone);
            let payload_value = serde_json::json!({
                "session_id": session_id,
                "event": "message",
                "timestamp": timestamp,
                "offline": offline,
                "data": data,
            });
            if let Ok(payload) = serde_json::to_string(&payload_value) {
                state
                    .broadcast_to_webhooks(session_id, "message", &payload)
                    .await;
                state.publish_to_nats(session_id, "message", &payload).await;
                runtime.broadcast_event(payload);
            }
        }
    } else if let Ok(payload) = serde_json::to_string(&event_to_json(event.as_ref(), session_id)) {
        let event_type = get_event_type(event.as_ref());
        state
            .broadcast_to_webhooks(session_id, &event_type, &payload)
            .await;
        state
            .publish_to_nats(session_id, &event_type, &payload)
            .await;
        runtime.broadcast_event(payload);
    }
}

/// `wacore::types::events::Event` is `#[non_exhaustive]`, so the wildcard
/// arm below can never be dropped even once every current variant is named
/// -- the compiler cannot flag a newly added upstream variant here.
/// Bumping the whatsapp-rust pin should include re-diffing this match
/// against the new `Event` definition.
fn get_event_type(event: &wacore::types::events::Event) -> String {
    use wacore::types::events::Event;
    match event {
        Event::PairingQrCode { .. } => "qr_code".to_string(),
        Event::PairingCode { .. } => "pair_code".to_string(),
        Event::Connected(_) => "connected".to_string(),
        Event::Disconnected(_) => "disconnected".to_string(),
        Event::LoggedOut(_) => "logged_out".to_string(),
        Event::Messages(_) => "message".to_string(),
        Event::Receipt(_) => "receipt".to_string(),
        Event::Presence(_) => "presence".to_string(),
        Event::ChatPresence(_) => "chat_presence".to_string(),
        Event::GroupUpdate(_) => "group_update".to_string(),
        Event::IncomingCall(_) => "incoming_call".to_string(),
        Event::PictureUpdate(_) => "picture_update".to_string(),
        Event::UserAboutUpdate(_) => "user_about_update".to_string(),
        Event::SelfPushNameUpdated(_) => "push_name_update".to_string(),
        Event::ContactUpdate(_) => "contact_update".to_string(),
        Event::DeviceListUpdate(_) => "device_list_update".to_string(),
        Event::PinUpdate(_) => "pin_update".to_string(),
        Event::MuteUpdate(_) => "mute_update".to_string(),
        Event::ArchiveUpdate(_) => "archive_update".to_string(),
        Event::MarkChatAsReadUpdate(_) => "mark_chat_as_read".to_string(),
        Event::UndecryptableMessage(_) => "undecryptable_message".to_string(),
        Event::ClientOutdated(_) => "client_outdated".to_string(),
        Event::OfflineSyncPreview(_) => "offline_sync_preview".to_string(),
        Event::OfflineSyncCompleted(_) => "offline_sync_completed".to_string(),
        Event::CallLogSync(_) => "call_log_sync".to_string(),
        Event::StreamError(_) => "stream_error".to_string(),
        Event::EncDecryptFailed(_) => "enc_decrypt_failed".to_string(),
        Event::AppStateSyncFailed(_) => "app_state_sync_failed".to_string(),
        Event::BusinessStatusUpdate(_) => "business_status_update".to_string(),
        Event::CallEndedElsewhere(_) => "call_ended_elsewhere".to_string(),
        Event::ClearChatUpdate(_) => "clear_chat_update".to_string(),
        Event::ClientExpirationChanged(_) => "client_expiration_changed".to_string(),
        Event::ConnectFailure(_) => "connect_failure".to_string(),
        Event::ContactNumberChanged(_) => "contact_number_changed".to_string(),
        Event::ContactRemoved(_) => "contact_removed".to_string(),
        Event::ContactSyncRequested(_) => "contact_sync_requested".to_string(),
        Event::ContactUpdated(_) => "contact_updated".to_string(),
        Event::DecryptedPayload(_) => "decrypted_payload".to_string(),
        Event::DeleteChatUpdate(_) => "delete_chat_update".to_string(),
        Event::DeleteMessageForMeUpdate(_) => "delete_message_for_me_update".to_string(),
        Event::DirtyState(_) => "dirty_state".to_string(),
        Event::DisableLinkPreviewsUpdate(_) => "disable_link_previews_update".to_string(),
        Event::DisappearingModeChanged(_) => "disappearing_mode_changed".to_string(),
        Event::FavoriteStickerUpdate(_) => "favorite_sticker_update".to_string(),
        Event::FavoritesUpdate(_) => "favorites_update".to_string(),
        Event::HistorySync(_) => "history_sync".to_string(),
        Event::IdentityChange(_) => "identity_change".to_string(),
        Event::LabelAssociationUpdate(_) => "label_association_update".to_string(),
        Event::LabelEditUpdate(_) => "label_edit_update".to_string(),
        Event::LockChatUpdate(_) => "lock_chat_update".to_string(),
        Event::MessageLabelAssociationUpdate(_) => "message_label_association_update".to_string(),
        Event::MexNotification(_) => "mex_notification".to_string(),
        Event::MissedCall(_) => "missed_call".to_string(),
        Event::NewsletterLiveUpdate(_) => "newsletter_live_update".to_string(),
        Event::Notification(_) => "notification".to_string(),
        Event::OfflineSyncInterrupted(_) => "offline_sync_interrupted".to_string(),
        Event::PairError(_) => "pair_error".to_string(),
        Event::PairingCodeError(_) => "pairing_code_error".to_string(),
        Event::PairingCodeRefresh(_) => "pairing_code_refresh".to_string(),
        Event::PairingQrCodesExhausted(_) => "pairing_qr_codes_exhausted".to_string(),
        Event::PairPasskeyConfirmation(_) => "pair_passkey_confirmation".to_string(),
        Event::PairPasskeyError(_) => "pair_passkey_error".to_string(),
        Event::PairPasskeyRequest(_) => "pair_passkey_request".to_string(),
        Event::PairSuccess(_) => "pair_success".to_string(),
        Event::QrScannedWithoutMultidevice(_) => "qr_scanned_without_multidevice".to_string(),
        Event::QuickReplyUpdate(_) => "quick_reply_update".to_string(),
        Event::RawNode(_) => "raw_node".to_string(),
        Event::RemoveRecentStickerUpdate(_) => "remove_recent_sticker_update".to_string(),
        Event::RetiredPushNameUpdate(_) => "retired_push_name_update".to_string(),
        Event::SentFrame(_) => "sent_frame".to_string(),
        Event::ServerAck(_) => "server_ack".to_string(),
        Event::StarUpdate(_) => "star_update".to_string(),
        Event::StreamReplaced(_) => "stream_replaced".to_string(),
        Event::TemporaryBan(_) => "temporary_ban".to_string(),
        Event::UserStatusMuteUpdate(_) => "user_status_mute_update".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Build the metadata blob a downstream consumer needs to call
/// /sessions/:id/media/download for an inbound media message. Returns null
/// for non-media or text-only messages.
fn extract_media_metadata(msg: &waproto::whatsapp::Message) -> serde_json::Value {
    use base64::Engine as _;
    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    if let Some(im) = msg.image_message.as_option() {
        return serde_json::json!({
            "kind": "image",
            "direct_path": im.direct_path,
            "media_key": im.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": im.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": im.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": im.file_length,
            "mimetype": im.mimetype,
            "width": im.width,
            "height": im.height,
        });
    }
    if let Some(vm) = msg.video_message.as_option() {
        return serde_json::json!({
            "kind": "video",
            "direct_path": vm.direct_path,
            "media_key": vm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": vm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": vm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": vm.file_length,
            "mimetype": vm.mimetype,
            "seconds": vm.seconds,
        });
    }
    if let Some(am) = msg.audio_message.as_option() {
        return serde_json::json!({
            "kind": "audio",
            "direct_path": am.direct_path,
            "media_key": am.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": am.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": am.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": am.file_length,
            "mimetype": am.mimetype,
            "seconds": am.seconds,
            "ptt": am.ptt,
        });
    }
    if let Some(dm) = msg.document_message.as_option() {
        return serde_json::json!({
            "kind": "document",
            "direct_path": dm.direct_path,
            "media_key": dm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": dm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": dm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": dm.file_length,
            "mimetype": dm.mimetype,
            "file_name": dm.file_name,
        });
    }
    if let Some(sm) = msg.sticker_message.as_option() {
        return serde_json::json!({
            "kind": "sticker",
            "direct_path": sm.direct_path,
            "media_key": sm.media_key.as_ref().map(|b| b64(b)),
            "file_sha256": sm.file_sha256.as_ref().map(|b| b64(b)),
            "file_enc_sha256": sm.file_enc_sha256.as_ref().map(|b| b64(b)),
            "file_length": sm.file_length,
            "mimetype": sm.mimetype,
        });
    }
    serde_json::Value::Null
}

/// Typed counterpart to [`extract_media_metadata`] for message-history
/// persistence ([`crate::handlers::search::record_incoming`] /
/// `record_outgoing`): the same five media envelopes, but shaped as
/// [`crate::db::messages::MediaPointer`] instead of a webhook JSON blob,
/// and `None` when the message isn't media or is missing a field
/// `/media/download` requires (an incomplete pointer is useless, so a
/// message with one is stored with no media pointer at all rather than
/// a partially-filled one).
pub(crate) fn extract_media_pointer(
    msg: &waproto::whatsapp::Message,
) -> Option<crate::db::messages::MediaPointer> {
    use base64::Engine as _;
    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    macro_rules! pointer {
        ($m:expr, $kind:literal) => {
            Some(crate::db::messages::MediaPointer {
                media_key: b64($m.media_key.as_ref()?),
                file_sha256: b64($m.file_sha256.as_ref()?),
                file_enc_sha256: b64($m.file_enc_sha256.as_ref()?),
                direct_path: $m.direct_path.clone()?,
                file_length: $m.file_length? as i64,
                media_type: $kind.to_string(),
                mimetype: $m.mimetype.clone().unwrap_or_default(),
            })
        };
    }

    if let Some(im) = msg.image_message.as_option() {
        return pointer!(im, "image");
    }
    if let Some(vm) = msg.video_message.as_option() {
        return pointer!(vm, "video");
    }
    if let Some(am) = msg.audio_message.as_option() {
        return pointer!(am, "audio");
    }
    if let Some(dm) = msg.document_message.as_option() {
        return pointer!(dm, "document");
    }
    if let Some(sm) = msg.sticker_message.as_option() {
        return pointer!(sm, "sticker");
    }
    None
}

/// Extracts location data (lat/lng + optional name/address/url) from a
/// LocationMessage / LiveLocationMessage when present. Returns null otherwise.
fn extract_location(msg: &waproto::whatsapp::Message) -> serde_json::Value {
    if let Some(loc) = msg.location_message.as_option() {
        return serde_json::json!({
            "latitude": loc.degrees_latitude,
            "longitude": loc.degrees_longitude,
            "name": loc.name,
            "address": loc.address,
            "url": loc.url,
            "accuracy_meters": loc.accuracy_in_meters,
            "speed_mps": loc.speed_in_mps,
            "is_live": false,
        });
    }
    if let Some(loc) = msg.live_location_message.as_option() {
        return serde_json::json!({
            "latitude": loc.degrees_latitude,
            "longitude": loc.degrees_longitude,
            "accuracy_meters": loc.accuracy_in_meters,
            "speed_mps": loc.speed_in_mps,
            "sequence_number": loc.sequence_number,
            "caption": loc.caption,
            "is_live": true,
        });
    }
    serde_json::Value::Null
}

fn message_event_data(
    msg: &waproto::whatsapp::Message,
    info: &wacore::types::message::MessageInfo,
    from_phone: Option<String>,
) -> serde_json::Value {
    let (text, caption, message_type, media_mimetype) = extract_message_content(msg);
    let media_meta = extract_media_metadata(msg);
    let location = extract_location(msg);
    serde_json::json!({
        "from": info.source.sender.to_string(),
        "from_phone": from_phone,
        "chat": info.source.chat.to_string(),
        "message_id": info.id.to_string(),
        "timestamp": info.timestamp,
        "is_from_me": info.source.is_from_me,
        "push_name": info.push_name,
        "verified_name": info.verified_name.as_ref().map(|c| format!("{:?}", c)),
        "type": info.r#type,
        "media_type": info.media_type,
        "message_type": message_type,
        "text": text,
        "caption": caption,
        "media_mimetype": media_mimetype,
        "media": media_meta,
        "location": location,
        "is_group": info.source.chat.to_string().ends_with("@g.us"),
        "participant": info.source.sender.to_string(),
    })
}

/// Resolves `sender` to its phone number for the `from_phone` webhook field.
///
/// A `@lid` sender is looked up in whatsapp-rust's own LID↔PN mapping cache
/// (cache-aside over its persistent backend — no network round trip), which
/// is populated from several passive sources including the `sender_pn`
/// attribute WhatsApp attaches to incoming messages, so it is usually already
/// warm by the time a message arrives. A plain-phone sender needs no lookup.
async fn resolve_sender_phone(
    client: &whatsapp_rust::Client,
    sender: &wacore_binary::Jid,
) -> Option<String> {
    if sender.is_pn() {
        return Some(sender.user.to_string());
    }
    if !sender.is_lid() {
        return None;
    }
    client
        .get_lid_pn_entry(sender)
        .await
        .ok()
        .flatten()
        .map(|entry| entry.phone_number.to_string())
}

/// Extracts user-visible content from a protobuf Message: best-effort text,
/// optional caption, the high-level type slug, and the media mimetype if any.
/// Shared with the message-history ingestion in
/// [`crate::handlers::search`].
pub(crate) fn extract_message_content(
    msg: &waproto::whatsapp::Message,
) -> (Option<String>, Option<String>, String, Option<String>) {
    let mut text: Option<String> = None;
    let mut caption: Option<String> = None;
    let mut message_type = "unknown".to_string();
    let mut media_mimetype: Option<String> = None;

    if let Some(t) = &msg.conversation {
        if !t.is_empty() {
            text = Some(t.clone());
            message_type = "text".to_string();
        }
    }
    if message_type == "unknown" {
        if let Some(e) = msg.extended_text_message.as_option() {
            text = e.text.clone();
            message_type = "text".to_string();
        } else if let Some(im) = msg.image_message.as_option() {
            caption = im.caption.clone();
            media_mimetype = im.mimetype.clone();
            message_type = "image".to_string();
        } else if let Some(vm) = msg.video_message.as_option() {
            caption = vm.caption.clone();
            media_mimetype = vm.mimetype.clone();
            message_type = "video".to_string();
        } else if let Some(am) = msg.audio_message.as_option() {
            media_mimetype = am.mimetype.clone();
            message_type = if am.ptt.unwrap_or(false) {
                "ptt".to_string()
            } else {
                "audio".to_string()
            };
        } else if let Some(dm) = msg.document_message.as_option() {
            caption = dm.caption.clone();
            text = dm.file_name.clone();
            media_mimetype = dm.mimetype.clone();
            message_type = "document".to_string();
        } else if let Some(sm) = msg.sticker_message.as_option() {
            media_mimetype = sm.mimetype.clone();
            message_type = "sticker".to_string();
        } else if msg.location_message.is_set() || msg.live_location_message.is_set() {
            message_type = "location".to_string();
        } else if msg.contact_message.is_set() {
            message_type = "contact".to_string();
            text = msg
                .contact_message
                .as_option()
                .and_then(|c| c.display_name.clone());
        } else if msg.contacts_array_message.is_set() {
            message_type = "contacts".to_string();
        } else if msg.poll_creation_message.is_set()
            || msg.poll_creation_message_v2.is_set()
            || msg.poll_creation_message_v3.is_set()
        {
            message_type = "poll".to_string();
            text = msg
                .poll_creation_message
                .as_option()
                .and_then(|p| p.name.clone())
                .or_else(|| {
                    msg.poll_creation_message_v2
                        .as_option()
                        .and_then(|p| p.name.clone())
                })
                .or_else(|| {
                    msg.poll_creation_message_v3
                        .as_option()
                        .and_then(|p| p.name.clone())
                });
        } else if msg.poll_update_message.is_set() {
            message_type = "poll_vote".to_string();
        } else if msg.reaction_message.is_set() {
            message_type = "reaction".to_string();
            text = msg
                .reaction_message
                .as_option()
                .and_then(|r| r.text.clone());
        } else if msg.buttons_message.is_set() {
            message_type = "buttons".to_string();
        } else if msg.list_message.is_set() {
            message_type = "list".to_string();
        } else if msg.template_message.is_set() {
            message_type = "template".to_string();
        }
    }

    (text, caption, message_type, media_mimetype)
}

async fn persist_contact_event(
    state: &AppState,
    session_id: &str,
    event: &wacore::types::events::Event,
) {
    use wacore::types::events::Event;
    let store = crate::db::contacts::ContactStore::new(state.session_manager().pool());

    if let Event::Messages(batch) = event {
        for im in batch.messages.iter() {
            let info = &im.info;
            if info.source.is_from_me {
                continue;
            }
            let sender = &info.source.sender;
            let jid_str = sender.to_string();
            let mut push_name = None::<String>;
            if !info.push_name.is_empty() {
                push_name = Some(info.push_name.to_string());
            }
            let mut business_name = None::<String>;
            if let Some(vn) = info.verified_name.as_ref() {
                let s = format!("{:?}", vn);
                if !s.is_empty() && s != "None" {
                    business_name = Some(s);
                }
            }
            let mut phone_str = None::<String>;
            if sender.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(sender.user.to_string());
            }
            let upsert = crate::db::contacts::ContactUpsert {
                session_id,
                jid: &jid_str,
                phone: phone_str.as_deref(),
                push_name: push_name.as_deref(),
                business_name: business_name.as_deref(),
                source: "message",
                ..Default::default()
            };
            if let Err(e) = store.upsert(&upsert).await {
                tracing::warn!(
                    "contacts: upsert failed for {}/{}: {}",
                    session_id,
                    jid_str,
                    e
                );
            }
        }
        return;
    }

    let mut upsert = crate::db::contacts::ContactUpsert {
        session_id,
        ..Default::default()
    };
    let jid_str;
    let mut phone_str = None::<String>;
    let mut lid_str = None::<String>;
    let mut full_name = None::<String>;
    let mut first_name = None::<String>;
    let push_name = None::<String>;
    let business_name = None::<String>;
    let source: &str;

    match event {
        Event::ContactUpdate(u) => {
            jid_str = u.jid.to_string();
            if let Some(name) = u.action.full_name.as_deref() {
                if !name.is_empty() {
                    full_name = Some(name.to_string());
                }
            }
            if let Some(name) = u.action.first_name.as_deref() {
                if !name.is_empty() {
                    first_name = Some(name.to_string());
                }
            }
            if let Some(lid) = u.action.lid_jid.as_deref() {
                if !lid.is_empty() {
                    lid_str = Some(lid.to_string());
                }
            }
            if u.jid.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(u.jid.user.to_string());
            }
            source = if u.from_full_sync {
                "appstate_sync"
            } else {
                "appstate"
            };
        }
        Event::ContactUpdated(u) => {
            jid_str = u.jid.to_string();
            if u.jid.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(u.jid.user.to_string());
            }
            source = "notification";
        }
        _ => return,
    }

    upsert.jid = &jid_str;
    upsert.phone = phone_str.as_deref();
    upsert.lid_jid = lid_str.as_deref();
    upsert.full_name = full_name.as_deref();
    upsert.first_name = first_name.as_deref();
    upsert.push_name = push_name.as_deref();
    upsert.business_name = business_name.as_deref();
    upsert.source = source;

    if let Err(e) = store.upsert(&upsert).await {
        tracing::warn!(
            "contacts: upsert failed for {}/{}: {}",
            session_id,
            jid_str,
            e
        );
    }
}

fn event_to_json(event: &wacore::types::events::Event, session_id: &str) -> serde_json::Value {
    use wacore::types::events::Event;

    let event_type = get_event_type(event);
    let timestamp = chrono::Utc::now().timestamp();

    let data = match event {
        Event::Messages(batch) => match batch.first() {
            Some(im) => message_event_data(&im.message, &im.info, None),
            None => serde_json::json!({}),
        },
        Event::Receipt(receipt) => {
            serde_json::json!({
                "receipt": format!("{:?}", receipt),
            })
        }
        Event::Presence(presence) => {
            serde_json::json!({
                "jid": presence.from.to_string(),
                "available": !presence.unavailable,
                "last_seen": presence.last_seen,
            })
        }
        Event::ChatPresence(presence) => {
            serde_json::json!({
                "chat": presence.source.chat.to_string(),
                "sender": presence.source.sender.to_string(),
                "state": format!("{:?}", presence.state),
            })
        }
        Event::GroupUpdate(update) => {
            serde_json::json!({
                "group": update.group_jid.to_string(),
                "update": format!("{:?}", update.action),
            })
        }
        Event::IncomingCall(call) => {
            serde_json::json!({
                "from": call.from.to_string(),
                "stanza_id": call.stanza_id,
                "call_id": call.action.call_id().to_string(),
                "call_creator": call.action.call_creator().to_string(),
                "notify": call.notify,
                "platform": call.platform,
                "version": call.version,
                "timestamp": call.timestamp.timestamp(),
                "offline": call.offline,
                "action": format!("{:?}", call.action),
            })
        }
        Event::PictureUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "author": update.author.as_ref().map(|j| j.to_string()).unwrap_or_default(),
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::UserAboutUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "status": update.status,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::SelfPushNameUpdated(update) => {
            serde_json::json!({
                "old_name": update.old_name,
                "new_name": update.new_name,
                "from_server": update.from_server,
            })
        }
        Event::ContactUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
            })
        }
        Event::DeviceListUpdate(update) => {
            serde_json::json!({
                "user": update.user.to_string(),
                "update_type": format!("{:?}", update.update_type),
            })
        }
        Event::PinUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "pinned": update.action.pinned,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::MuteUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "muted": update.action.muted,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::ArchiveUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "archived": update.action.archived,
                "timestamp": update.timestamp.timestamp(),
            })
        }
        Event::MarkChatAsReadUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "timestamp": update.timestamp.timestamp(),
                "from_full_sync": update.from_full_sync,
            })
        }
        Event::UndecryptableMessage(msg) => {
            serde_json::json!({
                "info": format!("{:?}", msg),
            })
        }
        Event::ClientOutdated(info) => {
            serde_json::json!({
                "info": format!("{:?}", info),
            })
        }
        Event::OfflineSyncPreview(preview) => {
            serde_json::json!({
                "info": format!("{:?}", preview),
            })
        }
        Event::OfflineSyncCompleted(completed) => {
            serde_json::json!({
                "info": format!("{:?}", completed),
            })
        }
        Event::CallLogSync(sync) => {
            serde_json::json!({
                "call_creator_jid": sync.call_creator_jid.to_string(),
                "call_id": sync.call_id,
                "from_me": sync.from_me,
                "timestamp": sync.timestamp.timestamp(),
                "from_full_sync": sync.from_full_sync,
                "record": format!("{:?}", sync.record),
            })
        }
        Event::StreamError(err) => {
            serde_json::json!({
                "code": err.code,
            })
        }
        Event::EncDecryptFailed(failed) => {
            serde_json::json!({
                "chat": failed.info.source.chat.to_string(),
                "sender": failed.info.source.sender.to_string(),
                "message_id": failed.info.id,
                "enc_index": failed.enc_index,
                "enc_type": failed.enc_type,
                "reason": format!("{:?}", failed.reason),
            })
        }
        Event::PairingQrCode(qr) => {
            serde_json::json!({
                "code": qr.code,
                "timeout_seconds": qr.timeout.as_secs(),
            })
        }
        Event::PairingCode(pair) => {
            serde_json::json!({
                "code": pair.code,
                "timeout_seconds": pair.timeout.as_secs(),
            })
        }
        _ => serde_json::json!({}),
    };

    serde_json::json!({
        "session_id": session_id,
        "event": event_type,
        "timestamp": timestamp,
        "data": data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_event_variants_map_to_their_webhook_names() {
        use wacore::types::events::{CallLogSync, EncDecryptFailed, Event, StreamError};

        let stream_error =
            Event::StreamError(StreamError::builder().code("429".to_string()).build());
        assert_eq!(get_event_type(&stream_error), "stream_error");

        let call_log = Event::CallLogSync(
            CallLogSync::builder()
                .call_creator_jid("12345@s.whatsapp.net".parse().unwrap())
                .call_id("call-1".to_string())
                .from_me(true)
                .timestamp(chrono::Utc::now())
                .record(Box::new(waproto::whatsapp::CallLogRecord::default()))
                .from_full_sync(false)
                .build(),
        );
        assert_eq!(get_event_type(&call_log), "call_log_sync");

        let enc_failed = Event::EncDecryptFailed(
            EncDecryptFailed::builder()
                .info(std::sync::Arc::new(
                    wacore::types::message::MessageInfo::default(),
                ))
                .enc_index(1)
                .enc_type(std::borrow::Cow::Borrowed("skmsg"))
                .reason(wacore::types::events::EncDecryptFailureReason::NoSession)
                .build(),
        );
        assert_eq!(get_event_type(&enc_failed), "enc_decrypt_failed");
    }

    #[test]
    fn new_event_variants_serialize_with_event_name_and_data() {
        use wacore::types::events::{CallLogSync, EncDecryptFailed, Event, StreamError};

        let stream_error =
            Event::StreamError(StreamError::builder().code("429".to_string()).build());
        let json = event_to_json(&stream_error, "sess");
        assert_eq!(json["event"], "stream_error");
        assert_eq!(json["session_id"], "sess");
        assert_eq!(json["data"]["code"], "429");

        let call_log = Event::CallLogSync(
            CallLogSync::builder()
                .call_creator_jid("12345@s.whatsapp.net".parse().unwrap())
                .call_id("call-1".to_string())
                .from_me(true)
                .timestamp(chrono::Utc::now())
                .record(Box::new(waproto::whatsapp::CallLogRecord::default()))
                .from_full_sync(true)
                .build(),
        );
        let json = event_to_json(&call_log, "sess");
        assert_eq!(json["event"], "call_log_sync");
        assert_eq!(json["data"]["call_id"], "call-1");
        assert_eq!(json["data"]["from_me"], true);
        assert_eq!(json["data"]["from_full_sync"], true);

        let enc_failed = Event::EncDecryptFailed(
            EncDecryptFailed::builder()
                .info(std::sync::Arc::new(
                    wacore::types::message::MessageInfo::default(),
                ))
                .enc_index(2)
                .enc_type(std::borrow::Cow::Borrowed("pkmsg"))
                .reason(wacore::types::events::EncDecryptFailureReason::BadMac)
                .build(),
        );
        let json = event_to_json(&enc_failed, "sess");
        assert_eq!(json["event"], "enc_decrypt_failed");
        assert_eq!(json["data"]["enc_index"], 2);
        assert_eq!(json["data"]["enc_type"], "pkmsg");
        assert_eq!(json["data"]["reason"], "BadMac");
    }

    #[test]
    fn qr_and_pair_code_events_carry_the_code_in_their_data() {
        use wacore::types::events::{Event, PairingCode, PairingQrCode};

        let qr = Event::PairingQrCode(
            PairingQrCode::builder()
                .code("2@abc123,def456==".to_string())
                .timeout(std::time::Duration::from_secs(60))
                .build(),
        );
        let json = event_to_json(&qr, "sess");
        assert_eq!(json["event"], "qr_code");
        assert_eq!(json["data"]["code"], "2@abc123,def456==");
        assert_eq!(json["data"]["timeout_seconds"], 60);

        let pair = Event::PairingCode(
            PairingCode::builder()
                .code("ABCD1234".to_string())
                .timeout(std::time::Duration::from_secs(180))
                .build(),
        );
        let json = event_to_json(&pair, "sess");
        assert_eq!(json["event"], "pair_code");
        assert_eq!(json["data"]["code"], "ABCD1234");
        assert_eq!(json["data"]["timeout_seconds"], 180);
    }

    #[test]
    fn zip_round_trips_nested_directory() {
        let src = tempfile::tempdir().expect("src tempdir");
        std::fs::write(src.path().join("device.json"), b"top-level file").unwrap();
        std::fs::create_dir(src.path().join("keys")).unwrap();
        std::fs::write(src.path().join("keys/identity.bin"), b"nested file").unwrap();

        let zip_bytes = zip_directory(src.path().to_str().unwrap()).expect("zip");

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        unzip_directory(dst_path.to_str().unwrap(), &zip_bytes).expect("unzip");

        assert_eq!(
            std::fs::read(dst_path.join("device.json")).unwrap(),
            b"top-level file"
        );
        assert_eq!(
            std::fs::read(dst_path.join("keys/identity.bin")).unwrap(),
            b"nested file"
        );
    }

    #[test]
    fn unzip_rejects_path_traversal() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("../../etc/passwd", options).unwrap();
            std::io::Write::write_all(&mut writer, b"pwned").unwrap();
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        assert!(result.is_err(), "path traversal entry must be rejected");
    }

    #[test]
    fn unzip_rejects_too_many_entries() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            for i in 0..=MAX_IMPORT_ENTRIES {
                writer.start_file(format!("f{i}"), options).unwrap();
            }
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        let err = result.expect_err("entry-count limit must be enforced");
        assert!(err.to_string().contains("entries"), "{err}");
    }

    /// The filler is all zeros -- highly compressible, so this zip stays
    /// small on disk. A real zip bomb needs the *decompressed* size
    /// checked, not the compressed size; that's exactly what this proves.
    #[test]
    fn unzip_rejects_entry_over_the_per_file_size_cap() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("huge.bin", options).unwrap();
            let chunk = vec![0u8; 8 * 1024 * 1024];
            let chunks_needed = (MAX_IMPORT_ENTRY_UNCOMPRESSED_BYTES / chunk.len() as u64) + 1;
            for _ in 0..chunks_needed {
                std::io::Write::write_all(&mut writer, &chunk).unwrap();
            }
            writer.finish().unwrap();
        }

        let dst = tempfile::tempdir().expect("dst tempdir");
        let dst_path = dst.path().join("restored");
        let result = unzip_directory(dst_path.to_str().unwrap(), &buf.into_inner());
        let err = result.expect_err("per-entry uncompressed size limit must be enforced");
        assert!(err.to_string().contains("per-file"), "{err}");
    }
}
