//! Routes for tokens.

use super::handlers;
use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn api_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/tokens",
            post(handlers::mint_token).get(handlers::list_tokens),
        )
        .route("/tokens/{id}/revoke", post(handlers::revoke_token))
}
