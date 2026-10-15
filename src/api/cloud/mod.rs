//! Endpoints for sessions that run on the official WhatsApp Cloud API.
//! The Graph API client itself lives in [`crate::cloud`]; this tree is only
//! the HTTP surface waxum exposes on top of it.
//!
//! - [`connection`] — connect a session to a Cloud API app, embedded
//!   signup, and the Meta webhook (verify and receive).
//! - [`admin`] — business profile, phone numbers, templates, subscriptions,
//!   quality and the other WABA administration calls.
//! - [`commerce`] — commerce settings and catalogue/product messages.
//! - [`flows`] — WhatsApp Flows: manage, publish and answer them.

use axum::Router;

use crate::state::AppState;

pub mod admin;
pub mod commerce;
pub mod connection;
pub mod flows;

/// Routes mounted under `/api/v1/sessions`.
pub fn session_routes() -> Router<AppState> {
    Router::new()
        .merge(admin::routes::session_routes())
        .merge(commerce::routes::session_routes())
        .merge(connection::routes::session_routes())
        .merge(flows::routes::session_routes())
}
