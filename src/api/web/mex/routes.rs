//! Routes for mex.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/mex/query", post(handlers::mex_query))
        .route("/{session_id}/mex/mutate", post(handlers::mex_mutate))
}
