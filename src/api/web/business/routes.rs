//! Routes for business.

use super::handlers;
use crate::state::AppState;
use axum::routing::{delete, get, patch};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route("/{session_id}/business/catalog", get(handlers::get_catalog))
        .route(
            "/{session_id}/business/collections",
            get(handlers::get_collections),
        )
        .route("/{session_id}/business/order", get(handlers::get_order))
        .route(
            "/{session_id}/business/profile",
            patch(handlers::update_business_profile),
        )
        .route(
            "/{session_id}/business/cover-photo/{photo_id}",
            delete(handlers::remove_cover_photo),
        )
}
