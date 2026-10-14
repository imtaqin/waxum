//! Routes for bots.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/bots", get(handlers::list_bots))
        .route("/{session_id}/capping", get(handlers::get_capping))
}
