//! Waxum Console — a server-rendered ops dashboard baked into the
//! binary. No separate frontend build step: templates are Handlebars
//! files pulled in with `include_str!`, styles are one CSS file, and
//! auto-refresh is a plain `setInterval` fetch — no HTMX or SPA runtime.
//!
//! Mount points (all under the root `/`, not `/console`):
//!
//! - `GET  /`                       — overview (or redirect to /login when unauth'd)
//! - `GET  /login`, `POST /login`   — token → cookie
//! - `POST /logout`                 — clear cookie, redirect to /login
//! - `GET  /data`                   — fragment used by the poll loop
//! - `GET  /drawer/{sid}`           — session drawer partial
//! - `POST /sessions`               — create session (proxies handlers::sessions)
//! - `POST /sessions/{sid}/{op}`    — connect/disconnect/logout/delete
//! - `GET  /assets/console.css`    — one CSS file
//!
//! Auth: a single `waxum_console` cookie carrying the superadmin token.
//! It is validated on every request against `SUPERADMIN_TOKEN` (or a
//! decoded JWT with `role=superadmin`). The console never issues its own
//! tokens — the intent is that whoever holds `SUPERADMIN_TOKEN` can drive
//! the UI, same as the REST API.

use axum::Router;
use handlebars::Handlebars;
use once_cell::sync::Lazy;
use serde::Serialize;

use crate::state::AppState;

pub mod handlers;

pub const CONSOLE_COOKIE: &str = "waxum_console";

pub const CSS: &str = include_str!("assets/console.css");
pub const PLAYGROUND_JS: &str = include_str!("assets/playground.js");
pub const LOGO_PNG: &[u8] = include_bytes!("assets/logo.png");

const TPL_LAYOUT: &str = include_str!("templates/layout.hbs");
const TPL_OVERVIEW: &str = include_str!("templates/overview.hbs");
const TPL_OVERVIEW_BODY: &str = include_str!("templates/overview_body.hbs");
const TPL_LOGIN: &str = include_str!("templates/login.hbs");
const TPL_DRAWER: &str = include_str!("templates/drawer.hbs");
const TPL_SESSION: &str = include_str!("templates/session.hbs");

/// Global Handlebars registry. Compiled once at process start. Templates
/// are baked into the binary via `include_str!`, so a corrupt template
/// disk file is not a class of failure we need to handle at runtime.
pub static HBS: Lazy<Handlebars<'static>> = Lazy::new(|| {
    let mut h = Handlebars::new();
    h.set_strict_mode(false);
    h.register_template_string("layout", TPL_LAYOUT).unwrap();
    h.register_template_string("overview", TPL_OVERVIEW)
        .unwrap();
    h.register_template_string("overview_body", TPL_OVERVIEW_BODY)
        .unwrap();
    h.register_template_string("login", TPL_LOGIN).unwrap();
    h.register_template_string("drawer", TPL_DRAWER).unwrap();
    h.register_template_string("session", TPL_SESSION).unwrap();
    h
});

/// Render the layout with a partial (`body`) filled in. Passes the same
/// `data` object through to the body partial so it can read fields
/// directly.
pub fn render_page<T: Serialize>(title: &str, body_partial: &str, data: &T) -> String {
    let value = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
    let mut root = serde_json::Map::new();
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            root.insert(k, v);
        }
    }
    root.insert("title".into(), serde_json::Value::String(title.to_string()));

    let body_tpl = match body_partial {
        "overview" => TPL_OVERVIEW,
        "login" => TPL_LOGIN,
        "session" => TPL_SESSION,
        _ => "",
    };

    let mut local = HBS.clone();
    local
        .register_template_string("body", body_tpl)
        .unwrap_or(());
    local
        .render("layout", &serde_json::Value::Object(root))
        .unwrap_or_else(|e| format!("<pre>template error: {e}</pre>"))
}

/// Render a bare partial (no layout wrap) — used for fragment responses.
pub fn render_partial<T: Serialize>(template: &str, data: &T) -> String {
    HBS.render(template, data)
        .unwrap_or_else(|e| format!("<pre>template error: {e}</pre>"))
}

pub fn console_router() -> Router<AppState> {
    use axum::routing::{get, post};

    Router::new()
        .route("/", get(handlers::overview))
        .route("/data", get(handlers::overview_fragment))
        .route(
            "/login",
            get(handlers::login_page).post(handlers::login_submit),
        )
        .route("/logout", post(handlers::logout))
        .route("/drawer/{sid}", get(handlers::drawer))
        .route("/s/{sid}", get(handlers::session_page))
        .route("/qr-svg/{sid}", get(handlers::qr_svg))
        .route("/sessions", post(handlers::create_session_proxy))
        .route("/sessions/{sid}/{op}", post(handlers::session_action_proxy))
        .route("/assets/console.css", get(handlers::css))
        .route("/assets/playground.js", get(handlers::playground_js))
        .route("/assets/logo.png", get(handlers::logo))
}

/// Paths the JWT bearer middleware must let through unauthenticated so
/// the console can serve its own login flow (which uses a cookie instead
/// of `Authorization: Bearer`).
pub fn is_console_path(path: &str) -> bool {
    matches!(
        path,
        "/" | "/login"
            | "/logout"
            | "/data"
            | "/assets/console.css"
            | "/assets/playground.js"
            | "/assets/logo.png"
    ) || path.starts_with("/drawer/")
        || path.starts_with("/qr-svg/")
        || path.starts_with("/s/")
        || (path.starts_with("/sessions") && !path.starts_with("/api/"))
}
