//! Routes for tags.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get};
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new().route("/tags", get(handlers::list_all_tags))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/tags",
            get(handlers::list_session_tags)
                .put(handlers::replace_session_tags)
                .post(handlers::add_session_tag),
        )
        .route(
            "/{session_id}/tags/{tag}",
            delete(handlers::remove_session_tag),
        )
}
