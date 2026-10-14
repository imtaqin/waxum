//! Routes for profile.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/profile", put(handlers::update_profile))
        .route(
            "/{session_id}/profile/picture",
            delete(handlers::remove_profile_picture),
        )
}
