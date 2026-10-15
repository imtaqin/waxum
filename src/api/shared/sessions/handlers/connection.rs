//! Connect, pair, QR, disconnect, unlink and logout, and the client connect/guard helpers.

use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    Json,
};
use futures::stream::Stream;

use crate::api::common::SuccessResponse;
use crate::api::shared::sessions::models::{
    ConnectRequest, PairCodeRequest, PairCodeResponse, QrCodeResponse, SessionStatus,
};
use crate::device_props::ResolvedDeviceProps;
use crate::error::ApiError;
use crate::state::AppState;

use super::*;

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
    let session = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    reject_cloud_session(&session, "connect")?;

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
pub(super) async fn ensure_connecting_for_wait(
    state: &AppState,
    session_id: &str,
    query: &ConnectWaitQuery,
) -> Result<std::sync::Arc<crate::state::SessionState>, ApiError> {
    let existing = state
        .session_manager()
        .get_session(session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if let Some(session) = &existing {
        reject_cloud_session(session, "connect/wait")?;
    }

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
    let session = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    reject_cloud_session(&session, "pair")?;

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
        if is_linked(&client) {
            return Err(ApiError::AlreadyConnected);
        }
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
        if session_client_is_linked(&state, &session_id) {
            return Err(ApiError::AlreadyConnected);
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

/// Turns a session dormant: its WhatsApp login is dropped, everything
/// else stays.
///
/// What goes is the device identity in the session's store (noise and
/// Signal keys, app-state), so the next `connect` shows a fresh QR. What
/// stays is the session row, its webhooks, settings, stored messages and
/// chat list, so scanning again continues the same session.
///
/// This replaces deleting the session, which is what a repeated
/// `LoggedOut` and the console's "log out" used to do: the session id
/// vanished along with its webhooks and history, and whatever was
/// integrated against it had to be set up again.
///
/// The caller must have stopped the client first.
pub(crate) async fn unlink_session(state: &AppState, session_id: &str) {
    if let Some(runtime) = state.get_session(session_id) {
        runtime.set_client(None);
        runtime.set_status(SessionStatus::Disconnected);
        runtime.clear_reconnecting();
        runtime.clear_pair_state();
        runtime.set_pair_code(None);
        runtime.clear_logout_history();
    }

    if let Ok(Some(storage_path)) = state.session_manager().get_storage_path(session_id).await {
        let db_path = format!("{}/whatsapp.db", storage_path);
        if tokio::fs::try_exists(&db_path).await.unwrap_or(false) {
            let reset = match whatsapp_rust_sqlite_storage::SqliteStore::new(&db_path).await {
                Ok(store) => store.reset_device(1).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = reset {
                tracing::warn!(
                    session_id = %session_id,
                    "could not reset the device identity ({e}); the next connect may need a manual re-pair"
                );
            }
        }
    }

    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::Disconnected, false)
        .await;
}

/// Stops a client for good before its store is reset.
///
/// `reset_device` must not run under a live client: its next key or
/// app-state save would write the old identity back into the fresh store.
/// `Client::logout` only disconnects as its last step, so when it is cut
/// short by the timeout the socket and run loop are still up. Disconnect
/// is idempotent, so this is called whether or not logout finished.
pub(super) async fn stop_client(session_id: &str, client: &std::sync::Arc<whatsapp_rust::Client>) {
    client
        .enable_auto_reconnect
        .store(false, std::sync::atomic::Ordering::Relaxed);
    if tokio::time::timeout(REBUILD_DISCONNECT_TIMEOUT, client.disconnect())
        .await
        .is_err()
    {
        tracing::warn!(
            session_id = %session_id,
            "the old client did not disconnect within {}s; resetting its store anyway",
            REBUILD_DISCONNECT_TIMEOUT.as_secs()
        );
    }
}

/// How long [`logout_session`] waits for WhatsApp to acknowledge the
/// device removal before unlinking locally anyway.
pub(super) const LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// Log the session out of WhatsApp without deleting it.
///
/// Removes waxum from the account's linked devices, then makes the session
/// dormant ([`unlink_session`]): the id, webhooks, settings and stored
/// messages are kept, and `POST .../connect` followed by a QR scan (or
/// `POST .../pair`) brings the same session back. Use `DELETE
/// /sessions/{id}` to remove a session and its data for good.
#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/logout",
    tag = "sessions",
    security(("bearer_auth" = [])),
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Logged out; the session is kept and can be linked again", body = SuccessResponse),
        (status = 400, description = "Not available on a whatsapp_cloud session"),
        (status = 404, description = "Session not found")
    )
)]
pub async fn logout_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let session = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;
    reject_cloud_session(&session, "logout")?;

    let client = state.get_session(&session_id).and_then(|runtime| {
        let client = runtime.get_client();
        runtime.set_client(None);
        client
    });
    if let Some(client) = client {
        if tokio::time::timeout(LOGOUT_TIMEOUT, client.logout())
            .await
            .is_err()
        {
            tracing::warn!(
                session_id = %session_id,
                "logout: WhatsApp did not acknowledge within {}s; unlinking locally",
                LOGOUT_TIMEOUT.as_secs()
            );
        }
        stop_client(&session_id, &client).await;
    }
    unlink_session(&state, &session_id).await;

    Ok(Json(SuccessResponse::with_message(
        "Logged out. The session is kept; connect and scan again to link it.",
    )))
}

/// Clears the session's runtime after its client's `bot.run()` returned,
/// but only if that client is still the one installed. A watchdog rebuild
/// (see [`force_rebuild`]) installs a new client while the old `run()` may
/// still be draining a half-open socket; when the old one finally returns
/// it must not mark the session disconnected or drop the new client.
pub(super) async fn mark_disconnected_if_still_current(
    state: &AppState,
    session_id: &str,
    own_client: &std::sync::Arc<whatsapp_rust::Client>,
) {
    if let Some(runtime) = state.get_session(session_id) {
        let still_current = runtime
            .get_client()
            .is_none_or(|installed| is_current_client(Some(installed), own_client));
        if !still_current {
            tracing::debug!(
                session_id = %session_id,
                "a replaced client finished running; leaving the current one in place"
            );
            return;
        }
        runtime.set_status(SessionStatus::Disconnected);
        runtime.set_client(None);
    }
    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::Disconnected, false)
        .await;
}

pub(super) async fn connect_client(
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

    if session_is_cloud(state, session_id).await {
        tracing::info!(
            session_id = %session_id,
            "session switched to whatsapp_cloud before its multi-device client started; not connecting"
        );
        return Ok(());
    }

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

    let own_client = bot.client();
    bot.run().await;
    mark_disconnected_if_still_current(state, session_id, &own_client).await;

    Ok(())
}

pub(super) async fn connect_client_with_pair_code(
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

    let fallback_device_props = device_props.clone();
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

    if session_is_cloud(state, session_id).await {
        tracing::info!(
            session_id = %session_id,
            "session switched to whatsapp_cloud before its multi-device client started; not connecting"
        );
        return Ok(());
    }

    if is_linked(&bot.client()) {
        tracing::warn!(
            session_id = %session_id,
            "pair code requested for a session that is already linked; connecting with the stored login instead"
        );
        drop(bot);
        return connect_client(state, session_id, fallback_device_props).await;
    }

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

    let own_client = bot.client();
    bot.run().await;
    mark_disconnected_if_still_current(state, session_id, &own_client).await;

    Ok(())
}

/// Whether the client already holds a WhatsApp login, i.e. the device was
/// linked at some point and its identity is in the session store. True from
/// the moment the store is loaded, before the socket has logged in.
///
/// A pair-code request must never be sent for such a session (#154). The
/// library only skips it when the client is already *logged in*, and a
/// reconnecting client is not: its socket is up a moment before `<success>`
/// arrives. In that window the request went out, WhatsApp refused it, and
/// the session reported `pairing_code_error` while otherwise connected.
/// `status == LoggedIn` has the same gap, which is why `POST .../pair`
/// checks this instead of the status alone.
pub(super) fn is_linked(client: &whatsapp_rust::client::Client) -> bool {
    client.pn().is_some()
}

pub(super) fn session_client_is_linked(state: &AppState, session_id: &str) -> bool {
    state
        .get_session(session_id)
        .and_then(|r| r.get_client())
        .is_some_and(|c| is_linked(&c))
}

/// Whether `emitter` is the client currently installed on the session.
///
/// A session can briefly have two clients: a watchdog rebuild or a manual
/// reconnect installs a new one while the old one is still winding down,
/// and on a half-open socket that can take minutes. Events from the old
/// client must not act on the session: its `Disconnected` would mark the
/// new connection down, and its `LoggedOut` could purge the session and
/// its keys. [`handle_event`] therefore drops every event from a client
/// that is not the installed one, except `Messages`, which are real
/// inbound messages (and deduplicated per session). With no client
/// installed at all (after a disconnect), nothing is current.
pub(super) fn is_current_client<T>(
    installed: Option<std::sync::Arc<T>>,
    emitter: &std::sync::Arc<T>,
) -> bool {
    installed.is_some_and(|c| std::sync::Arc::ptr_eq(&c, emitter))
}

#[cfg(test)]
mod client_guard_tests {
    use super::is_current_client;
    use std::sync::Arc;

    #[test]
    fn only_the_installed_client_is_current() {
        let current = Arc::new(1u8);
        let replaced = Arc::new(1u8);
        assert!(is_current_client(Some(current.clone()), &current));
        assert!(
            !is_current_client(Some(current.clone()), &replaced),
            "an equal value in a different client is still a different client"
        );
        assert!(
            !is_current_client(None, &current),
            "nothing is current after a disconnect"
        );
    }
}
