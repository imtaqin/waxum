//! Routes for the chat store.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/store/chats", get(handlers::chats))
        .route("/{session_id}/store/chats/{jid}", get(handlers::chat))
        .route("/{session_id}/store/chats/{jid}/messages", get(handlers::messages))
        .route("/{session_id}/store/chats/{jid}/messages/{id}", get(handlers::message))
        .route(
            "/{session_id}/store/chats/{jid}/messages/{id}/reactions",
            get(handlers::reactions),
        )
        .route(
            "/{session_id}/store/chats/{jid}/messages/{id}/receipts",
            get(handlers::receipts),
        )
        .route("/{session_id}/store/chats/{jid}/kinds", get(handlers::kinds))
        .route("/{session_id}/store/chats/{jid}/links", get(handlers::links))
        .route("/{session_id}/store/recent", get(handlers::recent))
        .route("/{session_id}/store/starred", get(handlers::starred))
        .route("/{session_id}/store/search", get(handlers::search))
        .route("/{session_id}/store/contacts", get(handlers::contacts))
        .route("/{session_id}/store/contacts/{jid}", get(handlers::contact))
        .route("/{session_id}/store/events", get(handlers::events))
}
