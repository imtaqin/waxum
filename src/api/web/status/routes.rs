//! Routes for status.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new().route(
        "/{session_id}/status/react",
        post(handlers::send_status_reaction),
    )
}
