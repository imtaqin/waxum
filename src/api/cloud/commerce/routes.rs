//! Routes for commerce.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/cloud/commerce-settings",
            get(handlers::get_commerce_settings).post(handlers::update_commerce_settings),
        )
        .route(
            "/{session_id}/messages/product",
            post(handlers::send_product),
        )
        .route(
            "/{session_id}/messages/product-list",
            post(handlers::send_product_list),
        )
        .route(
            "/{session_id}/messages/catalog",
            post(handlers::send_catalog),
        )
}
