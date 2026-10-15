//! Routes for new chats.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new().route(
        "/{session_id}/settings/new-chat-limit",
        get(handlers::get_new_chat_limit)
            .put(handlers::set_new_chat_limit)
            .delete(handlers::clear_new_chat_limit),
    )
}
