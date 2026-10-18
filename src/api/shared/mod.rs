//! Endpoints that do not belong to one channel: they work on a session
//! whether it is a linked WhatsApp Web device or a Cloud API connection, or
//! they manage the gateway itself. One folder per feature, each with its
//! `handlers.rs`, `models.rs` (when it has request/response types) and
//! `routes.rs`.
//!
//! - [`sessions`] — CRUD, pair/connect/logout, QR polling, status, diagnostics.
//! - [`messages`] — every send variant; dispatches on the session's channel.
//! - [`webhooks`] — per-session webhook targets and their health.
//! - [`blast`], [`schedule`] — bulk-send jobs and scheduled sends.
//! - [`search`] — message history ingestion and full-text search.
//! - [`store`] — the chat store row for row, and its live changes.
//! - [`new_chats`] — new-chat counter and limit.
//! - [`bulk`], [`tags`], [`events`], [`tokens`], [`nats`], [`info`] — fleet
//!   operations, session tags, the event tail, API tokens, NATS inspection,
//!   build info.

use axum::Router;

use crate::state::AppState;

pub mod blast;
pub mod bulk;
pub mod events;
pub mod info;
pub mod messages;
pub mod nats;
pub mod new_chats;
pub mod schedule;
pub mod search;
pub mod sessions;
pub mod store;
pub mod tags;
pub mod tokens;
pub mod webhooks;

/// Routes mounted directly under `/api/v1`.
pub fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(blast::routes::api_routes())
        .merge(bulk::routes::api_routes())
        .merge(events::routes::api_routes())
        .merge(schedule::routes::api_routes())
        .merge(search::routes::api_routes())
        .merge(tags::routes::api_routes())
        .merge(tokens::routes::api_routes())
}

/// Routes mounted under `/api/v1/nats`.
pub fn nats_routes() -> Router<AppState> {
    Router::new().merge(nats::routes::nats_routes())
}

/// Routes mounted under `/api/v1/sessions`.
pub fn session_routes() -> Router<AppState> {
    Router::new()
        .merge(blast::routes::session_routes())
        .merge(bulk::routes::session_routes())
        .merge(messages::routes::session_routes())
        .merge(store::routes::session_routes())
        .merge(new_chats::routes::session_routes())
        .merge(schedule::routes::session_routes())
        .merge(search::routes::session_routes())
        .merge(sessions::routes::session_routes())
        .merge(tags::routes::session_routes())
        .merge(webhooks::routes::session_routes())
}
