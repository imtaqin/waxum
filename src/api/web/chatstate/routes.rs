//! Routes for chatstate.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/chatstate/send",
            post(handlers::send_chatstate),
        )
        .route(
            "/{session_id}/chatstate/typing",
            post(handlers::send_typing),
        )
}
