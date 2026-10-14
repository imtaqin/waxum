//! Routes for search.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new().route("/messages/search", get(handlers::search_all_messages))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/messages/search",
            get(handlers::search_session_messages),
        )
        .route(
            "/{session_id}/messages/chat/{chat_jid}",
            get(handlers::list_chat_messages),
        )
        .route(
            "/{session_id}/messages",
            get(handlers::list_session_messages),
        )
}
