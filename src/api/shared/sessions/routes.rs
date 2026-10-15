//! Routes for sessions.

use super::handlers;
use crate::api::MAX_SESSION_IMPORT_BYTES;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(handlers::create_session))
        .route("/", get(handlers::list_sessions))
        .route("/{session_id}", get(handlers::get_session))
        .route("/{session_id}", delete(handlers::delete_session))
        .route("/{session_id}/status", get(handlers::get_session_status))
        .route("/{session_id}/qr", get(handlers::get_qr_code))
        .route("/{session_id}/connect", post(handlers::connect_session))
        .route(
            "/{session_id}/connect/wait",
            get(handlers::connect_and_wait),
        )
        .route("/{session_id}/pair", post(handlers::pair_session))
        .route(
            "/{session_id}/disconnect",
            post(handlers::disconnect_session),
        )
        .route("/{session_id}/logout", post(handlers::logout_session))
        .route("/{session_id}/export", post(handlers::export_session))
        .route(
            "/{session_id}/import",
            post(handlers::import_session).layer(DefaultBodyLimit::max(MAX_SESSION_IMPORT_BYTES)),
        )
        .route("/{session_id}/device", get(handlers::get_device_info))
}
