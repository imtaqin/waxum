//! HTTP handlers for the browser-facing console. All auth is via a
//! single `waxum_console` cookie carrying the superadmin token — no
//! CSRF token flow because every write endpoint is a POST from
//! same-origin JS driven by the operator who already holds the cookie,
//! and there is no third-party embedding surface. SameSite=Strict on
//! the cookie makes cross-site POSTs unusable.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Form, Json,
};
use serde::{Deserialize, Serialize};

use crate::console::{render_page, render_partial, CONSOLE_COOKIE};
use crate::models::sessions::SessionStatus;
use crate::state::AppState;

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix(&format!("{CONSOLE_COOKIE}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn token_is_superadmin(token: &str) -> bool {
    if let Ok(superadmin) = std::env::var("SUPERADMIN_TOKEN") {
        if !superadmin.is_empty() && token == superadmin {
            return true;
        }
    }
    let jwt_auth = crate::middleware::jwt::JwtAuth::new();
    match jwt_auth.validate_token(token) {
        Ok(claims) => crate::middleware::jwt::JwtAuth::is_superadmin(&claims),
        Err(_) => false,
    }
}

fn require_auth(headers: &HeaderMap) -> Result<(), Box<Response>> {
    match cookie_token(headers) {
        Some(t) if token_is_superadmin(&t) => Ok(()),
        _ => Err(Box::new(redirect_to("/login"))),
    }
}

fn html(body: String) -> Response {
    let mut r = Response::new(Body::from(body));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn redirect_to(path: &str) -> Response {
    let mut r = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .body(Body::empty())
        .unwrap();
    r.headers_mut()
        .insert(header::LOCATION, HeaderValue::from_str(path).unwrap());
    r
}

#[derive(Serialize)]
struct EventRow {
    time: String,
    kind: String,
    level: String,
    msg: String,
    session: String,
}

#[derive(Serialize)]
struct SessionRow {
    id: String,
    phone: Option<String>,
    status_class: String,
    status_label: String,
}

#[derive(Serialize)]
struct OverviewData {
    version: &'static str,
    connected: u32,
    total: u32,
    pairing_count: u32,
    disconnected_count: u32,
    webhook_count: usize,
    circuits_open: usize,
    event_rate: u32,
    sessions: Vec<SessionRow>,
    sessions_count: usize,
    has_sessions: bool,
    events: Vec<EventRow>,
    has_events: bool,
}

fn status_row(s: SessionStatus) -> (&'static str, &'static str) {
    match s {
        SessionStatus::Connected | SessionStatus::LoggedIn => ("connected", "CONNECTED"),
        SessionStatus::Connecting => ("connecting", "CONNECTING"),
        SessionStatus::Disconnected => ("disconnected", "OFFLINE"),
        _ => ("disconnected", "OFFLINE"),
    }
}

async fn build_overview_data(state: &AppState) -> OverviewData {
    let db_sessions = state
        .session_manager()
        .list_sessions()
        .await
        .unwrap_or_default();

    let mut rows: Vec<SessionRow> = Vec::with_capacity(db_sessions.len());
    let (mut connected, mut pairing, mut offline) = (0u32, 0u32, 0u32);

    for s in &db_sessions {
        let runtime_status = state
            .get_session(&s.id)
            .map(|r| r.effective_status())
            .unwrap_or(s.status);
        let (cls, label) = status_row(runtime_status);
        match cls {
            "connected" => connected += 1,
            "connecting" => pairing += 1,
            _ => offline += 1,
        }
        rows.push(SessionRow {
            id: s.id.clone(),
            phone: s.phone_number.clone(),
            status_class: cls.to_string(),
            status_label: label.to_string(),
        });
    }

    let webhook_count: usize = db_sessions
        .iter()
        .map(|s| state.get_webhooks(&s.id).len())
        .sum();
    let circuits_open = state.webhook_circuits_open_count();

    let raw = state.recent_events(20);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let event_rate: u32 = raw
        .iter()
        .filter(|e| now_ms - e.at_epoch_ms < 60_000)
        .count() as u32;

    let events: Vec<EventRow> = raw
        .iter()
        .map(|e| {
            let dt = chrono::DateTime::from_timestamp_millis(e.at_epoch_ms)
                .unwrap_or_else(chrono::Utc::now);
            let level = if e.event_type.contains("error") || e.event_type.contains("fail") {
                "err"
            } else if e.event_type.contains("disconnect")
                || e.event_type.contains("qr")
                || e.event_type.contains("logout")
            {
                "warn"
            } else {
                ""
            };
            EventRow {
                time: dt.format("%H:%M:%S").to_string(),
                kind: e.event_type.clone(),
                level: level.to_string(),
                msg: e.payload_preview.clone(),
                session: e.session_id.clone(),
            }
        })
        .collect();

    let has_events = !events.is_empty();
    let sessions_count = rows.len();
    OverviewData {
        version: env!("CARGO_PKG_VERSION"),
        connected,
        total: db_sessions.len() as u32,
        pairing_count: pairing,
        disconnected_count: offline,
        webhook_count,
        circuits_open,
        event_rate,
        has_sessions: !rows.is_empty(),
        sessions: rows,
        sessions_count,
        events,
        has_events,
    }
}

pub async fn overview(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }
    let data = build_overview_data(&state).await;
    html(render_page("Overview", "overview", &data))
}

pub async fn overview_fragment(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if cookie_token(&headers)
        .as_deref()
        .map(token_is_superadmin)
        .unwrap_or(false)
    {
        let data = build_overview_data(&state).await;
        html(render_partial("overview_body", &data))
    } else {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::UNAUTHORIZED;
        r
    }
}

#[derive(Serialize)]
struct LoginData<'a> {
    error: Option<&'a str>,
}

pub async fn login_page(headers: HeaderMap) -> Response {
    if cookie_token(&headers)
        .as_deref()
        .map(token_is_superadmin)
        .unwrap_or(false)
    {
        return redirect_to("/");
    }
    html(render_page("Sign in", "login", &LoginData { error: None }))
}

#[derive(Deserialize)]
pub struct LoginForm {
    token: String,
}

pub async fn login_submit(Form(form): Form<LoginForm>) -> Response {
    if !token_is_superadmin(form.token.trim()) {
        return html(render_page(
            "Sign in",
            "login",
            &LoginData {
                error: Some("Token rejected. Check SUPERADMIN_TOKEN on the server."),
            },
        ));
    }

    let cookie = format!(
        "{CONSOLE_COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000",
        form.token.trim()
    );
    let mut r = redirect_to("/");
    r.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    r
}

pub async fn logout() -> Response {
    let cookie = format!("{CONSOLE_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    let mut r = redirect_to("/login");
    r.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    r
}

#[derive(Serialize)]
struct DrawerData {
    session_id: String,
    status_class: String,
    status_label: String,
    phone: Option<String>,
    jid: Option<String>,
    storage_path: String,
    qr_svg: Option<String>,
    is_connected: bool,
}

pub async fn drawer(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(sid): Path<String>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    let info = match state.session_manager().get_session(&sid).await {
        Ok(Some(info)) => info,
        _ => {
            let mut r = Response::new(Body::from("<div class='empty'>Session not found.</div>"));
            *r.status_mut() = StatusCode::NOT_FOUND;
            return r;
        }
    };

    let runtime = state.get_session(&sid);
    let status = runtime
        .as_ref()
        .map(|r| r.effective_status())
        .unwrap_or(info.status);
    let (cls, label) = status_row(status);
    let is_connected = matches!(status, SessionStatus::Connected | SessionStatus::LoggedIn);

    let qr_svg = if !is_connected {
        runtime
            .as_ref()
            .and_then(|r| r.get_qr_codes().first().cloned())
            .and_then(|code| render_qr(&code).ok())
    } else {
        None
    };

    let storage_path = state
        .session_manager()
        .get_storage_path(&sid)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let data = DrawerData {
        session_id: sid.clone(),
        status_class: cls.to_string(),
        status_label: label.to_string(),
        phone: info.phone_number.clone(),
        jid: None,
        storage_path,
        qr_svg,
        is_connected,
    };
    html(render_partial("drawer", &data))
}

#[derive(Serialize)]
struct SessionPageData {
    version: &'static str,
    session_id: String,
    sid_json: String,
    status_class: String,
    status_label: String,
    phone: Option<String>,
    storage_path: String,
}

pub async fn session_page(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(sid): Path<String>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    let info = match state.session_manager().get_session(&sid).await {
        Ok(Some(info)) => info,
        _ => return redirect_to("/"),
    };

    let runtime_status = state
        .get_session(&sid)
        .map(|r| r.effective_status())
        .unwrap_or(info.status);
    let (cls, label) = status_row(runtime_status);

    let storage_path = state
        .session_manager()
        .get_storage_path(&sid)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let data = SessionPageData {
        version: env!("CARGO_PKG_VERSION"),
        session_id: sid.clone(),
        sid_json: serde_json::to_string(&sid).unwrap_or_else(|_| "\"\"".to_string()),
        status_class: cls.to_string(),
        status_label: label.to_string(),
        phone: info.phone_number.clone(),
        storage_path,
    };
    html(render_page(&format!("Session · {}", sid), "session", &data))
}

fn render_qr(code: &str) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let qr = QrCode::new(code.as_bytes()).map_err(|e| e.to_string())?;
    let svg = qr
        .render()
        .min_dimensions(220, 220)
        .max_dimensions(320, 320)
        .dark_color(svg::Color("#0F1712"))
        .light_color(svg::Color("#FFFFFF"))
        .build();
    Ok(svg)
}

#[derive(Deserialize)]
pub struct CreateReq {
    id: Option<String>,
    name: Option<String>,
}

pub async fn create_session_proxy(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(req): Json<CreateReq>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    use crate::models::sessions::CreateSessionRequest;
    let create_req = CreateSessionRequest {
        id: req.id,
        name: req.name,
        webhook: None,
        device: None,
    };

    match crate::handlers::sessions::create_session(State(state), Json(create_req)).await {
        Ok(Json(resp)) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

pub async fn session_action_proxy(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((sid, op)): Path<(String, String)>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    match op.as_str() {
        "connect" => {
            match crate::handlers::sessions::connect_session(State(state), Path(sid), None).await {
                Ok(Json(_)) => StatusCode::OK.into_response(),
                Err(e) => e.into_response(),
            }
        }
        "disconnect" => {
            match crate::handlers::sessions::disconnect_session(State(state), Path(sid)).await {
                Ok(Json(_)) => StatusCode::OK.into_response(),
                Err(e) => e.into_response(),
            }
        }
        "logout" | "delete" => {
            match crate::handlers::sessions::delete_session(State(state), Path(sid)).await {
                Ok(Json(_)) => StatusCode::OK.into_response(),
                Err(e) => e.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn css() -> Response {
    let mut r = Response::new(Body::from(crate::console::CSS));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    r
}

pub async fn playground_js() -> Response {
    let mut r = Response::new(Body::from(crate::console::PLAYGROUND_JS));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    r
}

pub async fn logo() -> Response {
    let mut r = Response::new(Body::from(crate::console::LOGO_PNG));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    r
}
