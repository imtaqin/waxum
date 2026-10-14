//! Endpoints that talk to a linked WhatsApp Web device through
//! whatsapp-rust: groups, contacts, presence and chat state, blocking and
//! privacy, calls, status, labels, newsletters, business and bot tools,
//! the linked account's own profile, media and the bulk operations. One folder per
//! feature, each with its `handlers.rs`, `models.rs` (when it has request or
//! response types) and `routes.rs`. A Cloud API session answers these with
//! the usual not-connected error.

use axum::Router;

use crate::state::AppState;

pub mod blocking;
pub mod bots;
pub mod business;
pub mod calls;
pub mod chatstate;
pub mod contacts;
pub mod fake_reply;
pub mod groups;
pub mod labels;
pub mod media;
pub mod mex;
pub mod newsletter;
pub mod operations;
pub mod presence;
pub mod privacy;
pub mod profile;
pub mod status;

/// Routes mounted directly under `/api/v1`.
pub fn api_routes() -> Router<AppState> {
    Router::new().merge(calls::routes::api_routes())
}

/// Routes mounted under `/api/v1/sessions`.
pub fn session_routes() -> Router<AppState> {
    Router::new()
        .merge(blocking::routes::session_routes())
        .merge(bots::routes::session_routes())
        .merge(business::routes::session_routes())
        .merge(calls::routes::session_routes())
        .merge(chatstate::routes::session_routes())
        .merge(contacts::routes::session_routes())
        .merge(groups::routes::session_routes())
        .merge(labels::routes::session_routes())
        .merge(media::routes::session_routes())
        .merge(mex::routes::session_routes())
        .merge(newsletter::routes::session_routes())
        .merge(operations::routes::session_routes())
        .merge(presence::routes::session_routes())
        .merge(privacy::routes::session_routes())
        .merge(profile::routes::session_routes())
        .merge(status::routes::session_routes())
}
