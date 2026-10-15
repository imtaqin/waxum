//! Routes for nats.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn nats_routes() -> Router<AppState> {
    Router::new()
        .route("/status", get(handlers::nats_status))
        .route(
            "/streams/{stream_name}/purge",
            post(handlers::nats_purge_stream),
        )
        .route(
            "/streams/{stream_name}/consumers",
            get(handlers::nats_list_consumers),
        )
}
