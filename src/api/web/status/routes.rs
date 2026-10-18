//! Routes for status.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/status/react",
            post(handlers::send_status_reaction),
        )
        .route(
            "/{session_id}/status/text",
            post(handlers::post_text_status),
        )
        .route(
            "/{session_id}/status/image",
            post(handlers::post_image_status),
        )
        .route(
            "/{session_id}/status/{message_id}/revoke",
            post(handlers::revoke_status),
        )
}
