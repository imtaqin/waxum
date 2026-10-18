//! Routes for privacy.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/privacy/settings",
            get(handlers::get_privacy_settings).put(handlers::set_privacy_setting),
        )
        .route(
            "/{session_id}/privacy/disappearing",
            put(handlers::set_default_disappearing),
        )
}
