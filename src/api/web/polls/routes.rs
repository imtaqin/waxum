//! Routes for polls.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/polls/{chat}/{message_id}",
            get(handlers::poll_results),
        )
        .route(
            "/{session_id}/polls/{chat}/{message_id}/vote",
            post(handlers::vote),
        )
}
