//! Routes for operations.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/spam/report", post(handlers::spam_report))
        .route("/{session_id}/tctoken/issue", post(handlers::tctoken_issue))
        .route("/{session_id}/tctoken/list", get(handlers::tctoken_list))
        .route(
            "/{session_id}/tctoken/expired",
            delete(handlers::tctoken_prune),
        )
        .route("/{session_id}/tctoken/{jid}", get(handlers::tctoken_get))
        .route(
            "/{session_id}/reconnect",
            get(handlers::get_auto_reconnect).put(handlers::set_auto_reconnect),
        )
        .route(
            "/{session_id}/history-sync",
            get(handlers::get_history_sync).put(handlers::set_history_sync),
        )
        .route("/{session_id}/pause", post(handlers::pause_session))
        .route("/{session_id}/resume", post(handlers::resume_session))
        .route(
            "/{session_id}/appstate/resync",
            post(handlers::appstate_resync),
        )
}
