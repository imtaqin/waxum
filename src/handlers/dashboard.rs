use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::Form;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::middleware::jwt::get_superadmin_token;
use crate::models::sessions::{SessionInfo, SessionStatus};
use crate::state::AppState;

pub struct SessionView {
    pub id: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub status: SessionStatus,
    pub created_at: String,
}

impl From<SessionInfo> for SessionView {
    fn from(s: SessionInfo) -> Self {
        let created_at = Utc
            .timestamp_opt(s.created_at, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| s.created_at.to_string());

        SessionView {
            id: s.id,
            name: s.name,
            phone: s.phone_number,
            status: s.status,
            created_at,
        }
    }
}

#[derive(Template)]
#[template(path = "dashboard.askama")]
pub struct DashboardTemplate {
    pub total_sessions: usize,
    pub connected_sessions: usize,
    pub disconnected_sessions: usize,
    pub sessions: Vec<SessionView>,
}

#[derive(Template)]
#[template(path = "sessions.askama")]
pub struct SessionsTemplate {
    pub sessions: Vec<SessionView>,
}

#[derive(Template)]
#[template(path = "session_new.askama")]
pub struct SessionNewTemplate {
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "session_detail.askama")]
pub struct SessionDetailTemplate {
    pub session: SessionView,
    pub qr_code: Option<String>,
    pub qr_error: Option<String>,
    pub pair_code: Option<String>,
    pub device_info: Option<DeviceInfoView>,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "settings.askama")]
pub struct SettingsTemplate {
    pub token: String,
    pub version: String,
    pub api_url: String,
}

pub struct DeviceInfoView {
    pub phone: String,
    pub platform: String,
    pub push_name: String,
}

#[derive(Deserialize)]
pub struct CreateSessionForm {
    pub id: String,
    pub name: Option<String>,
    pub webhook_url: Option<String>,
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub events: String,
}

impl CreateSessionForm {
    pub fn get_events(&self) -> Vec<String> {
        if self.events.is_empty() {
            return Vec::new();
        }
        self.events
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

#[derive(Deserialize)]
pub struct PairCodeForm {
    pub phone: String,
}

fn generate_qr_image(data: &str) -> Option<String> {
    use base64::Engine;
    use image::{ImageEncoder, Luma};
    use qrcode::QrCode;

    let code = QrCode::new(data.as_bytes()).ok()?;
    let image = code.render::<Luma<u8>>().build();

    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::L8,
        )
        .ok()?;

    Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
}

pub async fn dashboard_home(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let mut updated_sessions: Vec<SessionView> = Vec::with_capacity(sessions.len());
    let mut connected = 0;
    let mut disconnected = 0;

    for mut session in sessions {
        if let Some(runtime) = state.get_session(&session.id) {
            session.status = runtime.get_status();
        }
        if session.status.is_connected() {
            connected += 1;
        } else {
            disconnected += 1;
        }
        updated_sessions.push(session.into());
    }

    let template = DashboardTemplate {
        total_sessions: updated_sessions.len(),
        connected_sessions: connected,
        disconnected_sessions: disconnected,
        sessions: updated_sessions,
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
}

pub async fn sessions_list(State(state): State<AppState>) -> impl IntoResponse {
    let sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let updated_sessions: Vec<SessionView> = sessions
        .into_iter()
        .map(|mut session| {
            if let Some(runtime) = state.get_session(&session.id) {
                session.status = runtime.get_status();
            }
            session.into()
        })
        .collect();

    let template = SessionsTemplate {
        sessions: updated_sessions,
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
}

pub async fn session_new_form() -> impl IntoResponse {
    let template = SessionNewTemplate { error: None };
    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
}

pub async fn session_create(
    State(state): State<AppState>,
    Form(form): Form<CreateSessionForm>,
) -> Response {
    let session_id = form.id.trim().to_string();

    if session_id.is_empty() {
        let template = SessionNewTemplate {
            error: Some("Session ID is required".to_string()),
        };
        return Html(
            template
                .render()
                .unwrap_or_else(|e| format!("Template error: {}", e)),
        )
        .into_response();
    }

    if let Ok(Some(_)) = state.session_manager().get_session(&session_id).await {
        let template = SessionNewTemplate {
            error: Some(format!("Session '{}' already exists", session_id)),
        };
        return Html(
            template
                .render()
                .unwrap_or_else(|e| format!("Template error: {}", e)),
        )
        .into_response();
    }

    let storage_path = format!("{}/{}", state.base_storage_path(), session_id);
    if let Err(e) = tokio::fs::create_dir_all(&storage_path).await {
        let template = SessionNewTemplate {
            error: Some(format!("Failed to create storage: {}", e)),
        };
        return Html(
            template
                .render()
                .unwrap_or_else(|e| format!("Template error: {}", e)),
        )
        .into_response();
    }

    let form_events = form.get_events();
    let name = form.name.filter(|n| !n.trim().is_empty());
    let webhook_url = form.webhook_url.filter(|u| !u.trim().is_empty());
    let webhook_secret = form.webhook_secret.filter(|s| !s.trim().is_empty());

    if let Err(e) = state
        .session_manager()
        .create_session(&session_id, name.as_deref(), &storage_path)
        .await
    {
        let template = SessionNewTemplate {
            error: Some(format!("Failed to create session: {}", e)),
        };
        return Html(
            template
                .render()
                .unwrap_or_else(|e| format!("Template error: {}", e)),
        )
        .into_response();
    }

    if let Some(webhook_url) = webhook_url {
        use crate::models::webhooks::{WebhookConfig, WebhookEvent};

        let webhook_id = uuid::Uuid::new_v4().to_string();
        let events: Vec<WebhookEvent> = form_events
            .iter()
            .filter_map(|e| match e.as_str() {
                "message" => Some(WebhookEvent::Message),
                "connected" => Some(WebhookEvent::Connected),
                "disconnected" => Some(WebhookEvent::Disconnected),
                "receipt" => Some(WebhookEvent::Receipt),
                "presence" => Some(WebhookEvent::Presence),
                "qr_code" => Some(WebhookEvent::QrCode),
                _ => None,
            })
            .collect();

        let events = if events.is_empty() {
            vec![WebhookEvent::All]
        } else {
            events
        };

        let config = WebhookConfig {
            url: webhook_url,
            events,
            secret: webhook_secret,
            enabled: true,
        };

        let _ = state
            .session_manager()
            .create_webhook(&webhook_id, &session_id, &config)
            .await;
        state.register_webhook(&session_id, &webhook_id, config);
    }

    Redirect::to(&format!("/dashboard/sessions/{}", session_id)).into_response()
}

pub async fn session_detail(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    let mut session = match state.session_manager().get_session(&session_id).await {
        Ok(Some(s)) => s,
        _ => {
            return Redirect::to("/dashboard/sessions").into_response();
        }
    };

    if let Some(runtime) = state.get_session(&session_id) {
        session.status = runtime.get_status();
    }

    let (qr_code, qr_error) = if !session.status.is_connected() {
        if let Some(runtime) = state.get_session(&session_id) {
            let qr_codes = runtime.get_qr_codes();
            if let Some(qr_data) = qr_codes.first() {
                (generate_qr_image(qr_data), None)
            } else {
                (
                    None,
                    Some("Waiting for QR code... Click Connect to start".to_string()),
                )
            }
        } else {
            (
                None,
                Some("Session not connected. Click Connect to start.".to_string()),
            )
        }
    } else {
        (None, None)
    };

    let device_info = if session.status.is_connected() {
        if let Some(runtime) = state.get_session(&session_id) {
            if let Some(client) = runtime.get_client() {
                let push_name = client.get_push_name().await;
                let phone = client
                    .get_pn()
                    .await
                    .map(|j| j.user.clone())
                    .unwrap_or_default();
                Some(DeviceInfoView {
                    phone,
                    platform: "WhatsApp".to_string(),
                    push_name,
                })
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let template = SessionDetailTemplate {
        session: session.into(),
        qr_code,
        qr_error,
        pair_code: None,
        device_info,
        message: None,
        error: None,
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
    .into_response()
}

pub async fn session_connect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    let _session = match state.session_manager().get_session(&session_id).await {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/dashboard/sessions").into_response(),
    };

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    if let Err(e) = tokio::fs::create_dir_all(&storage_path).await {
        tracing::error!("Failed to create storage directory {}: {}", storage_path, e);
        return Redirect::to(&format!(
            "/dashboard/sessions/{}?error=Failed to create storage",
            session_id
        ))
        .into_response();
    }

    let runtime = state.get_or_create_session(&session_id, &storage_path);
    runtime.set_status(SessionStatus::Connecting);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        if let Err(e) =
            crate::handlers::sessions::connect_client_public(&state_clone, &session_id_clone).await
        {
            tracing::error!("Session {} connection failed: {}", session_id_clone, e);
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Redirect::to(&format!("/dashboard/sessions/{}", session_id)).into_response()
}

pub async fn session_disconnect(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
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

    Redirect::to(&format!(
        "/dashboard/sessions/{}?message=Disconnected",
        session_id
    ))
    .into_response()
}

pub async fn session_delete(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    if let Some(runtime) = state.get_session(&session_id) {
        if let Some(client) = runtime.get_client() {
            client.disconnect().await;
        }
    }

    state.remove_session(&session_id);

    if let Ok(Some(storage_path)) = state.session_manager().get_storage_path(&session_id).await {
        let _ = tokio::fs::remove_dir_all(&storage_path).await;
    }

    let _ = state.session_manager().delete_session(&session_id).await;

    Redirect::to("/dashboard/sessions").into_response()
}

pub async fn session_pair(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Form(form): Form<PairCodeForm>,
) -> Response {
    let mut session = match state.session_manager().get_session(&session_id).await {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/dashboard/sessions").into_response(),
    };

    if let Some(runtime) = state.get_session(&session_id) {
        session.status = runtime.get_status();
    }

    let storage_path = state
        .session_manager()
        .get_storage_path(&session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("{}/{}", state.base_storage_path(), session_id));

    if let Err(e) = tokio::fs::create_dir_all(&storage_path).await {
        tracing::error!("Failed to create storage directory {}: {}", storage_path, e);
        return Redirect::to(&format!(
            "/dashboard/sessions/{}?error=Failed to create storage",
            session_id
        ))
        .into_response();
    }

    let runtime = state.get_or_create_session(&session_id, &storage_path);
    runtime.set_status(SessionStatus::WaitingForPairCode);

    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    let phone = form.phone.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::handlers::sessions::connect_client_with_pair_code_public(
            &state_clone,
            &session_id_clone,
            &phone,
            false,
        )
        .await
        {
            tracing::error!("Session {} pair code failed: {}", session_id_clone, e);
            if let Some(runtime) = state_clone.get_session(&session_id_clone) {
                runtime.set_status(SessionStatus::Disconnected);
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let pair_code = state
        .get_session(&session_id)
        .and_then(|r| r.get_pair_code());

    let error = if pair_code.is_none() {
        Some("Failed to get pair code. Please try again.".to_string())
    } else {
        None
    };

    let template = SessionDetailTemplate {
        session: session.into(),
        qr_code: None,
        qr_error: None,
        pair_code,
        device_info: None,
        message: None,
        error,
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
    .into_response()
}

pub async fn settings_page() -> impl IntoResponse {
    let (token, _) = get_superadmin_token();

    let template = SettingsTemplate {
        token,
        version: env!("CARGO_PKG_VERSION").to_string(),
        api_url: "http://localhost:3451/api/v1".to_string(),
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("Template error: {}", e)),
    )
}
