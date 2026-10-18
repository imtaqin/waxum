//! Routes for communities.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/communities",
            get(handlers::list_communities).post(handlers::create_community),
        )
        .route(
            "/{session_id}/communities/{jid}/subgroups",
            get(handlers::list_subgroups),
        )
        .route(
            "/{session_id}/communities/{jid}/subgroups/{subgroup}/join",
            post(handlers::join_subgroup),
        )
}
