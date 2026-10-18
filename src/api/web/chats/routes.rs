//! Routes for chat actions.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get, post, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/chats", get(handlers::list_chats))
        .route("/{session_id}/chats/{jid}", delete(handlers::delete_chat))
        .route(
            "/{session_id}/chats/{jid}/archive",
            post(handlers::archive_chat),
        )
        .route("/{session_id}/chats/{jid}/pin", post(handlers::pin_chat))
        .route("/{session_id}/chats/{jid}/mute", post(handlers::mute_chat))
        .route(
            "/{session_id}/chats/{jid}/read",
            post(handlers::mark_chat_read),
        )
        .route(
            "/{session_id}/chats/{jid}/clear",
            post(handlers::clear_chat),
        )
        .route(
            "/{session_id}/chats/{jid}/disappearing",
            put(handlers::set_disappearing),
        )
        .route("/{session_id}/messages/star", post(handlers::star_message))
        .route(
            "/{session_id}/messages/starred",
            get(handlers::list_starred),
        )
        .route(
            "/{session_id}/messages/delete-for-me",
            post(handlers::delete_for_me),
        )
}
