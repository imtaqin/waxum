//! HTTP handlers for the browser-facing console. All auth is via a
//! single `waxum_console` cookie carrying the superadmin token — no
//! CSRF token flow because every write endpoint is a POST from
//! same-origin JS driven by the operator who already holds the cookie,
//! and there is no third-party embedding surface. SameSite=Strict on
//! the cookie makes cross-site POSTs unusable. `Secure` is added
//! whenever the request arrived over TLS (detected via
//! `X-Forwarded-Proto`, see `console_cookie_attrs`) and omitted
//! otherwise, so a plain `cargo run` over `http://localhost` still
//! works with no extra config.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Form, Json,
};
use serde::{Deserialize, Serialize};

use crate::console::{render_page, render_partial, CONSOLE_COOKIE};
use crate::models::sessions::{SessionInfo, SessionStatus};
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

/// A minted token (carries a `jti`, see `crate::db::tokens`) is a
/// session-scoped API credential -- it must not also grant the fleet-wide
/// console, so this rejects any JWT carrying one even if its role claim
/// says `superadmin`.
fn token_is_superadmin(token: &str) -> bool {
    if let Ok(superadmin) = std::env::var("SUPERADMIN_TOKEN") {
        if !superadmin.is_empty() && crate::middleware::jwt::constant_time_eq(token, &superadmin) {
            return true;
        }
    }
    let jwt_auth = crate::middleware::jwt::JwtAuth::new();
    match jwt_auth.validate_token(token) {
        Ok(claims) => {
            crate::middleware::jwt::JwtAuth::is_superadmin(&claims) && claims.jti.is_none()
        }
        Err(_) => false,
    }
}

/// Whether to mark the console cookie `Secure`. Waxum itself only ever
/// speaks plain HTTP (see `main.rs` -- TLS, if any, is terminated by a
/// reverse proxy in front of it), so there's no local signal for "this
/// connection is actually TLS". `X-Forwarded-Proto` is the de facto
/// standard a proxy sets to say so; trusting a client that spoofs it can
/// only make the cookie *stricter* (sent without `Secure` over their own
/// plaintext connection, so it just doesn't get echoed back to them) --
/// never weaker. A direct `cargo run` with no proxy in front sees no such
/// header and keeps working over `http://localhost` exactly as before.
fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

fn console_cookie_attrs(headers: &HeaderMap) -> &'static str {
    if request_is_https(headers) {
        "Path=/; HttpOnly; Secure; SameSite=Strict"
    } else {
        "Path=/; HttpOnly; SameSite=Strict"
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
    /// Display name, when it says more than the id.
    name: Option<String>,
    phone: Option<String>,
    status_class: String,
    status_label: String,
    is_cloud: bool,
    webhooks: usize,
    webhooks_suspended: usize,
    can_connect: bool,
    can_disconnect: bool,
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

/// CSS class and label for a session's state.
///
/// A `whatsapp_cloud` session has no socket: once its credentials are
/// stored it is reachable, so it is always "Ready". For a linked-device
/// session the pairing states get their own label: a session showing a
/// QR used to read "OFFLINE", which sent people looking for a fault.
fn session_status(state: &AppState, s: &SessionInfo) -> (&'static str, &'static str) {
    if crate::handlers::sessions::is_cloud_session(s) {
        return ("connected", "Ready");
    }
    status_row(runtime_status(state, s))
}

fn runtime_status(state: &AppState, s: &SessionInfo) -> SessionStatus {
    state
        .get_session(&s.id)
        .map(|r| r.effective_status())
        .unwrap_or(s.status)
}

fn status_row(s: SessionStatus) -> (&'static str, &'static str) {
    match s {
        SessionStatus::LoggedIn => ("connected", "Connected"),
        SessionStatus::Connected => ("connecting", "Logging in"),
        SessionStatus::Connecting => ("connecting", "Connecting"),
        SessionStatus::WaitingForQr => ("pairing", "Waiting for QR scan"),
        SessionStatus::WaitingForPairCode => ("pairing", "Waiting for pair code"),
        SessionStatus::Disconnected => ("idle", "Disconnected"),
    }
}

/// One line describing an event for the activity feed. The raw payload is
/// JSON meant for webhooks; dumping it into the page was unreadable and,
/// for `qr_code`, put the pairing secret on screen.
pub fn summarize_event(event: &str, payload: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) else {
        return String::new();
    };
    let data = parsed.get("data").unwrap_or(&parsed);
    let text = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let line = match event {
        "message" => {
            let who = [text("push_name"), text("from_phone"), text("chat")]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("");
            let body = [text("text"), text("caption")]
                .into_iter()
                .find(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("[{}]", text("message_type")));
            let direction = if data.get("is_from_me").and_then(|v| v.as_bool()) == Some(true) {
                "to"
            } else {
                "from"
            };
            format!("{direction} {who}: {body}")
        }
        "qr_code" => "new QR ready to scan".to_string(),
        "pair_code" => "pair code issued".to_string(),
        "webhook_suspended" => format!(
            "{} — retry in {} min ({})",
            text("url"),
            data.get("retry_in_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                / 60,
            text("last_error")
        ),
        "webhook_resumed" => text("url").to_string(),
        "pairing_code_error" | "pair_error" => text("error").to_string(),
        "disconnected" | "logged_out" | "connect_failure" | "account_locked" => {
            text("reason").to_string()
        }
        "receipt" => text("type").to_string(),
        _ => String::new(),
    };
    line.chars().take(140).collect()
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
        let (cls, label) = session_status(state, s);
        match cls {
            "connected" => connected += 1,
            "connecting" | "pairing" => pairing += 1,
            _ => offline += 1,
        }
        let is_cloud = crate::handlers::sessions::is_cloud_session(s);
        let hooks = state.get_webhooks(&s.id);
        let webhooks_suspended = hooks
            .iter()
            .filter(|(_, w)| w.enabled && state.webhook_circuit_snapshot(&w.url).open)
            .count();
        let status = runtime_status(state, s);
        rows.push(SessionRow {
            id: s.id.clone(),
            name: s.name.clone().filter(|n| !n.is_empty() && *n != s.id),
            phone: s.phone_number.clone(),
            status_class: cls.to_string(),
            status_label: label.to_string(),
            is_cloud,
            webhooks: hooks.len(),
            webhooks_suspended,
            can_connect: !is_cloud && status == SessionStatus::Disconnected,
            can_disconnect: !is_cloud && status != SessionStatus::Disconnected,
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
            let level = if e.event_type.contains("error")
                || e.event_type.contains("fail")
                || e.event_type.contains("suspended")
                || e.event_type.contains("locked")
            {
                "err"
            } else if e.event_type.contains("disconnect") || e.event_type.contains("logged_out") {
                "warn"
            } else {
                ""
            };
            EventRow {
                time: dt.format("%H:%M:%S").to_string(),
                kind: e.event_type.clone(),
                level: level.to_string(),
                msg: e.summary.clone(),
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

pub async fn login_submit(headers: HeaderMap, Form(form): Form<LoginForm>) -> Response {
    if !token_is_superadmin(form.token.trim()) {
        tracing::warn!("console: rejected login attempt with an invalid token");
        return html(render_page(
            "Sign in",
            "login",
            &LoginData {
                error: Some("Token rejected. Check SUPERADMIN_TOKEN on the server."),
            },
        ));
    }

    let cookie = format!(
        "{CONSOLE_COOKIE}={}; {}; Max-Age=2592000",
        form.token.trim(),
        console_cookie_attrs(&headers)
    );
    let mut r = redirect_to("/");
    r.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    r
}

pub async fn logout(headers: HeaderMap) -> Response {
    let cookie = format!(
        "{CONSOLE_COOKIE}=; {}; Max-Age=0",
        console_cookie_attrs(&headers)
    );
    let mut r = redirect_to("/login");
    r.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    r
}

#[derive(Serialize)]
struct SessionPageData {
    version: &'static str,
    session_id: String,
    sid_json: String,
    provider_json: String,
    status_class: String,
    status_label: String,
    phone: Option<String>,
    storage_path: String,
    cloud: Option<CloudPanel>,
}

/// Non-secret Cloud API identifiers shown on a `whatsapp_cloud` session's
/// page. The webhook and Flow endpoint URLs the operator pastes into Meta's
/// app dashboard are built client-side from `location.origin`, since the
/// console can't know the public host waxum is reachable at behind a proxy.
#[derive(Serialize)]
struct CloudPanel {
    waba_id: Option<String>,
    phone_number_id: Option<String>,
    business_id: Option<String>,
    app_id: Option<String>,
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

    let (cls, label) = session_status(&state, &info);
    let is_cloud = crate::handlers::sessions::is_cloud_session(&info);

    let storage_path = state
        .session_manager()
        .get_storage_path(&sid)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let cloud = is_cloud.then(|| CloudPanel {
        waba_id: info.cloud_waba_id.clone(),
        phone_number_id: info.cloud_phone_number_id.clone(),
        business_id: info.cloud_business_id.clone(),
        app_id: info.cloud_app_id.clone(),
    });

    let data = SessionPageData {
        version: env!("CARGO_PKG_VERSION"),
        session_id: sid.clone(),
        sid_json: serde_json::to_string(&sid).unwrap_or_else(|_| "\"\"".to_string()),
        provider_json: serde_json::to_string(if is_cloud { "cloud" } else { "web" })
            .unwrap_or_else(|_| "\"web\"".to_string()),
        status_class: cls.to_string(),
        status_label: label.to_string(),
        phone: info.phone_number.clone(),
        storage_path,
        cloud,
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
        reuse: None,
        webhook: None,
        device: None,
    };

    match crate::handlers::sessions::create_session(State(state), None, Json(create_req)).await {
        Ok(Json(resp)) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Body for `POST /sessions/cloud`: creates a session and attaches Cloud
/// API credentials to it in one step.
#[derive(Deserialize)]
pub struct CreateCloudReq {
    id: String,
    #[serde(flatten)]
    credentials: crate::models::cloud::ConnectCloudRequest,
}

pub async fn create_cloud_session_proxy(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(req): Json<CreateCloudReq>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    use crate::models::sessions::CreateSessionRequest;
    let create_req = CreateSessionRequest {
        id: Some(req.id.clone()),
        name: Some(req.id.clone()),
        reuse: None,
        webhook: None,
        device: None,
    };
    if let Err(e) =
        crate::handlers::sessions::create_session(State(state.clone()), None, Json(create_req))
            .await
    {
        return e.into_response();
    }

    match crate::handlers::cloud::connect_cloud(State(state), Path(req.id), Json(req.credentials))
        .await
    {
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
        "logout" => {
            match crate::handlers::sessions::logout_session(State(state), Path(sid)).await {
                Ok(Json(_)) => StatusCode::OK.into_response(),
                Err(e) => e.into_response(),
            }
        }
        "delete" => {
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

pub async fn qr_svg(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(sid): Path<String>,
) -> Response {
    if let Err(r) = require_auth(&headers) {
        return *r;
    }

    let code = state
        .get_session(&sid)
        .and_then(|r| r.get_qr_codes().first().cloned());

    let body: String = match code {
        Some(c) => render_qr(&c).unwrap_or_else(|_| String::new()),
        None => String::new(),
    };

    if body.is_empty() {
        let mut r = Response::new(Body::from(""));
        *r.status_mut() = StatusCode::NO_CONTENT;
        return r;
    }

    let mut r = Response::new(Body::from(body));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
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

#[cfg(test)]
mod tests {
    use super::summarize_event;

    #[test]
    fn a_qr_event_is_summarised_without_the_pairing_code() {
        let payload = r#"{"session_id":"s","event":"qr_code","data":{"code":"2@SECRETSECRET,abc==","timeout_seconds":59}}"#;
        let line = summarize_event("qr_code", payload);
        assert_eq!(line, "new QR ready to scan");
        assert!(!line.contains("SECRET"));
    }

    #[test]
    fn messages_read_as_who_and_what() {
        let inbound =
            r#"{"event":"message","data":{"push_name":"Budi","text":"halo","is_from_me":false}}"#;
        assert_eq!(summarize_event("message", inbound), "from Budi: halo");
        let media = r#"{"event":"message","data":{"from_phone":"628111","message_type":"image","is_from_me":false}}"#;
        assert_eq!(summarize_event("message", media), "from 628111: [image]");
        let outbound = r#"{"event":"message","data":{"chat":"628222@s.whatsapp.net","caption":"cek","is_from_me":true}}"#;
        assert_eq!(
            summarize_event("message", outbound),
            "to 628222@s.whatsapp.net: cek"
        );
    }

    #[test]
    fn unknown_events_and_bad_payloads_give_an_empty_line() {
        assert_eq!(summarize_event("presence", "{}"), "");
        assert_eq!(summarize_event("message", "not json"), "");
    }
}
