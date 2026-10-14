//! Routes for blast.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new().route("/blasts", get(handlers::list_all_blasts))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/blast", post(handlers::create_blast))
        .route("/{session_id}/blasts", get(handlers::list_session_blasts))
        .route("/{session_id}/blasts/{id}", get(handlers::get_blast))
        .route(
            "/{session_id}/blasts/{id}/recipients",
            get(handlers::list_blast_recipients),
        )
        .route(
            "/{session_id}/blasts/{id}/cancel",
            post(handlers::cancel_blast),
        )
        .route(
            "/{session_id}/blasts/{id}/retry",
            post(handlers::retry_blast),
        )
}
