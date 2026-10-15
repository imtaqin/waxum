//! Routes for events.

use super::handlers;
use crate::state::AppState;
use axum::routing::get;
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new().route("/events/tail", get(handlers::events_tail))
}
