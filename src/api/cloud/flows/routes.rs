//! Routes for flows.

use super::handlers;
use crate::state::AppState;
use axum::routing::{get, post, put};
use axum::Router;

pub fn session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{session_id}/cloud/flows",
            get(handlers::list_flows).post(handlers::create_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}",
            get(handlers::get_flow)
                .post(handlers::update_flow_metadata)
                .delete(handlers::delete_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/json",
            put(handlers::update_flow_json),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/assets",
            get(handlers::get_flow_assets),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/publish",
            post(handlers::publish_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/deprecate",
            post(handlers::deprecate_flow),
        )
        .route(
            "/{session_id}/cloud/flows/{flow_id}/send",
            post(handlers::send_flow),
        )
        .route(
            "/{session_id}/cloud/flow-endpoint",
            post(handlers::configure_flow_endpoint),
        )
        .route(
            "/{session_id}/cloud/flow-endpoint/exchange",
            post(handlers::flow_data_exchange),
        )
}
