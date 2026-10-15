//! Routes for privacy.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new().route(
        "/{session_id}/privacy/settings",
        get(handlers::get_privacy_settings),
    )
}
