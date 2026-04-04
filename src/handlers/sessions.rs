use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use crate::models::common::SuccessResponse;
use crate::models::sessions::{
    CreateSessionRequest, CreateSessionResponse, DeviceInfo, PairCodeRequest, PairCodeResponse,
    QrCodeResponse, SessionInfo, SessionListResponse, SessionStatus, SessionStatusResponse,
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

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone).await {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
            }
        }
    });

    Ok(Json(CreateSessionResponse { session }))
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
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut updated_sessions = Vec::with_capacity(sessions.len());
    for mut session in sessions {
        if let Some(runtime) = state.get_session(&session.id) {
            session.status = runtime.get_status();
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

    let (status, is_logged_in) = if let Some(runtime) = state.get_session(&session_id) {
        let s = runtime.get_status();
        (s, s == SessionStatus::LoggedIn)
    } else {
        (session.status, session.is_logged_in)
    };

    Ok(Json(SessionStatusResponse {
        status,
        is_logged_in,
        phone_number: session.phone_number,
        push_name: session.push_name,
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
    responses(
        (status = 200, description = "Connection initiated", body = SuccessResponse),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Already connected")
    )
)]
pub async fn connect_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let _ = state
        .session_manager()
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::SessionNotFound(session_id.clone()))?;

    if let Some(runtime) = state.get_session(&session_id) {
        let status = runtime.get_status();
        if status != SessionStatus::Disconnected {
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
    runtime.set_status(SessionStatus::Connecting);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_client(&state_clone, &session_id_clone).await {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
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
    runtime.set_status(SessionStatus::WaitingForPairCode);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    let phone_number = request.phone_number.clone();
    let show_notification = request.show_push_notification;

    tokio::spawn(async move {
        if let Err(e) = connect_client_with_pair_code(
            &state_clone,
            &session_id_clone,
            &phone_number,
            show_notification,
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

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let pair_code = state
        .get_session(&session_id)
        .and_then(|r| r.get_pair_code())
        .unwrap_or_default();

    Ok(Json(PairCodeResponse {
        code: pair_code,
        timeout_seconds: 60,
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

    let push_name_str = client.get_push_name().await;
    let push_name = if push_name_str.is_empty() {
        None
    } else {
        Some(push_name_str)
    };
    let pn = client.get_pn().await.map(|j| j.to_string());
    let lid = client.get_lid().await.map(|j| j.to_string());

    Ok(Json(DeviceInfo {
        device_id: None,
        phone_number: pn,
        lid,
        push_name,
    }))
}

async fn connect_client(state: &AppState, session_id: &str) -> Result<(), ApiError> {
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::TokioRuntime;
    use whatsapp_rust_sqlite_storage::SqliteStore;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
    use whatsapp_rust_ureq_http_client::UreqHttpClient;

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
    let http_client = UreqHttpClient::new();

    let state_for_events = state.clone();
    let session_id_for_events = session_id.to_string();

    let mut bot = Bot::builder()
        .with_backend(Arc::new(backend))
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .with_runtime(TokioRuntime)
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
        runtime.set_client(Some(bot.client()));
        runtime.set_status(SessionStatus::WaitingForQr);
    }

    let _ = state
        .session_manager()
        .update_session_status(session_id, SessionStatus::WaitingForQr, false)
        .await;

    let handle = bot
        .run()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let _ = handle.await;

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
) -> Result<(), ApiError> {
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust::pair_code::PairCodeOptions;
    use whatsapp_rust::TokioRuntime;
    use whatsapp_rust_sqlite_storage::SqliteStore;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
    use whatsapp_rust_ureq_http_client::UreqHttpClient;

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
    let http_client = UreqHttpClient::new();

    let state_for_events = state.clone();
    let session_id_for_events = session_id.to_string();

    let pair_options = PairCodeOptions {
        phone_number: phone_number.to_string(),
        show_push_notification: show_notification,
        custom_code: None,
        platform_id: whatsapp_rust::pair_code::PlatformId::Chrome,
        platform_display: "Chrome (Linux)".to_string(),
    };

    let mut bot = Bot::builder()
        .with_backend(Arc::new(backend))
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .with_runtime(TokioRuntime)
        .with_pair_code(pair_options)
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
        runtime.set_client(Some(bot.client()));
    }

    let handle = bot
        .run()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let _ = handle.await;

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
    event: wacore::types::events::Event,
    state: &AppState,
    session_id: &str,
    client: std::sync::Arc<whatsapp_rust::Client>,
) {
    use wacore::types::events::Event;

    let runtime = match state.get_session(session_id) {
        Some(r) => r,
        None => return,
    };

    match &event {
        Event::PairingQrCode { code, timeout: _ } => {
            tracing::info!("Session {}: QR code received", session_id);
            runtime.set_qr_codes(vec![code.clone()]);
            runtime.set_status(SessionStatus::WaitingForQr);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::WaitingForQr, false)
                .await;
        }
        Event::PairingCode { code, timeout: _ } => {
            tracing::info!("Session {}: Pair code received: {}", session_id, code);
            runtime.set_pair_code(Some(code.clone()));
        }
        Event::Connected(_) => {
            tracing::info!("Session {}: Connected", session_id);
            runtime.set_status(SessionStatus::LoggedIn);
            runtime.set_qr_codes(vec![]);
            runtime.set_pair_code(None);

            let push_name_str = client.get_push_name().await;
            let push_name = if push_name_str.is_empty() {
                None
            } else {
                Some(push_name_str)
            };
            let phone = client.get_pn().await.map(|j| j.user.clone());

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
        Event::Disconnected(_) => {
            tracing::warn!("Session {}: Disconnected", session_id);
            runtime.set_status(SessionStatus::Disconnected);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Disconnected, false)
                .await;
        }
        Event::LoggedOut(logged_out) => {
            tracing::warn!(
                "Session {}: Logged out: {:?}",
                session_id,
                logged_out.reason
            );
            runtime.set_status(SessionStatus::Disconnected);
            runtime.set_client(None);
            let _ = state
                .session_manager()
                .update_session_status(session_id, SessionStatus::Disconnected, false)
                .await;
        }
        _ => {}
    }

    if let Ok(payload) = serde_json::to_string(&event_to_json(&event, session_id)) {
        let event_type = get_event_type(&event);
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
        Event::Message(_, _) => "message".to_string(),
        Event::Receipt(_) => "receipt".to_string(),
        Event::Presence(_) => "presence".to_string(),
        Event::ChatPresence(_) => "chat_presence".to_string(),
        Event::GroupUpdate(_) => "group_update".to_string(),
        Event::JoinedGroup(_) => "joined_group".to_string(),
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

fn event_to_json(event: &wacore::types::events::Event, session_id: &str) -> serde_json::Value {
    use wacore::types::events::Event;

    let event_type = get_event_type(event);
    let timestamp = chrono::Utc::now().timestamp();

    let data = match event {
        Event::Message(_msg, info) => {
            serde_json::json!({
                "from": info.source.sender.to_string(),
                "chat": info.source.chat.to_string(),
                "message_id": info.id.to_string(),
                "timestamp": info.timestamp,
                "is_from_me": info.source.is_from_me,
            })
        }
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
