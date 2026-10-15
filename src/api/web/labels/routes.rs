//! Routes for labels.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, post, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/labels", post(handlers::create_label))
        .route(
            "/{session_id}/labels/{label_id}",
            delete(handlers::delete_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/chats/{chat_jid}",
            post(handlers::add_chat_label).delete(handlers::remove_chat_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/messages",
            post(handlers::add_message_label),
        )
        .route(
            "/{session_id}/labels/{label_id}/messages/remove",
            post(handlers::remove_message_label),
        )
        .route(
            "/{session_id}/quick-replies",
            put(handlers::set_quick_reply),
        )
        .route(
            "/{session_id}/quick-replies/{id}",
            delete(handlers::delete_quick_reply),
        )
        .route(
            "/{session_id}/settings/link-previews",
            post(handlers::set_link_previews),
        )
}
