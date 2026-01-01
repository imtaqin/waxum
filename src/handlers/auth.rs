use axum::{extract::State, Json};
use std::sync::Arc;

use crate::error::ApiError;
use crate::models::auth::{QrCodeResponse, StatusResponse};
use crate::models::common::SuccessResponse;
use crate::state::{AppState, ConnectionState};

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/auth/qr",
    tag = "auth",
    responses(
        (status = 200, description = "QR codes for scanning", body = QrCodeResponse),
        (status = 503, description = "Not connected or no QR codes available")
    )
)]
pub async fn get_qr_code(State(state): State<AppState>) -> Result<Json<QrCodeResponse>, ApiError> {
    let qr_codes = state.get_qr_codes();
    let status = state.get_connection_status();

    Ok(Json(QrCodeResponse {
        qr_codes,
        timeout_seconds: 60,
        status: status.into(),
    }))
}

#[utoipa::path(
    get,
    security(("bearer_auth" = [])),
    path = "/api/v1/auth/status",
    tag = "auth",
    responses(
        (status = 200, description = "Current status", body = StatusResponse)
    )
)]
pub async fn get_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let status = state.get_connection_status();
    let logged_in = status == ConnectionState::LoggedIn;

    Json(StatusResponse {
        status: status.into(),
        logged_in,
        phone_number: None,
        push_name: None,
    })
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/auth/connect",
    tag = "auth",
    responses(
        (status = 200, description = "Connection initiated", body = SuccessResponse),
        (status = 409, description = "Already connected")
    )
)]
pub async fn connect(State(state): State<AppState>) -> Result<Json<SuccessResponse>, ApiError> {
    let current_status = state.get_connection_status();
    if current_status != ConnectionState::Disconnected {
        return Err(ApiError::AlreadyConnected);
    }

    state.set_connection_status(ConnectionState::Connecting);

    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = connect_client(state_clone).await {
            tracing::error!("Connection failed: {}", e);
        }
    });

    Ok(Json(SuccessResponse::with_message("Connection initiated")))
}

async fn connect_client(state: AppState) -> Result<(), ApiError> {
    use whatsapp_rust::bot::Bot;
    use whatsapp_rust_sqlite_storage::SqliteStore;
    use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
    use whatsapp_rust_ureq_http_client::UreqHttpClient;

    let storage_path = state.get_storage_path();
    let db_path = format!("{}/whatsapp.db", storage_path);

    let backend = SqliteStore::new(&db_path)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let transport_factory = TokioWebSocketTransportFactory;
    let http_client = UreqHttpClient::new();

    let state_for_events = state.clone();

    let mut bot = Bot::builder()
        .with_backend(Arc::new(backend))
        .with_transport_factory(transport_factory)
        .with_http_client(http_client)
        .on_event(move |event, _client| {
            let state = state_for_events.clone();
            async move {
                handle_event(event, state).await;
            }
        })
        .build()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    state.set_client(Some(bot.client()));
    state.set_connection_status(ConnectionState::WaitingForQr);

    let handle = bot
        .run()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let _ = handle.await;

    state.set_connection_status(ConnectionState::Disconnected);
    state.set_client(None);

    Ok(())
}

async fn handle_event(event: wacore::types::events::Event, state: AppState) {
    use wacore::types::events::Event;

    match event {
        Event::PairingQrCode { code, timeout: _ } => {
            tracing::info!("QR code received");
            state.set_qr_codes(vec![code]);
            state.set_connection_status(ConnectionState::WaitingForQr);
        }
        Event::PairingCode { code, timeout: _ } => {
            tracing::info!("Pair code received: {}", code);
            state.set_pair_code(Some(code));
        }
        Event::Connected(_) => {
            tracing::info!("Connected to WhatsApp");
            state.set_connection_status(ConnectionState::LoggedIn);
            state.set_qr_codes(vec![]);
        }
        Event::Disconnected(_) => {
            tracing::warn!("Disconnected from WhatsApp");
            state.set_connection_status(ConnectionState::Disconnected);
        }
        Event::LoggedOut(logged_out) => {
            tracing::warn!("Logged out: {:?}", logged_out.reason);
            state.set_connection_status(ConnectionState::Disconnected);
            state.set_client(None);
        }
        Event::Message(_msg, info) => {
            tracing::debug!("Message received: {:?}", info);

            if let Ok(payload) = serde_json::to_string(&serde_json::json!({
                "event": "message_received",
                "timestamp": chrono::Utc::now().timestamp(),
                "data": {
                    "from": info.source.sender.to_string(),
                    "chat": info.source.chat.to_string(),
                }
            })) {
                state.broadcast_event(payload);
            }
        }
        Event::Receipt(receipt) => {
            tracing::debug!("Receipt received: {:?}", receipt);
        }
        _ => {
            tracing::debug!("Other event received");
        }
    }
}

#[utoipa::path(
    post,
    security(("bearer_auth" = [])),
    path = "/api/v1/auth/disconnect",
    tag = "auth",
    responses(
        (status = 200, description = "Disconnected", body = SuccessResponse),
        (status = 503, description = "Not connected")
    )
)]
pub async fn disconnect(State(state): State<AppState>) -> Result<Json<SuccessResponse>, ApiError> {
    let client = state.get_client().ok_or(ApiError::NotConnected)?;

    client.disconnect().await;
    state.set_connection_status(ConnectionState::Disconnected);
    state.set_client(None);

    Ok(Json(SuccessResponse::with_message("Disconnected")))
}
