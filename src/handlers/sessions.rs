use axum::{
    extract::{Multipart, Path, State},
    response::Response as AxumResponse,
    Json,
};
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
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Session ID already exists")
    )
)]
pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<CreateSessionResponse>, ApiError> {
    let session_id = request.id.unwrap_or_else(|| Uuid::new_v4().to_string());

    if state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some()
    {
        return Err(ApiError::AlreadyConnected);
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
        session.status = runtime.get_status();
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

    let (status, is_logged_in, pair) = if let Some(runtime) = state.get_session(&session_id) {
        let ps = runtime.get_pair_state();
        let pair = crate::models::sessions::PairStatus {
            last_qr_at: ps.last_qr_at,
            last_pair_code_at: ps.last_pair_code_at,
            pair_code_expires_at: ps.pair_code_expires_at,
            last_error: ps.last_error,
            attempts: ps.attempts,
        };
        if runtime.is_alive() {
            (SessionStatus::LoggedIn, true, pair)
        } else {
            let s = runtime.get_status();
            let (status, is_logged_in) = if s == SessionStatus::LoggedIn {
                (SessionStatus::Connecting, true)
            } else {
                (s, false)
            };
            (status, is_logged_in, pair)
        }
    } else {
        (
            session.status,
            session.is_logged_in,
            crate::models::sessions::PairStatus::default(),
        )
    };

    Ok(Json(SessionStatusResponse {
        status,
        is_logged_in,
        phone_number: session.phone_number,
        push_name: session.push_name,
        pair,
    }))
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
            SessionStatus::Connecting
                | SessionStatus::WaitingForQr
                | SessionStatus::WaitingForPairCode
        ) {
            return Err(ApiError::AlreadyConnected);
        }
        runtime.set_client(None);
        runtime.set_status(SessionStatus::Disconnected);
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

/// Unzip into `dir`, creating it if needed. Rejects entries whose path
/// would escape `dir` (zip-slip) instead of writing them. Blocking;
/// run inside `spawn_blocking`.
fn unzip_directory(dir: &str, zip_bytes: &[u8]) -> anyhow::Result<()> {
    let base = std::path::Path::new(dir);
    std::fs::create_dir_all(base)?;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
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
        let out_path = base.join(rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut file, &mut out)?;
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

    let push_name_str = client.get_push_name();
    let push_name = if push_name_str.is_empty() {
        None
    } else {
        Some(push_name_str)
    };
    let pn = client.get_pn().map(|j| j.to_string());
    let lid = client.get_lid().map(|j| j.to_string());

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
        c.enable_auto_reconnect
            .store(true, std::sync::atomic::Ordering::Relaxed);
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
        c.enable_auto_reconnect
            .store(true, std::sync::atomic::Ordering::Relaxed);
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
            runtime.set_status(SessionStatus::LoggedIn);
            runtime.set_qr_codes(vec![]);
            runtime.set_pair_code(None);
            runtime.clear_pair_state();

            let push_name_str = client.get_push_name();
            let push_name = if push_name_str.is_empty() {
                None
            } else {
                Some(push_name_str)
            };
            let phone = client.get_pn().map(|j| j.user.clone());

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
        }
        Event::LoggedOut(logged_out) => {
            if let Some(client) = runtime.get_client() {
                client.disconnect().await;
            }
            runtime.set_status(SessionStatus::Disconnected);
            runtime.set_client(None);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Disconnected, false)
                .await;

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
            if !call_id.is_empty() {
                state.incoming_calls().insert(call_id, call.clone());
            }
        }
        _ => {}
    }

    persist_contact_event(state, session_id, event.as_ref()).await;

    if let Event::Messages(batch) = event.as_ref() {
        let timestamp = chrono::Utc::now().timestamp();
        for im in batch.messages.iter() {
            crate::handlers::search::record_incoming(state, session_id, &im.message, &im.info)
                .await;
            let data = message_event_data(&im.message, &im.info);
            let payload_value = serde_json::json!({
                "session_id": session_id,
                "event": "message",
                "timestamp": timestamp,
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
        Event::PushNameUpdate(_) => "push_name_update".to_string(),
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
) -> serde_json::Value {
    let (text, caption, message_type, media_mimetype) = extract_message_content(msg);
    let media_meta = extract_media_metadata(msg);
    let location = extract_location(msg);
    serde_json::json!({
        "from": info.source.sender.to_string(),
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
                push_name = Some(info.push_name.clone());
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
    let mut push_name = None::<String>;
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
        Event::PushNameUpdate(u) => {
            jid_str = u.jid.to_string();
            if !u.new_push_name.is_empty() {
                push_name = Some(u.new_push_name.clone());
            }
            if u.jid.server == wacore_binary::jid::SERVER_JID {
                phone_str = Some(u.jid.user.to_string());
            }
            source = "push_name";
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
            Some(im) => message_event_data(&im.message, &im.info),
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
        Event::PushNameUpdate(update) => {
            serde_json::json!({
                "jid": update.jid.to_string(),
                "old_push_name": update.old_push_name,
                "new_push_name": update.new_push_name,
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
}
