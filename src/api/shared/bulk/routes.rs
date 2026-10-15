//! Routes for bulk.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/stats", get(handlers::fleet_stats))
        .route("/webhooks/reenable-all", post(handlers::reenable_circuits))
}

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/purge", post(handlers::purge_sessions))
        .route("/disconnect-all", post(handlers::disconnect_all))
        .route("/reconnect-all", post(handlers::reconnect_all))
        .route("/search", get(handlers::search_sessions))
}
