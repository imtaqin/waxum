//! Routes for schedule.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get};
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new().route("/scheduled", get(handlers::list_all_scheduled))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/scheduled",
            get(handlers::list_session_scheduled),
        )
        .route(
            "/{session_id}/scheduled/{id}",
            delete(handlers::cancel_scheduled),
        )
}
