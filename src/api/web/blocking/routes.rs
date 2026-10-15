//! Routes for blocking.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/blocking/list", get(handlers::get_blocklist))
        .route(
            "/{session_id}/blocking/block",
            post(handlers::block_contact),
        )
        .route(
            "/{session_id}/blocking/unblock",
            post(handlers::unblock_contact),
        )
        .route(
            "/{session_id}/blocking/check/{jid}",
            get(handlers::is_blocked),
        )
}
