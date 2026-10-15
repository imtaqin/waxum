//! Routes for presence.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/presence/set", post(handlers::set_presence))
        .route(
            "/{session_id}/presence/subscribe",
            post(handlers::subscribe_presence),
        )
}
